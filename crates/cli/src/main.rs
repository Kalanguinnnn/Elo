//! `continuity` — command-line client. Talks to the running Agent over local
//! IPC; `--direct` runs the engine in-process instead (no Agent needed).

use continuity_core::credentials::{askpass_answer, KeyringStore};
use continuity_core::engine::{Engine, EngineConfig};
use continuity_core::ide::SystemProbe;
use continuity_core::ipc::{Client, Endpoint};
use serde_json::{json, Value};
use std::sync::Arc;

const USAGE: &str = "continuity — sincronização de projetos Git

USO: continuity [--direct] <comando> [args]

  status                              estado de todos os projetos
  add <pasta> [--init] [--remote URL] adicionar projeto (--init cria o repositório Git)
  clone <url> <pasta>                 clonar e adicionar projeto
  checkpoint <projeto>                criar checkpoint agora
  sync <projeto>                      enviar pendentes e buscar atualizações
  incoming <projeto>                  atualizações de outros dispositivos
  plan <projeto> <checkpoint>         simular aplicação (nada é alterado)
  apply <projeto> <checkpoint> [--allow-branch-switch] [--files-only]
  history <projeto>                   histórico de checkpoints
  restore <projeto> <checkpoint> --to <pasta-vazia>
  restore <projeto> <checkpoint> --files <arq>...
  trust <projeto> on|off              confiança (aplicação automática)
  conflicts | activity | queue | devices
  device-name <nome>                  renomear este computador
  sync-and-quit                       checkpoint + envio de tudo e encerra o Agent
  agent-status | agent-stop
  call <método> [json]                chamada IPC genérica

<projeto> aceita id, prefixo de id, nome ou caminho.";

enum Backend {
    Agent(Client),
    Direct(Box<Engine>),
}

impl Backend {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        match self {
            Backend::Agent(c) => c.call(method, params).map_err(|e| e.to_string()),
            Backend::Direct(e) => continuity_core::api::dispatch(e, method, params).map_err(|e| e.to_string()),
        }
    }
}

fn resolve_project(b: &Backend, key: &str) -> Result<String, String> {
    let list = b.call("projects.list", Value::Null)?;
    let arr = list.as_array().cloned().unwrap_or_default();
    let mut hits = Vec::new();
    for s in &arr {
        let p = &s["project"];
        let id = p["id"].as_str().unwrap_or("");
        let name = p["name"].as_str().unwrap_or("");
        let path = p["path"].as_str().unwrap_or("");
        if id == key || name.eq_ignore_ascii_case(key) || path == key || (key.len() >= 4 && id.starts_with(key)) {
            hits.push(id.to_string());
        }
    }
    match hits.len() {
        1 => Ok(hits.remove(0)),
        0 => Err(format!("projeto não encontrado: {key}")),
        _ => Err(format!("'{key}' é ambíguo")),
    }
}

fn print(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn main() {
    if std::env::var_os("CONTINUITY_ASKPASS").is_some() {
        let prompt = std::env::args().nth(1).unwrap_or_default();
        match askpass_answer(&prompt, &KeyringStore) {
            Some(a) => {
                println!("{a}");
                return;
            }
            None => std::process::exit(1),
        }
    }
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let direct = args.first().is_some_and(|a| a == "--direct");
    if direct {
        args.remove(0);
    }
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        println!("{USAGE}");
        return;
    }
    let data_dir = continuity_core::default_data_dir();
    let backend = if direct {
        match Engine::open(EngineConfig {
            data_dir,
            askpass_exe: std::env::current_exe().ok(),
            secrets: Arc::new(KeyringStore),
            ide_probe: Arc::new(SystemProbe),
        }) {
            Ok(e) => Backend::Direct(Box::new(e)),
            Err(e) => fail(&e.to_string()),
        }
    } else {
        match Client::from_endpoint(Endpoint::default_for(&data_dir)) {
            Ok(c) if c.is_alive() => Backend::Agent(c),
            _ => fail("o Agent não está em execução (inicie o app ou `continuity-agent`), ou use --direct"),
        }
    };
    if let Err(e) = run(&backend, &args) {
        fail(&e);
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("erro: {msg}");
    std::process::exit(1)
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn need<'a>(args: &'a [String], i: usize, what: &str) -> Result<&'a str, String> {
    args.get(i).map(String::as_str).ok_or_else(|| format!("faltando {what}\n\n{USAGE}"))
}

fn run(b: &Backend, args: &[String]) -> Result<(), String> {
    let cmd = args[0].as_str();
    let out = match cmd {
        "status" => {
            let v = b.call("projects.list", Value::Null)?;
            for s in v.as_array().cloned().unwrap_or_default() {
                let p = &s["project"];
                println!(
                    "{}  {:<24} branch={} pendentes={} recebidos={} conflitos={} confiável={}{}",
                    &p["id"].as_str().unwrap_or("")[..8],
                    p["name"].as_str().unwrap_or(""),
                    s["branch"].as_str().unwrap_or("-"),
                    s["pending_push"],
                    s["incoming"],
                    s["open_conflicts"],
                    p["trusted"],
                    s["queue_error"].as_str().map(|e| format!("  ⚠ {e}")).unwrap_or_default()
                );
            }
            return Ok(());
        }
        "add" => {
            let path = std::fs::canonicalize(need(args, 1, "pasta")?).map_err(|e| e.to_string())?;
            let path = path.to_string_lossy().trim_start_matches(r"\\?\").to_string();
            b.call(
                "project.add",
                json!({ "path": path, "init_git": flag(args, "--init"), "remote_url": opt(args, "--remote") }),
            )?
        }
        "clone" => b.call("project.clone", json!({ "url": need(args, 1, "url")?, "dest": need(args, 2, "pasta")? }))?,
        "checkpoint" => b.call("checkpoint.create", json!({ "project_id": resolve_project(b, need(args, 1, "projeto")?)? }))?,
        "sync" => b.call("sync.project", json!({ "project_id": resolve_project(b, need(args, 1, "projeto")?)? }))?,
        "incoming" => b.call("incoming.list", json!({ "project_id": resolve_project(b, need(args, 1, "projeto")?)? }))?,
        "history" => b.call("history.list", json!({ "project_id": resolve_project(b, need(args, 1, "projeto")?)?, "limit": 50 }))?,
        "plan" | "apply" => {
            let pid = resolve_project(b, need(args, 1, "projeto")?)?;
            let options = json!({ "allow_branch_switch": flag(args, "--allow-branch-switch"), "files_only": flag(args, "--files-only") });
            let method = if cmd == "plan" { "apply.plan" } else { "apply.run" };
            b.call(method, json!({ "project_id": pid, "checkpoint_id": need(args, 2, "checkpoint")?, "options": options }))?
        }
        "restore" => {
            let pid = resolve_project(b, need(args, 1, "projeto")?)?;
            let cp = need(args, 2, "checkpoint")?;
            if let Some(dir) = opt(args, "--to") {
                b.call("restore.dir", json!({ "project_id": pid, "checkpoint_id": cp, "target": dir }))?
            } else if let Some(i) = args.iter().position(|a| a == "--files") {
                let files: Vec<&String> = args[i + 1..].iter().collect();
                b.call("restore.files", json!({ "project_id": pid, "checkpoint_id": cp, "paths": files }))?
            } else {
                return Err("use --to <pasta> ou --files <arquivos>".into());
            }
        }
        "trust" => {
            let pid = resolve_project(b, need(args, 1, "projeto")?)?;
            let on = match need(args, 2, "on|off")? {
                "on" => true,
                "off" => false,
                _ => return Err("use on|off".into()),
            };
            b.call("project.update", json!({ "project_id": pid, "patch": { "trusted": on } }))?
        }
        "conflicts" => b.call("conflicts.list", json!({}))?,
        "activity" => {
            let v = b.call("activity.list", json!({ "limit": 40 }))?;
            for a in v.as_array().cloned().unwrap_or_default().iter().rev() {
                println!(
                    "{} {:5} {}",
                    continuity_core::util::ts_to_rfc3339(a["ts_ms"].as_i64().unwrap_or(0)),
                    a["level"].as_str().unwrap_or(""),
                    a["message"].as_str().unwrap_or("")
                );
            }
            return Ok(());
        }
        "queue" => b.call("queue.list", Value::Null)?,
        "devices" => b.call("devices.list", Value::Null)?,
        "device-name" => b.call("device.rename", json!({ "name": need(args, 1, "nome")? }))?,
        "sync-and-quit" => match b {
            Backend::Agent(_) => b.call("agent.sync_and_quit", Value::Null)?,
            Backend::Direct(_) => b.call("shutdown.sync", Value::Null)?,
        },
        "agent-status" => b.call("agent.status", Value::Null)?,
        "agent-stop" => b.call("agent.stop", Value::Null)?,
        "call" => {
            let params = match args.get(2) {
                Some(s) => serde_json::from_str(s).map_err(|e| e.to_string())?,
                None => Value::Null,
            };
            b.call(need(args, 1, "método")?, params)?
        }
        other => return Err(format!("comando desconhecido: {other}\n\n{USAGE}")),
    };
    print(&out);
    Ok(())
}
