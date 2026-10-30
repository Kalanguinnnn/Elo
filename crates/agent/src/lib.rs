//! The Continuity background Agent.
//!
//! Threads:
//! * IPC accept loop (+ one thread per client connection);
//! * file watcher callback (marks projects dirty, records activity);
//! * scheduler tick: hybrid checkpoints (events + periodic reconciliation),
//!   queue processing, periodic fetch/auto-apply, daily retention;
//! * short-lived workers for network syncs, never two per project.

pub mod autostart;
pub mod logger;

use continuity_core::checkpoint::Trigger;
use continuity_core::credentials::SecretStore;
use continuity_core::engine::{Engine, EngineConfig};
use continuity_core::ide::IdeProbe;
use continuity_core::ipc::{self, Endpoint, Handler, Server};
use continuity_core::util::now_ms;
use continuity_core::{Error, Result};
use notify::{EventKind, RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct AgentOptions {
    pub data_dir: PathBuf,
    pub tick: Duration,
    pub askpass_exe: Option<PathBuf>,
    pub secrets: Arc<dyn SecretStore>,
    pub ide_probe: Arc<dyn IdeProbe>,
    /// Override the IPC endpoint (tests); defaults to the per-user endpoint.
    pub endpoint: Option<Endpoint>,
}

#[derive(Default)]
struct Runtime {
    started_ms: i64,
    last_tick_ms: i64,
    dirty: HashSet<String>,
    last_attempt: HashMap<String, i64>,
    last_reconcile: HashMap<String, i64>,
    last_fetch: HashMap<String, i64>,
    last_retention: HashMap<String, i64>,
    watch_errors: HashMap<String, String>,
    watched: HashMap<String, PathBuf>,
    in_flight: HashSet<String>,
}

pub struct Agent {
    pub engine: Arc<Engine>,
    rt: Arc<Mutex<Runtime>>,
    stop: Arc<AtomicBool>,
    endpoint: Endpoint,
}

/// Hold the single-instance lock for the lifetime of the Agent.
pub struct InstanceLock(#[allow(dead_code)] std::fs::File);

pub fn acquire_instance_lock(data_dir: &Path) -> Result<InstanceLock> {
    std::fs::create_dir_all(data_dir)?;
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(data_dir.join("agent.lock"))?;
    match f.try_lock() {
        Ok(()) => Ok(InstanceLock(f)),
        Err(std::fs::TryLockError::WouldBlock) => Err(Error::Busy("outro Agent já está em execução".into())),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

fn is_inside_git_dir(p: &Path, root: &Path) -> bool {
    p.strip_prefix(root)
        .map(|rel| rel.components().any(|c| matches!(c, Component::Normal(n) if n.eq_ignore_ascii_case(".git"))))
        .unwrap_or(true)
}

impl Agent {
    pub fn start(opts: AgentOptions) -> Result<(Agent, Server)> {
        ipc::restrict_dir(&opts.data_dir);
        let engine = Arc::new(Engine::open(EngineConfig {
            data_dir: opts.data_dir.clone(),
            askpass_exe: opts.askpass_exe.clone(),
            secrets: opts.secrets.clone(),
            ide_probe: opts.ide_probe.clone(),
        })?);
        let endpoint = opts.endpoint.clone().unwrap_or_else(|| Endpoint::default_for(&opts.data_dir));
        let token = ipc::write_token_file(&endpoint.token_file)?;
        let rt = Arc::new(Mutex::new(Runtime { started_ms: now_ms(), ..Default::default() }));
        let stop = Arc::new(AtomicBool::new(false));
        let agent = Agent { engine: engine.clone(), rt: rt.clone(), stop: stop.clone(), endpoint: endpoint.clone() };

        let handler: Handler = {
            let engine = engine.clone();
            let rt = rt.clone();
            let stop = stop.clone();
            let data_dir = opts.data_dir.clone();
            Arc::new(move |method: &str, params: Value| agent_method(&engine, &rt, &stop, &data_dir, method, params))
        };
        let (server, _accept) = Server::bind(endpoint, token, handler)?;
        engine.db().log(None, "info", "agent_started", &format!("Agent iniciado (v{})", env!("CARGO_PKG_VERSION")))?;
        Ok((agent, server))
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }

    /// Run watcher + scheduler until stopped.
    pub fn run(&self, tick: Duration) {
        let projects_for_watch: Arc<Mutex<Vec<(String, PathBuf)>>> = Arc::default();
        let watcher = {
            let engine = self.engine.clone();
            let rt = self.rt.clone();
            let roots = projects_for_watch.clone();
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(ev) = res else { return };
                if matches!(ev.kind, EventKind::Access(_)) {
                    return;
                }
                let roots = roots.lock().unwrap().clone();
                for path in &ev.paths {
                    let hit = roots
                        .iter()
                        .filter(|(_, r)| path.starts_with(r))
                        .max_by_key(|(_, r)| r.as_os_str().len());
                    if let Some((id, root)) = hit {
                        if is_inside_git_dir(path, root) {
                            continue;
                        }
                        engine.note_fs_event(id);
                        rt.lock().unwrap().dirty.insert(id.clone());
                    }
                }
            })
        };
        let mut watcher = match watcher {
            Ok(w) => Some(w),
            Err(e) => {
                log::warn!("watcher indisponível: {e}");
                let _ = self.engine.db().log(None, "warn", "watcher_unavailable", &format!("Monitoramento de arquivos indisponível ({e}); usando apenas reconciliação periódica"));
                None
            }
        };

        while !self.stop.load(Ordering::SeqCst) {
            if let Err(e) = self.tick(&mut watcher, &projects_for_watch) {
                log::error!("tick: {e}");
            }
            let step = Duration::from_millis(200);
            let mut waited = Duration::ZERO;
            while waited < tick && !self.stop.load(Ordering::SeqCst) {
                std::thread::sleep(step);
                waited += step;
            }
        }
        // Let in-flight syncs finish (bounded wait).
        for _ in 0..150 {
            if self.rt.lock().unwrap().in_flight.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = self.engine.db().log(None, "info", "agent_stopped", "Agent encerrado");
    }

    fn tick(&self, watcher: &mut Option<notify::RecommendedWatcher>, roots: &Arc<Mutex<Vec<(String, PathBuf)>>>) -> Result<()> {
        let now = now_ms();
        let settings = self.engine.settings()?;
        let projects = self.engine.projects()?;
        self.rt.lock().unwrap().last_tick_ms = now;

        // Keep the watch set in sync with active projects.
        {
            let mut rt = self.rt.lock().unwrap();
            let active: HashMap<String, PathBuf> =
                projects.iter().filter(|p| !p.paused).map(|p| (p.id.clone(), PathBuf::from(&p.path))).collect();
            if let Some(w) = watcher.as_mut() {
                let stale: Vec<String> = rt.watched.keys().filter(|k| !active.contains_key(*k)).cloned().collect();
                for id in stale {
                    if let Some(p) = rt.watched.remove(&id) {
                        let _ = w.unwatch(&p);
                    }
                }
                for (id, path) in &active {
                    if rt.watched.contains_key(id) || rt.watch_errors.contains_key(id) {
                        continue;
                    }
                    match w.watch(path, RecursiveMode::Recursive) {
                        Ok(()) => {
                            rt.watched.insert(id.clone(), path.clone());
                        }
                        Err(e) => {
                            rt.watch_errors.insert(id.clone(), e.to_string());
                        }
                    }
                }
            }
            *roots.lock().unwrap() = active.into_iter().collect();
        }

        for p in projects.iter().filter(|p| !p.paused) {
            let interval_ms = p.interval_secs.max(60) * 1000;
            let (due_checkpoint, trigger) = {
                let mut rt = self.rt.lock().unwrap();
                let last = *rt.last_attempt.entry(p.id.clone()).or_insert(now);
                let last_rec = *rt.last_reconcile.entry(p.id.clone()).or_insert(now);
                let dirty = rt.dirty.contains(&p.id);
                let watched = rt.watched.contains_key(&p.id);
                let elapsed = now - last >= interval_ms;
                if elapsed && dirty {
                    (true, Trigger::Watcher)
                } else if elapsed && (!watched || now - last_rec >= 3 * interval_ms) {
                    (true, Trigger::Timer)
                } else {
                    (false, Trigger::Timer)
                }
            };
            if due_checkpoint {
                {
                    let mut rt = self.rt.lock().unwrap();
                    rt.last_attempt.insert(p.id.clone(), now);
                    rt.dirty.remove(&p.id);
                    if trigger == Trigger::Timer {
                        rt.last_reconcile.insert(p.id.clone(), now);
                    }
                }
                match self.engine.checkpoint_now(&p.id, trigger) {
                    Ok(_) => {}
                    Err(Error::Busy(why)) => {
                        log::info!("checkpoint adiado ({why})");
                        self.rt.lock().unwrap().dirty.insert(p.id.clone());
                    }
                    Err(e) => {
                        let _ = self.engine.db().set_project_error(&p.id, Some(&e.to_string()));
                        let _ = self.engine.db().log(Some(&p.id), "error", "checkpoint_failed", &e.to_string());
                    }
                }
            }

            let fetch_due = {
                let mut rt = self.rt.lock().unwrap();
                let last = *rt.last_fetch.entry(p.id.clone()).or_insert(0);
                if now - last >= settings.fetch_interval_secs * 1000 && !rt.in_flight.contains(&p.id) {
                    rt.last_fetch.insert(p.id.clone(), now);
                    rt.in_flight.insert(p.id.clone());
                    true
                } else {
                    false
                }
            };
            if fetch_due {
                let engine = self.engine.clone();
                let rt = self.rt.clone();
                let id = p.id.clone();
                std::thread::spawn(move || {
                    if let Err(e) = engine.sync_project(&id, true) {
                        log::info!("sync {id}: {e}");
                    }
                    rt.lock().unwrap().in_flight.remove(&id);
                });
            }

            let retention_due = {
                let mut rt = self.rt.lock().unwrap();
                let last = *rt.last_retention.entry(p.id.clone()).or_insert(now);
                if now - last >= 24 * 3_600_000 {
                    rt.last_retention.insert(p.id.clone(), now);
                    true
                } else {
                    false
                }
            };
            if retention_due
                && let Err(e) = self.engine.run_retention(&p.id) {
                    log::warn!("retenção {}: {e}", p.id);
                }
        }
        // Drain whatever became due (pushes, remote deletions) between fetches.
        let busy: HashSet<String> = self.rt.lock().unwrap().in_flight.clone();
        if busy.is_empty() {
            self.engine.process_queue()?;
        }
        Ok(())
    }
}

fn agent_method(
    engine: &Arc<Engine>,
    rt: &Arc<Mutex<Runtime>>,
    stop: &Arc<AtomicBool>,
    data_dir: &Path,
    method: &str,
    params: Value,
) -> Result<Value> {
    match method {
        "agent.status" => {
            let rt = rt.lock().unwrap();
            Ok(json!({
                "pid": std::process::id(),
                "version": env!("CARGO_PKG_VERSION"),
                "started_ms": rt.started_ms,
                "last_tick_ms": rt.last_tick_ms,
                "data_dir": data_dir.display().to_string(),
                "watched": rt.watched.keys().collect::<Vec<_>>(),
                "watch_errors": rt.watch_errors,
                "syncing": rt.in_flight.iter().collect::<Vec<_>>(),
                "autostart": autostart::is_enabled(),
            }))
        }
        "agent.stop" => {
            stop.store(true, Ordering::SeqCst);
            Ok(json!({ "stopping": true }))
        }
        "agent.sync_and_quit" => {
            let rep = engine.sync_and_shutdown()?;
            stop.store(true, Ordering::SeqCst);
            Ok(serde_json::to_value(rep)?)
        }
        "agent.autostart" => {
            let enable = params.get("enable").and_then(|v| v.as_bool()).ok_or_else(|| Error::Invalid("enable".into()))?;
            let exe = std::env::current_exe()?;
            autostart::set(enable, &exe)?;
            let mut s = engine.settings()?;
            s.autostart = enable;
            engine.set_settings(&s)?;
            Ok(json!({ "autostart": autostart::is_enabled() }))
        }
        "agent.reset_watch" => {
            let mut rt = rt.lock().unwrap();
            rt.watch_errors.clear();
            Ok(Value::Null)
        }
        _ => continuity_core::api::dispatch(engine, method, params),
    }
}
