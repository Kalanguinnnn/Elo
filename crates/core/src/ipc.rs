//! Local IPC between the Agent and its clients (desktop UI, CLI).
//!
//! Transport: a named pipe on Windows, a Unix domain socket in a 0700
//! directory on Linux. Protocol: one JSON object per line.
//!
//! Both sides share a per-user random token read from a file only the user
//! can read. The token itself never crosses the channel: each connection
//! starts with a nonce exchange in which the server proves it knows the
//! token (HMAC over both nonces) before the client sends anything else, and
//! every request then carries the client's own proof. A process that grabbed
//! the pipe name first (second security audit, finding 1) learns nothing and
//! is reported as an impostor. On Windows the pipe also gets a DACL limited
//! to the current user, and the client refuses a server run by another user.

use crate::error::{Error, Result};
use hmac::{Hmac, Mac};
use interprocess::local_socket::{prelude::*, GenericFilePath, GenericNamespaced, ListenerOptions, Stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const MAX_LINE: u64 = 32 * 1024 * 1024;
const MAX_HANDSHAKE_LINE: u64 = 4096;
const SERVER_TAG: &str = "elo-ipc-server-v1";
const CLIENT_TAG: &str = "elo-ipc-client-v1";

/// First line of a connection, client → server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub client_nonce: String,
}

/// Server → client: its nonce and proof that it knows the token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub server_nonce: String,
    pub server_proof: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    /// Client proof for this connection (see [`proof`]); never the token.
    pub auth: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// HMAC-SHA256(token, tag ‖ client nonce ‖ server nonce), hex.
fn proof(token: &str, tag: &str, client_nonce: &str, server_nonce: &str) -> String {
    let mut m = Hmac::<sha2::Sha256>::new_from_slice(token.as_bytes()).expect("HMAC aceita chaves de qualquer tamanho");
    for part in [tag, client_nonce, server_nonce] {
        m.update(part.as_bytes());
        m.update(b"\n");
    }
    hex::encode(m.finalize().into_bytes())
}

fn valid_nonce(n: &str) -> bool {
    n.len() == 64 && n.bytes().all(|b| b.is_ascii_hexdigit())
}

fn read_json_line<T: serde::de::DeserializeOwned>(reader: &mut BufReader<Stream>, limit: u64) -> Option<T> {
    let mut line = String::new();
    match reader.take(limit).read_line(&mut line) {
        Ok(n) if n > 0 && line.ends_with('\n') => serde_json::from_str(&line).ok(),
        _ => None,
    }
}

fn write_json_line<T: Serialize>(reader: &mut BufReader<Stream>, v: &T) -> std::io::Result<()> {
    let mut out = serde_json::to_vec(v).map_err(std::io::Error::other)?;
    out.push(b'\n');
    reader.get_mut().write_all(&out)
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
        let opts = ListenerOptions::new().name(endpoint.to_name()?).try_overwrite(cfg!(unix));
        #[cfg(windows)]
        let opts = {
            use interprocess::os::windows::local_socket::ListenerOptionsExt;
            opts.security_descriptor(winsec::owner_only_descriptor()?)
        };
        let listener = opts.create_sync().map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse || e.kind() == std::io::ErrorKind::PermissionDenied {
                Error::Other(format!("o canal do Agent ({}) já está em uso por outro processo: {e}", endpoint.name))
            } else {
                e.into()
            }
        })?;
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
    // Handshake: prove we know the token before the client sends anything.
    let Some(hello) = read_json_line::<Hello>(&mut reader, MAX_HANDSHAKE_LINE) else {
        let _ = write_json_line(&mut reader, &unauthorized(0, "handshake ausente ou inválido"));
        return;
    };
    if !valid_nonce(&hello.client_nonce) {
        let _ = write_json_line(&mut reader, &unauthorized(0, "nonce inválido"));
        return;
    }
    let server_nonce = crate::util::random_hex(32);
    let challenge = Challenge { server_proof: proof(token, SERVER_TAG, &hello.client_nonce, &server_nonce), server_nonce };
    if write_json_line(&mut reader, &challenge).is_err() {
        return;
    }
    let expected = proof(token, CLIENT_TAG, &hello.client_nonce, &challenge.server_nonce);

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
            Ok(req) if !constant_time_eq(&req.auth, &expected) => unauthorized(req.id, "token inválido"),
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
        if write_json_line(&mut reader, &resp).is_err() {
            return;
        }
    }
}

fn unauthorized(id: u64, message: &str) -> Response {
    Response { id, ok: false, result: None, error: Some(RpcError { code: "unauthorized".into(), message: message.into() }) }
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
        #[cfg(windows)]
        {
            // Defence in depth: a pipe served by another account is refused
            // outright. When the owner cannot be read, the handshake decides.
            if let Some(pid) = conn.peer_creds().ok().and_then(|c| c.pid())
                && winsec::process_user_matches(pid) == Some(false)
            {
                return Err(Error::Impostor("o processo que atende o canal pertence a outro usuário do Windows".into()));
            }
        }
        let mut reader = BufReader::new(conn);
        let client_nonce = crate::util::random_hex(32);
        write_json_line(&mut reader, &Hello { client_nonce: client_nonce.clone() })?;
        let Some(ch) = read_json_line::<Challenge>(&mut reader, MAX_HANDSHAKE_LINE) else {
            return Err(Error::Impostor("o processo no canal não respondeu ao handshake do Agent".into()));
        };
        if !valid_nonce(&ch.server_nonce)
            || !constant_time_eq(&ch.server_proof, &proof(&self.token, SERVER_TAG, &client_nonce, &ch.server_nonce))
        {
            return Err(Error::Impostor("o processo no canal não provou ser o Agent; nada foi enviado a ele".into()));
        }
        let auth = proof(&self.token, CLIENT_TAG, &client_nonce, &ch.server_nonce);
        let req = Request { id: 1, auth, method: method.into(), params };
        write_json_line(&mut reader, &req)?;
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

/// Windows account checks for the named pipe.
#[cfg(windows)]
mod winsec {
    use crate::error::{Error, Result};
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{EqualSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

    struct Owned(HANDLE);
    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the handle was returned open by the OS and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// TOKEN_USER of a process, in a u64 buffer so the struct is aligned.
    fn token_user(process: HANDLE) -> Option<Vec<u64>> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: valid process handle and out-pointer.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let token = Owned(token);
        let mut len = 0u32;
        // SAFETY: size query with a null buffer.
        unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut len) };
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        // SAFETY: the buffer holds at least `len` bytes.
        let ok = unsafe { GetTokenInformation(token.0, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) };
        (ok != 0).then_some(buf)
    }

    fn sid_of(buf: &[u64]) -> *mut core::ffi::c_void {
        // SAFETY: `buf` was filled by GetTokenInformation(TokenUser).
        unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid }
    }

    fn current_user() -> Option<Vec<u64>> {
        // SAFETY: pseudo-handle of the current process; never closed.
        token_user(unsafe { GetCurrentProcess() })
    }

    /// Whether process `pid` runs as the current user; `None` when unknown.
    pub fn process_user_matches(pid: u32) -> Option<bool> {
        let me = current_user()?;
        // SAFETY: plain OpenProcess; a null result is handled.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if h.is_null() {
            return None;
        }
        let h = Owned(h);
        let other = token_user(h.0)?;
        // SAFETY: both SIDs point into live TOKEN_USER buffers.
        Some(unsafe { EqualSid(sid_of(&me), sid_of(&other)) } != 0)
    }

    /// Protected DACL granting access to the current user only.
    pub fn owner_only_descriptor() -> Result<SecurityDescriptor> {
        let me = current_user().ok_or_else(|| Error::Other("não foi possível ler o usuário atual do Windows".into()))?;
        let mut raw: *mut u16 = std::ptr::null_mut();
        // SAFETY: valid SID; the output string is freed with LocalFree below.
        if unsafe { ConvertSidToStringSidW(sid_of(&me), &mut raw) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: `raw` is a NUL-terminated wide string from the OS.
        let sid = unsafe { widestring::U16CStr::from_ptr_str(raw) }.to_string_lossy();
        // SAFETY: allocated by ConvertSidToStringSidW.
        unsafe { LocalFree(raw.cast()) };
        let sddl = widestring::U16CString::from_str(format!("D:P(A;;GA;;;{sid})")).map_err(|e| Error::Other(e.to_string()))?;
        Ok(SecurityDescriptor::deserialize(&sddl)?)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn own_process_matches_and_descriptor_builds() {
            assert_eq!(super::process_user_matches(std::process::id()), Some(true));
            super::owner_only_descriptor().unwrap();
        }
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
        // A client with the wrong token cannot verify the server either.
        let bad = Client::new(ep.clone(), "wrong".into());
        assert!(matches!(bad.call("echo", Value::Null), Err(Error::Impostor(_))));
        // A client that skips the handshake or forges its proof is refused.
        let raw = |first: String| {
            let mut r = BufReader::new(Stream::connect(ep.to_name().unwrap()).unwrap());
            r.get_mut().write_all(first.as_bytes()).unwrap();
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            (r, line)
        };
        let (_, line) = raw(format!("{}\n", serde_json::json!({"id": 1, "token": token_hint(), "method": "echo"})));
        assert!(line.contains("unauthorized"), "{line}");
        let (mut r, line) = raw(format!("{}\n", serde_json::json!({"client_nonce": "ab".repeat(32)})));
        assert!(line.contains("server_proof"), "{line}");
        write_json_line(&mut r, &Request { id: 7, auth: "00".repeat(32), method: "echo".into(), params: Value::Null }).unwrap();
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        assert!(line.contains("unauthorized"), "{line}");
        server.stop();
    }

    fn token_hint() -> String {
        "f".repeat(64)
    }

    /// Second security audit, finding 1: a process that grabbed the
    /// endpoint name before the Agent must receive neither the token nor the
    /// request, and the client must say why.
    #[test]
    fn client_sends_nothing_secret_to_a_squatter() {
        let dir = tempfile::tempdir().unwrap();
        let ep = Endpoint::default_for(dir.path());
        let token = write_token_file(&ep.token_file).unwrap();
        #[cfg(unix)]
        std::fs::create_dir_all(Path::new(&ep.name).parent().unwrap()).unwrap();
        let squatter = Arc::new(ListenerOptions::new().name(ep.to_name().unwrap()).create_sync().unwrap());
        let seen = Arc::new(std::sync::Mutex::new(String::new()));
        let (seen2, squatter2) = (seen.clone(), squatter.clone());
        std::thread::spawn(move || {
            for conn in squatter2.incoming().take(2) {
                let mut r = BufReader::new(conn.unwrap());
                // Answer every line as an old-style Agent would, and try a
                // forged challenge too.
                for _ in 0..3 {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    seen2.lock().unwrap().push_str(&line);
                    let reply = r#"{"id":1,"ok":true,"result":{},"server_nonce":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","server_proof":"bb"}"#;
                    let _ = r.get_mut().write_all(format!("{reply}\n").as_bytes());
                }
            }
        });

        let c = Client::new(ep.clone(), token.clone());
        let err = c.call("github.set_token", serde_json::json!({"token": "ghp_secret_value"})).unwrap_err();
        assert!(matches!(err, Error::Impostor(_)), "{err}");
        assert!(!c.is_alive(), "a ping answered by the squatter is not proof of the Agent");
        let seen = seen.lock().unwrap().clone();
        assert!(!seen.contains(&token), "token leaked: {seen}");
        assert!(!seen.contains("ghp_secret_value") && !seen.contains("github.set_token"), "request leaked: {seen}");

        // The real Agent cannot take over (or share) a name someone else holds.
        #[cfg(windows)]
        {
            let handler: Handler = Arc::new(|_, p| Ok(p));
            let err = Server::bind(ep, token, handler).err().expect("bind must fail while squatted");
            assert!(err.to_string().contains("em uso"), "{err}");
        }
        drop(squatter);
    }

    #[test]
    fn proofs_depend_on_token_role_and_both_nonces() {
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        let p = proof("t", SERVER_TAG, &a, &b);
        assert_eq!(p.len(), 64);
        assert_ne!(p, proof("u", SERVER_TAG, &a, &b));
        assert_ne!(p, proof("t", CLIENT_TAG, &a, &b));
        assert_ne!(p, proof("t", SERVER_TAG, &b, &a));
        assert!(valid_nonce(&a) && !valid_nonce("zz") && !valid_nonce(&"g".repeat(64)));
    }
}
