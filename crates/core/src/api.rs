//! Method table shared by the Agent's IPC server and the CLI's direct mode.
//! Every UI control maps to exactly one of these methods.

use crate::apply::{ApplyOptions, Resolution};
use crate::checkpoint::Trigger;
use crate::db::ProjectPatch;
use crate::engine::{AddProjectOptions, Engine, Settings, Side};
use crate::error::{Error, Result};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

fn p<T: DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(if v.is_null() { json!({}) } else { v }).map_err(|e| Error::Invalid(format!("parâmetros: {e}")))
}

fn ok<T: serde::Serialize>(v: T) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}

#[derive(Deserialize)]
struct ProjectId {
    project_id: String,
}
#[derive(Deserialize)]
struct Limit {
    project_id: String,
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    200
}
#[derive(Deserialize)]
struct CheckpointRef {
    project_id: String,
    checkpoint_id: String,
    #[serde(default)]
    options: ApplyOptions,
}

pub const METHODS: &[&str] = &[
    "ping", "device.get", "device.rename", "devices.list", "settings.get", "settings.set", "path.inspect",
    "projects.list", "project.add", "project.clone", "project.status", "project.update", "project.remove",
    "project.set_remote", "checkpoint.create", "history.list", "sync.project", "sync.push", "sync.fetch",
    "incoming.list", "apply.plan", "apply.run", "conflicts.list", "conflict.detail", "conflict.resolve",
    "conflict.abort", "compare", "file.view", "restore.files", "restore.dir", "operations.list", "operation.ack",
    "secrets.list", "secrets.approve", "retention.run", "queue.list", "activity.list", "ide.check", "github.status",
    "github.set_token", "github.logout", "github.device_start", "github.device_poll", "github.repos",
    "github.create_repo", "shutdown.sync",
];

pub fn dispatch(e: &Engine, method: &str, params: Value) -> Result<Value> {
    match method {
        "ping" => ok(json!({ "version": env!("CARGO_PKG_VERSION"), "device": e.device() })),
        "device.get" => ok(e.device()),
        "device.rename" => {
            #[derive(Deserialize)]
            struct A {
                name: String,
            }
            let a: A = p(params)?;
            ok(e.rename_device(&a.name)?)
        }
        "devices.list" => ok(e.devices()?),
        "settings.get" => ok(e.settings()?),
        "settings.set" => {
            #[derive(Deserialize)]
            struct A {
                settings: Settings,
            }
            let a: A = p(params)?;
            ok(e.set_settings(&a.settings)?)
        }
        "path.inspect" => {
            #[derive(Deserialize)]
            struct A {
                path: String,
            }
            let a: A = p(params)?;
            ok(e.inspect_path(&a.path)?)
        }
        "projects.list" => {
            let mut v = Vec::new();
            for pr in e.projects()? {
                v.push(e.project_status(&pr.id)?);
            }
            ok(v)
        }
        "project.add" => ok(e.add_project(&p::<AddProjectOptions>(params)?)?),
        "project.clone" => {
            #[derive(Deserialize)]
            struct A {
                url: String,
                dest: String,
            }
            let a: A = p(params)?;
            ok(e.clone_project(&a.url, &a.dest)?)
        }
        "project.status" => ok(e.project_status(&p::<ProjectId>(params)?.project_id)?),
        "project.update" => {
            #[derive(Deserialize)]
            struct A {
                project_id: String,
                patch: ProjectPatch,
            }
            let a: A = p(params)?;
            ok(e.update_project(&a.project_id, &a.patch)?)
        }
        "project.remove" => ok(e.remove_project(&p::<ProjectId>(params)?.project_id)?),
        "project.set_remote" => {
            #[derive(Deserialize)]
            struct A {
                project_id: String,
                url: String,
            }
            let a: A = p(params)?;
            ok(e.set_project_remote(&a.project_id, &a.url)?)
        }
        "checkpoint.create" => ok(e.checkpoint_now(&p::<ProjectId>(params)?.project_id, Trigger::Manual)?),
        "history.list" => {
            let a: Limit = p(params)?;
            ok(e.history(&a.project_id, a.limit)?)
        }
        "sync.project" => ok(e.sync_project(&p::<ProjectId>(params)?.project_id, false)?),
        "sync.push" => ok(e.push_pending(&p::<ProjectId>(params)?.project_id)?),
        "sync.fetch" => ok(e.fetch(&p::<ProjectId>(params)?.project_id)?),
        "incoming.list" => ok(e.incoming(&p::<ProjectId>(params)?.project_id)?),
        "apply.plan" => {
            let a: CheckpointRef = p(params)?;
            ok(e.plan_apply(&a.project_id, &a.checkpoint_id, &a.options)?)
        }
        "apply.run" => {
            let a: CheckpointRef = p(params)?;
            ok(e.apply(&a.project_id, &a.checkpoint_id, &a.options, false)?)
        }
        "conflicts.list" => {
            #[derive(Deserialize)]
            struct A {
                project_id: Option<String>,
            }
            let a: A = p(params)?;
            ok(e.conflicts(a.project_id.as_deref())?)
        }
        "conflict.detail" | "conflict.abort" | "conflict.resolve" => {
            #[derive(Deserialize)]
            struct A {
                conflict_id: String,
                #[serde(default)]
                resolutions: HashMap<String, Resolution>,
            }
            let a: A = p(params)?;
            match method {
                "conflict.detail" => ok(e.conflict_detail(&a.conflict_id)?),
                "conflict.abort" => ok(e.abort_conflict(&a.conflict_id)?),
                _ => ok(e.resolve_conflict(&a.conflict_id, &a.resolutions)?),
            }
        }
        "compare" => {
            #[derive(Deserialize)]
            struct A {
                project_id: String,
                from: Side,
                to: Side,
            }
            let a: A = p(params)?;
            ok(e.compare(&a.project_id, &a.from, &a.to)?)
        }
        "file.view" => {
            #[derive(Deserialize)]
            struct A {
                project_id: String,
                side: Side,
                path: String,
            }
            let a: A = p(params)?;
            ok(e.file_view(&a.project_id, &a.side, &a.path)?)
        }
        "restore.files" => {
            #[derive(Deserialize)]
            struct A {
                project_id: String,
                checkpoint_id: String,
                paths: Vec<String>,
            }
            let a: A = p(params)?;
            ok(e.restore_files(&a.project_id, &a.checkpoint_id, &a.paths)?)
        }
        "restore.dir" => {
            #[derive(Deserialize)]
            struct A {
                project_id: String,
                checkpoint_id: String,
                target: String,
            }
            let a: A = p(params)?;
            ok(e.restore_to_dir(&a.project_id, &a.checkpoint_id, &a.target)?)
        }
        "operations.list" => {
            let a: Limit = p(params)?;
            ok(e.db().operations(&a.project_id, a.limit)?)
        }
        "operation.ack" => {
            #[derive(Deserialize)]
            struct A {
                op_id: String,
            }
            let a: A = p(params)?;
            ok(e.acknowledge_operation(&a.op_id)?)
        }
        "secrets.list" => ok(e.findings(&p::<ProjectId>(params)?.project_id)?),
        "secrets.approve" => {
            #[derive(Deserialize)]
            struct A {
                project_id: String,
                fingerprints: Vec<String>,
            }
            let a: A = p(params)?;
            ok(e.approve_findings(&a.project_id, &a.fingerprints)?)
        }
        "retention.run" => ok(e.run_retention(&p::<ProjectId>(params)?.project_id)?),
        "queue.list" => ok(e.queue()?),
        "activity.list" => {
            #[derive(Deserialize)]
            struct A {
                project_id: Option<String>,
                #[serde(default = "default_limit")]
                limit: i64,
            }
            let a: A = p(params)?;
            ok(e.db().activity(a.project_id.as_deref(), a.limit.clamp(1, 2000))?)
        }
        "ide.check" => ok(e.ide_check(&p::<ProjectId>(params)?.project_id)),
        "github.status" => {
            let s = e.settings()?;
            ok(json!({
                "connected": e.github_token()?.is_some(),
                "login": e.github_login()?,
                "device_flow_available": !s.effective_client_id().is_empty(),
                "askpass_available": e.askpass_available(),
            }))
        }
        "github.set_token" => {
            #[derive(Deserialize)]
            struct A {
                token: String,
            }
            let a: A = p(params)?;
            ok(e.github_set_token(&a.token)?)
        }
        "github.logout" => ok(e.github_logout()?),
        "github.device_start" => {
            let cid = e.settings()?.effective_client_id();
            if cid.is_empty() {
                return Err(Error::Invalid("client id do OAuth App do GitHub não configurado".into()));
            }
            ok(crate::github::device_start(&cid)?)
        }
        "github.device_poll" => {
            #[derive(Deserialize)]
            struct A {
                device_code: String,
            }
            let a: A = p(params)?;
            let cid = e.settings()?.effective_client_id();
            match crate::github::device_poll(&cid, &a.device_code)? {
                crate::github::DevicePoll::Done { token } => {
                    let user = e.github_set_token(&token)?;
                    ok(json!({ "status": "done", "login": user.login }))
                }
                other => ok(other),
            }
        }
        "github.repos" => {
            let t = e.github_token()?.ok_or_else(|| Error::Credentials("GitHub não conectado".into()))?;
            ok(crate::github::list_repos(&t)?)
        }
        "github.create_repo" => {
            #[derive(Deserialize)]
            struct A {
                name: String,
                #[serde(default = "yes")]
                private: bool,
            }
            fn yes() -> bool {
                true
            }
            let a: A = p(params)?;
            let t = e.github_token()?.ok_or_else(|| Error::Credentials("GitHub não conectado".into()))?;
            ok(crate::github::create_repo(&t, &a.name, a.private, "Sincronizado pelo Continuity")?)
        }
        "shutdown.sync" => ok(e.sync_and_shutdown()?),
        other => Err(Error::NotFound(format!("método {other}"))),
    }
}
