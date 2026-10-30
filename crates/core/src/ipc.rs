//! Local IPC between the Agent and its clients (desktop UI, CLI).
//!
//! Transport: a named pipe on Windows, a Unix domain socket in a 0700
//! directory on Linux. Protocol: one JSON object per line. Every request
//! carries a per-user random token read from a file only the user can read,
//! so other local users cannot drive the Agent.

use crate::error::{Error, Result};
use interprocess::local_socket::{prelude::*, GenericFilePath, GenericNamespaced, ListenerOptions, Stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const MAX_LINE: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub token: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// Where the Agent listens.
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// Pipe name (Windows) or socket path (Unix).
    pub name: String,
    pub token_file: PathBuf,
}

fn user_tag() -> String {
    let u = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_else(|_| "user".into());
    u.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(32).collect()
}

impl Endpoint {
    /// Default endpoint for this user and data directory.
    pub fn default_for(data_dir: &Path) -> Endpoint {
        let token_file = data_dir.join("agent.token");
        if cfg!(windows) {
            // Include a hash of the data dir so isolated profiles (tests,
            // portable installs) do not collide.
            let tag = &crate::util::sha256_hex(data_dir.to_string_lossy().as_bytes())[..10];
            Endpoint { name: format!("continuity-agent-{}-{tag}", user_tag()), token_file }
        } else {
            let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| data_dir.to_path_buf());
            let tag = &crate::util::sha256_hex(data_dir.to_string_lossy().as_bytes())[..10];
            Endpoint { name: dir.join(format!("continuity-{tag}.sock")).display().to_string(), token_file }
        }
    }

    fn to_name(&self) -> std::io::Result<interprocess::local_socket::Name<'_>> {
        if cfg!(windows) {
            self.name.as_str().to_ns_name::<GenericNamespaced>()
        } else {
            self.name.as_str().to_fs_name::<GenericFilePath>()
        }
    }
}

pub fn write_token_file(path: &Path) -> Result<String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
        restrict_dir(d);
    }
    let token = crate::util::random_hex(32);
    let tmp = path.with_extension("token.tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(token.as_bytes())?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(token)
}

pub fn read_token_file(path: &Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?.trim().to_string())
}

pub fn restrict_dir(_d: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(_d, std::fs::Permissions::from_mode(0o700));
    }
}

pub type Handler = Arc<dyn Fn(&str, Value) -> Result<Value> + Send + Sync>;

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub struct Server {
    endpoint: Endpoint,
    stop: Arc<AtomicBool>,
}

impl Server {
    /// Bind the endpoint. Fails if another Agent already owns it.
    pub fn bind(endpoint: Endpoint, token: String, handler: Handler) -> Result<(Server, std::thread::JoinHandle<()>)> {
        #[cfg(unix)]
        if let Some(d) = Path::new(&endpoint.name).parent() {
            std::fs::create_dir_all(d)?;
        }
        let listener = ListenerOptions::new().name(endpoint.to_name()?).try_overwrite(cfg!(unix)).create_sync()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let token = Arc::new(token);
        let handle = std::thread::Builder::new()
            .name("ipc-accept".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if stop2.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(conn) = conn else { continue };
                    let handler = handler.clone();
                    let token = token.clone();
                    let _ = std::thread::Builder::new().name("ipc-conn".into()).spawn(move || serve_conn(conn, &token, &handler));
                }
            })?;
        Ok((Server { endpoint, stop }, handle))
    }

    /// Stop accepting connections (wakes the accept loop).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Ok(name) = self.endpoint.to_name() {
            let _ = Stream::connect(name);
        }
    }
}

fn serve_conn(conn: Stream, token: &str, handler: &Handler) {
    let mut reader = BufReader::new(conn);
    loop {
        let mut line = String::new();
        match (&mut reader).take(MAX_LINE).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if !line.ends_with('\n') {
            return; // oversized or truncated
        }
        let resp = match serde_json::from_str::<Request>(&line) {
            Err(e) => Response { id: 0, ok: false, result: None, error: Some(RpcError { code: "bad_request".into(), message: e.to_string() }) },
            Ok(req) if !constant_time_eq(&req.token, token) => Response {
                id: req.id,
                ok: false,
                result: None,
                error: Some(RpcError { code: "unauthorized".into(), message: "token inválido".into() }),
            },
            Ok(req) => {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(&req.method, req.params)));
                match r {
                    Ok(Ok(v)) => Response { id: req.id, ok: true, result: Some(v), error: None },
                    Ok(Err(e)) => Response {
                        id: req.id,
                        ok: false,
                        result: None,
                        error: Some(RpcError { code: e.code().into(), message: e.to_string() }),
                    },
                    Err(_) => Response {
                        id: req.id,
                        ok: false,
                        result: None,
                        error: Some(RpcError { code: "panic".into(), message: "erro interno no Agent".into() }),
                    },
                }
            }
        };
        let mut out = match serde_json::to_vec(&resp) {
            Ok(v) => v,
            Err(_) => return,
        };
        out.push(b'\n');
        if reader.get_mut().write_all(&out).is_err() {
            return;
        }
    }
}

pub struct Client {
    endpoint: Endpoint,
    token: String,
}

impl Client {
    pub fn new(endpoint: Endpoint, token: String) -> Client {
        Client { endpoint, token }
    }

    /// Connect using the token file of the endpoint.
    pub fn from_endpoint(endpoint: Endpoint) -> Result<Client> {
        let token = read_token_file(&endpoint.token_file)?;
        Ok(Client { endpoint, token })
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let conn = Stream::connect(self.endpoint.to_name()?)?;
        let mut reader = BufReader::new(conn);
        let req = Request { id: 1, token: self.token.clone(), method: method.into(), params };
        let mut data = serde_json::to_vec(&req)?;
        data.push(b'\n');
        reader.get_mut().write_all(&data)?;
        let mut line = String::new();
        (&mut reader).take(MAX_LINE).read_line(&mut line)?;
        if line.is_empty() {
            return Err(Error::Other("o Agent encerrou a conexão".into()));
        }
        let resp: Response = serde_json::from_str(&line)?;
        if resp.ok {
            Ok(resp.result.unwrap_or(Value::Null))
        } else {
            let e = resp.error.unwrap_or(RpcError { code: "unknown".into(), message: "erro".into() });
            Err(Error::Other(format!("[{}] {}", e.code, e.message)))
        }
    }

    pub fn is_alive(&self) -> bool {
        self.call("ping", Value::Null).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_rejects_bad_token() {
        let dir = tempfile::tempdir().unwrap();
        let ep = Endpoint::default_for(dir.path());
        let token = write_token_file(&ep.token_file).unwrap();
        let handler: Handler = Arc::new(|m, p| match m {
            "echo" => Ok(p),
            _ => Err(Error::NotFound(m.into())),
        });
        let (server, _h) = Server::bind(ep.clone(), token.clone(), handler).unwrap();
        let c = Client::new(ep.clone(), token);
        assert_eq!(c.call("echo", serde_json::json!({"a": 1})).unwrap(), serde_json::json!({"a": 1}));
        assert!(c.call("nope", Value::Null).unwrap_err().to_string().contains("not_found"));
        let bad = Client::new(ep, "wrong".into());
        assert!(bad.call("echo", Value::Null).unwrap_err().to_string().contains("unauthorized"));
        server.stop();
    }
}
