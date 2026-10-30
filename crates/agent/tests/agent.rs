use continuity_agent::{acquire_instance_lock, Agent, AgentOptions};
use continuity_core::credentials::MemoryStore;
use continuity_core::ide::FixedProbe;
use continuity_core::ipc::{Client, Endpoint};
use serde_json::json;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=T", "-c", "user.email=t@e", "-c", "core.autocrlf=false"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn agent_serves_ipc_watches_files_and_stops() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "1\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-m", "init"]);

    let _lock = acquire_instance_lock(&data).unwrap();
    assert!(acquire_instance_lock(&data).is_err(), "single instance");

    let endpoint = Endpoint::default_for(&data);
    let (agent, server) = Agent::start(AgentOptions {
        data_dir: data.clone(),
        tick: Duration::from_millis(300),
        askpass_exe: None,
        secrets: Arc::new(MemoryStore::default()),
        ide_probe: Arc::new(FixedProbe(true)),
        endpoint: Some(endpoint.clone()),
    })
    .unwrap();
    let agent = Arc::new(agent);
    let runner = {
        let a = agent.clone();
        std::thread::spawn(move || a.run(Duration::from_millis(300)))
    };

    let c = Client::from_endpoint(endpoint).unwrap();
    assert!(c.is_alive());
    let p = c.call("project.add", json!({ "path": repo.to_str().unwrap() })).unwrap();
    let pid = p["id"].as_str().unwrap().to_string();

    // Watcher picks up edits (visible through the IDE/quiet check).
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = false;
    while Instant::now() < deadline {
        std::fs::write(repo.join("a.txt"), format!("{:?}\n", Instant::now())).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let chk = c.call("ide.check", json!({ "project_id": pid })).unwrap();
        if chk["reason"].as_str().unwrap_or("").contains("modificados") {
            seen = true;
            break;
        }
    }
    assert!(seen, "file watcher did not report activity");

    let cp = c.call("checkpoint.create", json!({ "project_id": pid })).unwrap();
    assert!(cp["id"].is_string());
    let hist = c.call("history.list", json!({ "project_id": pid })).unwrap();
    assert_eq!(hist.as_array().unwrap().len(), 1);
    let st = c.call("agent.status", json!(null)).unwrap();
    assert!(st["watched"].as_array().unwrap().iter().any(|w| w == &json!(pid)));

    c.call("agent.stop", json!(null)).unwrap();
    runner.join().unwrap();
    server.stop();
}
