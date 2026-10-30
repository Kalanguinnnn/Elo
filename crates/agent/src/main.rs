// No console window on Windows: the Agent runs in the background.
#![cfg_attr(windows, windows_subsystem = "windows")]

use continuity_agent::{acquire_instance_lock, logger::FileLogger, Agent, AgentOptions};
use continuity_core::credentials::{askpass_answer, KeyringStore};
use continuity_core::ide::SystemProbe;
use std::sync::Arc;
use std::time::Duration;

fn main() {
    // GIT_ASKPASS mode: git runs us with the prompt as the only argument.
    if std::env::var_os("CONTINUITY_ASKPASS").is_some() {
        let prompt = std::env::args().nth(1).unwrap_or_default();
        match askpass_answer(&prompt, &KeyringStore) {
            Some(answer) => {
                println!("{answer}");
                std::process::exit(0);
            }
            None => std::process::exit(1),
        }
    }

    let data_dir = continuity_core::default_data_dir();
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("não foi possível criar {}: {e}", data_dir.display());
        std::process::exit(2);
    }
    FileLogger::install(data_dir.join("agent.log"));
    let _lock = match acquire_instance_lock(&data_dir) {
        Ok(l) => l,
        Err(e) => {
            log::info!("{e}");
            std::process::exit(0);
        }
    };
    let opts = AgentOptions {
        data_dir,
        tick: Duration::from_secs(5),
        askpass_exe: std::env::current_exe().ok(),
        secrets: Arc::new(KeyringStore),
        ide_probe: Arc::new(SystemProbe),
        endpoint: None,
    };
    let tick = opts.tick;
    let (agent, server) = match Agent::start(opts) {
        Ok(x) => x,
        Err(e) => {
            log::error!("falha ao iniciar o Agent: {e}");
            std::process::exit(1);
        }
    };
    log::info!("Agent em {}", agent.endpoint().name);
    agent.run(tick);
    server.stop();
}
