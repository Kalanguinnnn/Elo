//! Desktop shell. All work happens in the Agent; this process only proxies
//! IPC calls from the web UI and starts the Agent when it is not running.

use continuity_core::ipc::{Client, Endpoint};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Set while an update is being installed: the Agent was stopped so the
/// installer can replace its executable, and must not be started again.
static UPDATING: AtomicBool = AtomicBool::new(false);

fn agent_exe() -> Option<PathBuf> {
    let name = if cfg!(windows) { "continuity-agent.exe" } else { "continuity-agent" };
    let here = std::env::current_exe().ok()?;
    let dir = here.parent()?;
    let p = dir.join(name);
    p.exists().then_some(p)
}

fn client() -> Result<Client, String> {
    Client::from_endpoint(Endpoint::default_for(&continuity_core::default_data_dir())).map_err(|e| e.to_string())
}

/// `Ok` when the Agent answers; `Err(Some)` when a process that failed to
/// prove it is the Agent holds the channel (never spawn over it or send it
/// anything); `Err(None)` when nothing usable is listening.
fn probe() -> Result<Client, Option<String>> {
    for attempt in 0..2 {
        let c = client().map_err(|_| None)?;
        match c.call("ping", Value::Null) {
            Ok(_) => return Ok(c),
            // The token file is re-read on retry: the Agent may have just
            // restarted with a new token.
            Err(continuity_core::Error::Impostor(_)) if attempt == 0 => std::thread::sleep(Duration::from_millis(500)),
            Err(e @ continuity_core::Error::Impostor(_)) => {
                return Err(Some(format!(
                    "{e}. Outro processo está ocupando o canal do Agent do Elo; nenhuma informação foi enviada a ele. \
                     Reinicie o computador e, se persistir, verifique os processos de outros usuários."
                )));
            }
            Err(_) => return Err(None),
        }
    }
    Err(None)
}

/// Connect to the Agent, starting it (detached) if needed.
fn ensure_agent() -> Result<Client, String> {
    match probe() {
        Ok(c) => return Ok(c),
        Err(Some(msg)) => return Err(msg),
        Err(None) => {}
    }
    let exe = agent_exe().ok_or("binário continuity-agent não encontrado ao lado do aplicativo")?;
    let mut cmd = std::process::Command::new(exe);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    continuity_core::git::hide_console(&mut cmd);
    cmd.spawn().map_err(|e| format!("não foi possível iniciar o Agent: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        match probe() {
            Ok(c) => return Ok(c),
            Err(Some(msg)) => return Err(msg),
            Err(None) => {}
        }
    }
    Err("o Agent não respondeu".into())
}

#[tauri::command]
async fn agent_call(method: String, params: Value) -> Result<Value, String> {
    if UPDATING.load(Ordering::SeqCst) {
        return Err("atualização do Elo em andamento".into());
    }
    tauri::async_runtime::spawn_blocking(move || ensure_agent()?.call(&method, params).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// Checkpoint + push everything, stop the Agent, close the app.
#[tauri::command]
async fn sync_and_quit(app: tauri::AppHandle) -> Result<Value, String> {
    let rep = tauri::async_runtime::spawn_blocking(|| ensure_agent()?.call("agent.sync_and_quit", Value::Null).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())??;
    let failed = rep.as_array().map(|a| a.iter().any(|r| !r["error"].is_null() || r["held"].as_u64().unwrap_or(0) > 0)).unwrap_or(false);
    if !failed {
        app.exit(0);
    }
    Ok(rep)
}

/// Stop the Agent before an update replaces its executable, and keep it
/// stopped. Pending work stays in the persistent queue and the new version's
/// Agent resumes it. Returns once the process has actually exited.
#[tauri::command]
async fn stop_agent_for_update() -> Result<(), String> {
    UPDATING.store(true, Ordering::SeqCst);
    let res = tauri::async_runtime::spawn_blocking(|| {
        let c = match probe() {
            Ok(c) => c,
            Err(None) => return Ok(()),
            Err(Some(msg)) => return Err(msg),
        };
        let pid = c.call("agent.status", Value::Null).ok().and_then(|v| v["pid"].as_u64()).map(|p| p as u32);
        c.call("agent.stop", Value::Null).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let exited = pid.is_none_or(|p| !continuity_core::util::process_alive(p));
            if exited && probe().is_err() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err("o Agent não encerrou a tempo; feche o Elo e instale a nova versão manualmente".to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    if res.is_err() {
        UPDATING.store(false, Ordering::SeqCst);
    }
    res
}

/// The update failed after the Agent was stopped: allow starting it again.
#[tauri::command]
fn resume_after_failed_update() {
    UPDATING.store(false, Ordering::SeqCst);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![agent_call, sync_and_quit, stop_agent_for_update, resume_after_failed_update])
        .run(tauri::generate_context!())
        .expect("erro ao executar o aplicativo");
}
