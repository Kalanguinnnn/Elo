//! Desktop shell. All work happens in the Agent; this process only proxies
//! IPC calls from the web UI and starts the Agent when it is not running.

use continuity_core::ipc::{Client, Endpoint};
use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, Instant};

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

/// Connect to the Agent, starting it (detached) if needed.
fn ensure_agent() -> Result<Client, String> {
    if let Ok(c) = client()
        && c.is_alive()
    {
        return Ok(c);
    }
    let exe = agent_exe().ok_or("binário continuity-agent não encontrado ao lado do aplicativo")?;
    let mut cmd = std::process::Command::new(exe);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    continuity_core::git::hide_console(&mut cmd);
    cmd.spawn().map_err(|e| format!("não foi possível iniciar o Agent: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        if let Ok(c) = client()
            && c.is_alive()
        {
            return Ok(c);
        }
    }
    Err("o Agent não respondeu".into())
}

#[tauri::command]
async fn agent_call(method: String, params: Value) -> Result<Value, String> {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![agent_call, sync_and_quit])
        .run(tauri::generate_context!())
        .expect("erro ao executar o aplicativo");
}
