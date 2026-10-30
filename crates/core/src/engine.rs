//! High-level service used by the Agent, the CLI and tests. Owns the database
//! and serialises work per project.

use crate::apply::{self, ApplyOptions, ApplyPlan, ApplyResult, Resolution};
use crate::checkpoint::{self, CheckpointMeta, Trigger};
use crate::credentials::{self, SecretStore};
use crate::db::{CheckpointRow, ConflictRow, Db, Project, ProjectPatch};
use crate::device::{self, DeviceInfo};
use crate::error::{Error, FailureKind, Result};
use crate::git::{os_args, Git, RunOpts};
use crate::ide::{IdeCheck, IdeProbe};
use crate::restore::{self, RestoreReport};
use crate::retention::{self, RetentionPolicy};
use crate::secrets::{self, Severity};
use crate::snapshot::{self, RefCheckpoint, WorkState};
use crate::transport::{self, NetAuth};
use crate::util::now_ms;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

pub struct EngineConfig {
    pub data_dir: PathBuf,
    /// Executable git should run as `GIT_ASKPASS` (our own binary).
    pub askpass_exe: Option<PathBuf>,
    pub secrets: Arc<dyn SecretStore>,
    pub ide_probe: Arc<dyn IdeProbe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub default_interval_secs: i64,
    pub fetch_interval_secs: i64,
    /// No filesystem events for this long before auto-apply.
    pub quiet_secs: i64,
    pub autostart: bool,
    pub github_client_id: String,
    pub retention: RetentionPolicy,
    pub prune_remote: bool,
}

/// Client ID of the "Elo" OAuth App (device flow). Public by design: the
/// device flow uses no client secret.
pub const DEFAULT_GITHUB_CLIENT_ID: &str = "Ov23ctWvLpY22HembEji";

impl Settings {
    /// The user's override when set, otherwise the built-in OAuth App.
    pub fn effective_client_id(&self) -> String {
        let custom = self.github_client_id.trim();
        if custom.is_empty() { DEFAULT_GITHUB_CLIENT_ID.to_string() } else { custom.to_string() }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            default_interval_secs: 600,
            fetch_interval_secs: 300,
            quiet_secs: 120,
            autostart: false,
            github_client_id: std::env::var("CONTINUITY_GITHUB_CLIENT_ID").unwrap_or_default(),
            retention: RetentionPolicy::default(),
            prune_remote: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AddProjectOptions {
    pub path: String,
    #[serde(default)]
    pub init_git: bool,
    #[serde(default)]
    pub remote_url: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathInspection {
    pub path: String,
    pub exists: bool,
    pub is_repo: bool,
    pub workdir: Option<String>,
    pub branch: Option<String>,
    pub has_commits: bool,
    pub remotes: Vec<(String, String)>,
    pub already_added: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectStatus {
    pub project: Project,
    pub available: bool,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub remote_url: Option<String>,
    pub uncommitted_changes: Option<usize>,
    pub pending_push: usize,
    pub needs_review: usize,
    pub quarantined: usize,
    pub incoming: usize,
    pub open_conflicts: usize,
    pub interrupted_ops: usize,
    pub queue_error: Option<String>,
    pub queue_failure_kind: Option<String>,
    pub git_busy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incoming {
    pub checkpoint: CheckpointRow,
    pub same_branch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyPreview {
    pub checkpoint: CheckpointRow,
    pub plan: ApplyPlan,
    pub auto_eligible: bool,
    pub ide: IdeCheck,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ApplyOutcome {
    Applied { result: ApplyResult, safety_checkpoint: String, lineage_head: String },
    NothingToDo { safety_checkpoint: String },
    Conflicts { conflict_id: String, count: usize },
    Blocked { plan: ApplyPlan },
    NeedsManual { reason: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PushReport {
    pub pushed: Vec<String>,
    pub needs_review: Vec<String>,
    pub quarantined: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FetchReport {
    pub imported: usize,
    pub incoming: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShutdownReport {
    pub project_id: String,
    pub name: String,
    pub checkpoint: Option<String>,
    pub pushed: usize,
    pub held: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Side {
    Checkpoint { id: String },
    Head,
    Current,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileView {
    pub exists: bool,
    pub binary: bool,
    pub size: u64,
    pub oid: Option<String>,
    pub text: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictDetail {
    pub conflict: ConflictRow,
    pub plan: ApplyPlan,
    pub theirs_device: String,
    pub files: Vec<ConflictFileView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictFileView {
    pub path: String,
    pub reason: String,
    pub ours: FileView,
    pub theirs: FileView,
    pub base: FileView,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetentionReport {
    pub pruned_local: usize,
    pub queued_remote: usize,
    pub kept: usize,
}

const VIEW_LIMIT: u64 = 1024 * 1024;

pub struct Engine {
    db: Mutex<Db>,
    cfg: EngineConfig,
    device: Mutex<DeviceInfo>,
    work_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    net_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    fs_activity: Mutex<HashMap<String, i64>>,
    notified: Mutex<HashMap<String, String>>,
}

fn lock_for(map: &Mutex<HashMap<String, Arc<Mutex<()>>>>, id: &str) -> Arc<Mutex<()>> {
    map.lock().unwrap().entry(id.to_string()).or_default().clone()
}

fn guard(m: &Mutex<()>) -> MutexGuard<'_, ()> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Engine {
    pub fn open(cfg: EngineConfig) -> Result<Engine> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        let db = Db::open(&cfg.data_dir.join("continuity.db"))?;
        let dev = device::load_or_create(&db)?;
        for op in db.mark_interrupted_ops()? {
            db.log(
                Some(&op.project_id),
                "warn",
                "operation_interrupted",
                &format!(
                    "Operação '{}' foi interrompida (reinício). Checkpoint de segurança: {}",
                    op.kind,
                    op.safety_checkpoint.as_deref().unwrap_or("nenhum")
                ),
            )?;
        }
        let projects = db.projects()?;
        let engine = Engine {
            db: Mutex::new(db),
            cfg,
            device: Mutex::new(dev),
            work_locks: Mutex::default(),
            net_locks: Mutex::default(),
            fs_activity: Mutex::default(),
            notified: Mutex::default(),
        };
        for p in projects {
            if let Ok(git) = Git::open(Path::new(&p.path)) {
                snapshot::cleanup_stale_temp(&git);
            }
        }
        Ok(engine)
    }

    pub fn db(&self) -> MutexGuard<'_, Db> {
        self.db.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn log(&self, project: Option<&str>, level: &str, kind: &str, msg: &str) {
        log::info!("[{kind}] {msg}");
        let _ = self.db().log(project, level, kind, msg);
    }

    pub fn data_dir(&self) -> &Path {
        &self.cfg.data_dir
    }

    // ---- device ---------------------------------------------------------

    pub fn device(&self) -> DeviceInfo {
        self.device.lock().unwrap().clone()
    }

    pub fn rename_device(&self, name: &str) -> Result<DeviceInfo> {
        let name = crate::util::sanitize_line(name, 60);
        if name.is_empty() {
            return Err(Error::Invalid("nome vazio".into()));
        }
        let mut d = self.device.lock().unwrap();
        self.db().rename_local_device(&name)?;
        self.db().touch_device(&d.id, &name, now_ms())?;
        d.name = name;
        Ok(d.clone())
    }

    pub fn devices(&self) -> Result<Vec<crate::db::DeviceRow>> {
        let id = self.device().id;
        self.db().devices(&id)
    }

    // ---- settings -------------------------------------------------------

    pub fn settings(&self) -> Result<Settings> {
        Ok(match self.db().setting("settings")? {
            Some(s) => serde_json::from_str(&s).unwrap_or_default(),
            None => Settings::default(),
        })
    }

    pub fn set_settings(&self, s: &Settings) -> Result<Settings> {
        let mut s = s.clone();
        s.default_interval_secs = s.default_interval_secs.clamp(60, 86_400);
        s.fetch_interval_secs = s.fetch_interval_secs.clamp(30, 86_400);
        s.quiet_secs = s.quiet_secs.clamp(10, 3_600);
        self.db().set_setting("settings", &serde_json::to_string(&s)?)?;
        Ok(s)
    }

    // ---- projects -------------------------------------------------------

    pub fn inspect_path(&self, path: &str) -> Result<PathInspection> {
        let p = Path::new(path);
        let mut r = PathInspection {
            path: path.to_string(),
            exists: p.is_dir(),
            is_repo: false,
            workdir: None,
            branch: None,
            has_commits: false,
            remotes: vec![],
            already_added: None,
        };
        if !r.exists {
            return Ok(r);
        }
        if let Ok(git) = Git::open(p) {
            r.is_repo = true;
            r.workdir = Some(git.workdir.display().to_string());
            r.branch = git.head_branch()?.map(|b| checkpoint::short_branch(&b).to_string());
            r.has_commits = git.head_commit()?.is_some();
            r.remotes = git.remotes()?;
            r.already_added = self.db().project_by_path(&git.workdir.display().to_string())?.map(|p| p.id);
        }
        Ok(r)
    }

    pub fn add_project(&self, opts: &AddProjectOptions) -> Result<Project> {
        let path = Path::new(&opts.path);
        if !path.is_dir() {
            return Err(Error::NotFound(format!("diretório {}", opts.path)));
        }
        let git = match Git::open(path) {
            Ok(g) => g,
            Err(_) if opts.init_git => {
                let g = Git::init(path, "main")?;
                self.log(None, "info", "git_init", &format!("Repositório Git criado em {}", g.workdir.display()));
                g
            }
            Err(_) => {
                return Err(Error::NeedsConfirmation(
                    "a pasta não é um repositório Git; confirme a inicialização (init_git)".into(),
                ))
            }
        };
        let workdir = git.workdir.display().to_string();
        if let Some(p) = self.db().project_by_path(&workdir)? {
            return Ok(p);
        }
        if let Some(url) = opts.remote_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            self.set_remote_url(&git, "origin", url)?;
        }
        let name = opts
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| git.workdir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or("projeto".into()));
        let p = Project {
            id: uuid::Uuid::new_v4().to_string(),
            name: crate::util::sanitize_line(&name, 80),
            path: workdir,
            remote: "origin".into(),
            trusted: false,
            paused: false,
            interval_secs: self.settings()?.default_interval_secs,
            auto_push: true,
            autocommit: false,
            lineage_head: None,
            last_checkpoint_ms: None,
            last_fetch_ms: None,
            last_push_ms: None,
            last_error: None,
            created_ms: now_ms(),
        };
        self.db().insert_project(&p)?;
        self.import_refs(&p, &git)?;
        self.log(Some(&p.id), "info", "project_added", &format!("Projeto '{}' adicionado ({})", p.name, p.path));
        self.db().project(&p.id)
    }

    fn set_remote_url(&self, git: &Git, name: &str, url: &str) -> Result<()> {
        transport::validate_remote_url(url)?;
        transport::validate_remote_name(name)?;
        match git.config_get(&format!("remote.{name}.url"))? {
            Some(existing) if existing == url => Ok(()),
            Some(_) => {
                git.run(os_args!["remote", "set-url", name, url], RunOpts::default())?;
                Ok(())
            }
            None => {
                git.run(os_args!["remote", "add", name, url], RunOpts::default())?;
                Ok(())
            }
        }
    }

    pub fn set_project_remote(&self, project_id: &str, url: &str) -> Result<Project> {
        let p = self.db().project(project_id)?;
        let git = Git::open(Path::new(&p.path))?;
        self.set_remote_url(&git, &p.remote, url)?;
        self.log(Some(project_id), "info", "remote_set", &format!("Remoto {} configurado: {url}", p.remote));
        Ok(p)
    }

    /// Clone a remote into `dest` and register it as a project.
    pub fn clone_project(&self, url: &str, dest: &str) -> Result<Project> {
        transport::validate_remote_url(url)?;
        let dest_path = Path::new(dest);
        if dest_path.exists() && std::fs::read_dir(dest_path)?.next().is_some() {
            return Err(Error::Invalid("o destino do clone deve estar vazio".into()));
        }
        let parent = dest_path.parent().ok_or_else(|| Error::Invalid("destino inválido".into()))?;
        std::fs::create_dir_all(parent)?;
        let auth = self.auth_for_url(url);
        crate::git::run_global(
            parent,
            os_args!["clone", "--origin", "origin", "--", url, dest_path.as_os_str()],
            auth.apply(RunOpts::default()),
        )?;
        let p = self.add_project(&AddProjectOptions { path: dest.into(), ..Default::default() })?;
        if let Err(e) = self.fetch(&p.id) {
            self.log(Some(&p.id), "warn", "fetch_failed", &format!("Falha ao buscar checkpoints após clone: {e}"));
        }
        self.db().project(&p.id)
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        self.db().projects()
    }

    pub fn update_project(&self, id: &str, patch: &ProjectPatch) -> Result<Project> {
        let before = self.db().project(id)?;
        let p = self.db().update_project(id, patch)?;
        if patch.trusted == Some(true) && !before.trusted {
            self.log(Some(id), "warn", "trust_enabled", "Projeto marcado como confiável: atualizações seguras serão aplicadas automaticamente");
        }
        if patch.autocommit == Some(true) && !before.autocommit {
            self.log(Some(id), "warn", "autocommit_enabled", "Modo de commits automáticos ativado (altera a branch)");
        }
        if patch.auto_push == Some(true) && !before.auto_push {
            self.db().enqueue(id, "push", "")?;
        }
        Ok(p)
    }

    /// Stop tracking a project. Files, refs and the remote are left untouched.
    pub fn remove_project(&self, id: &str) -> Result<()> {
        let p = self.db().project(id)?;
        self.db().delete_project(id)?;
        self.log(None, "info", "project_removed", &format!("Projeto '{}' removido do Continuity (arquivos preservados)", p.name));
        Ok(())
    }

    fn open_git(&self, p: &Project) -> Result<Git> {
        Git::open(Path::new(&p.path))
    }

    pub fn project_status(&self, id: &str) -> Result<ProjectStatus> {
        let p = self.db().project(id)?;
        let mut st = ProjectStatus {
            project: p.clone(),
            available: false,
            branch: None,
            head: None,
            remote_url: None,
            uncommitted_changes: None,
            pending_push: 0,
            needs_review: 0,
            quarantined: 0,
            incoming: 0,
            open_conflicts: 0,
            interrupted_ops: 0,
            queue_error: None,
            queue_failure_kind: None,
            git_busy: None,
        };
        {
            let db = self.db();
            st.pending_push = db.checkpoints_by_state(id, "pending")?.len();
            st.needs_review = db.checkpoints_by_state(id, "needs_review")?.len();
            st.quarantined = db.checkpoints_by_state(id, "quarantined")?.len();
            st.open_conflicts = db.conflicts(Some(id), true)?.len();
            st.interrupted_ops = db.operations(id, 50)?.iter().filter(|o| o.state == "interrupted").count();
            if let Some(q) = db.queue(Some(id))?.into_iter().find(|q| q.last_error.is_some()) {
                st.queue_error = q.last_error;
                st.queue_failure_kind = q.failure_kind;
            }
        }
        if let Ok(git) = self.open_git(&p) {
            st.available = true;
            st.branch = git.head_branch()?.map(|b| checkpoint::short_branch(&b).to_string());
            st.head = git.head_commit()?;
            st.remote_url = git.config_get(&format!("remote.{}.url", p.remote))?;
            st.git_busy = snapshot::git_busy(&git);
            if let Ok(o) = git.run(os_args!["status", "--porcelain=v1", "-z", "--untracked-files=normal"], RunOpts::default()) {
                st.uncommitted_changes = Some(o.stdout.split(|b| *b == 0).filter(|r| r.len() > 3).count());
            }
            st.incoming = self.incoming(id).map(|v| v.len()).unwrap_or(0);
        }
        Ok(st)
    }

    // ---- checkpoints ----------------------------------------------------

    pub fn note_fs_event(&self, project_id: &str) {
        self.fs_activity.lock().unwrap().insert(project_id.to_string(), now_ms());
    }

    pub fn last_fs_event(&self, project_id: &str) -> Option<i64> {
        self.fs_activity.lock().unwrap().get(project_id).copied()
    }

    /// Create a checkpoint if the state changed since the lineage head.
    pub fn checkpoint_now(&self, project_id: &str, trigger: Trigger) -> Result<Option<CheckpointRow>> {
        let lock = lock_for(&self.work_locks, project_id);
        let _g = guard(&lock);
        let p = self.db().project(project_id)?;
        let git = self.open_git(&p)?;
        if p.autocommit && matches!(trigger, Trigger::Timer | Trigger::Watcher | Trigger::Manual | Trigger::Shutdown)
            && let Err(e) = crate::autocommit::auto_commit(self, &p, &git) {
                self.log(Some(project_id), "warn", "autocommit_skipped", &format!("Commit automático não realizado: {e}"));
            }
        let (row, _state) = self.checkpoint_locked(&p, &git, trigger, &[])?;
        Ok(row)
    }

    /// Requires the work lock. Returns (new row if created, captured state).
    fn checkpoint_locked(
        &self,
        p: &Project,
        git: &Git,
        trigger: Trigger,
        extra_parents: &[String],
    ) -> Result<(Option<CheckpointRow>, WorkState)> {
        if let Some(b) = snapshot::git_busy(git) {
            return Err(Error::Busy(format!("operação Git em andamento ({b})")));
        }
        let state = snapshot::capture(git)?;
        let p = self.db().project(&p.id)?; // fresh lineage head
        let head_row = match &p.lineage_head {
            Some(h) => self.db().checkpoint_opt(&p.id, h)?,
            None => None,
        };
        if extra_parents.is_empty()
            && let Some(h) = &head_row {
                let same = h.tree_oid == state.tree
                    && h.head_oid == state.head
                    && h.branch == state.branch
                    && git.tree_of(&h.index_commit).ok().as_deref() == Some(state.index_tree.as_str());
                if same {
                    return Ok((None, state));
                }
            }
        let dev = self.device();
        let created = now_ms();
        let id = checkpoint::new_id(created);
        let mut parents: Vec<String> = p.lineage_head.iter().cloned().collect();
        for e in extra_parents {
            if !parents.contains(e) {
                parents.push(e.clone());
            }
        }
        let compare_tree = match &head_row {
            Some(h) if git.has_object(&h.tree_oid) => h.tree_oid.clone(),
            _ => match &state.head {
                Some(h) => git.tree_of(h)?,
                None => git.empty_tree()?,
            },
        };
        let files_changed = snapshot::change_count(git, &compare_tree, &state.tree).unwrap_or(0);
        let summary = format!("{files_changed} arquivo(s) alterado(s)");
        let mut meta = CheckpointMeta {
            id: id.clone(),
            device_id: dev.id.clone(),
            device_name: dev.name.clone(),
            branch: None,
            head: None,
            index_commit: String::new(),
            created_ms: created,
            trigger,
            parents: parents.clone(),
            summary: summary.clone(),
        };
        let commit = snapshot::write_checkpoint(git, &state, &mut meta)?;
        let row = CheckpointRow {
            project_id: p.id.clone(),
            id: id.clone(),
            device_id: dev.id.clone(),
            device_name: dev.name.clone(),
            refname: checkpoint::ref_name(&dev.id, &id),
            commit_oid: commit,
            tree_oid: state.tree.clone(),
            head_oid: state.head.clone(),
            branch: state.branch.clone(),
            index_commit: meta.index_commit.clone(),
            created_ms: created,
            trigger: trigger.as_str().into(),
            parents,
            summary,
            files_changed: Some(files_changed as i64),
            origin: "local".into(),
            push_state: "pending".into(),
            applied_ms: None,
            pruned: false,
        };
        {
            let db = self.db();
            db.insert_checkpoint(&row)?;
            db.set_lineage_head(&p.id, Some(&id))?;
            db.set_project_time(&p.id, "last_checkpoint_ms", created)?;
            db.touch_device(&dev.id, &dev.name, created)?;
            if p.auto_push {
                db.enqueue(&p.id, "push", "")?;
            }
        }
        self.log(
            Some(&p.id),
            "info",
            "checkpoint",
            &format!(
                "Checkpoint {} criado ({}, {}, branch {})",
                id,
                trigger.as_str(),
                row.summary,
                state.branch.as_deref().map(checkpoint::short_branch).unwrap_or("destacada")
            ),
        );
        Ok((Some(row), state))
    }

    pub fn history(&self, project_id: &str, limit: i64) -> Result<Vec<CheckpointRow>> {
        self.db().checkpoints(project_id, false, limit.clamp(1, 5000))
    }

    // ---- network --------------------------------------------------------

    fn auth_for_url(&self, url: &str) -> NetAuth {
        let mut auth = NetAuth::default();
        if credentials::is_github_https(url)
            && let (Some(exe), Ok(Some(_))) = (&self.cfg.askpass_exe, self.cfg.secrets.get(credentials::GITHUB_ACCOUNT)) {
                auth.askpass = Some(OsString::from(exe.as_os_str()));
            }
        auth
    }

    fn auth_for(&self, p: &Project, git: &Git) -> Result<NetAuth> {
        let url = git
            .config_get(&format!("remote.{}.url", p.remote))?
            .ok_or_else(|| Error::Invalid(format!("remoto '{}' não configurado", p.remote)))?;
        Ok(self.auth_for_url(&url))
    }

    /// Scan and push every pending checkpoint of this device.
    pub fn push_pending(&self, project_id: &str) -> Result<PushReport> {
        let lock = lock_for(&self.net_locks, project_id);
        let _g = guard(&lock);
        let p = self.db().project(project_id)?;
        let git = self.open_git(&p)?;
        let auth = self.auth_for(&p, &git)?;
        let dev = self.device();
        let mut report = PushReport::default();

        let mut candidates = self.db().checkpoints_by_state(project_id, "pending")?;
        candidates.extend(self.db().checkpoints_by_state(project_id, "needs_review")?);
        candidates.retain(|c| c.device_id == dev.id);
        candidates.sort_by(|a, b| a.id.cmp(&b.id));
        if candidates.is_empty() {
            return Ok(report);
        }

        let mut base = match self.db().last_pushed_own(project_id, &dev.id)? {
            Some(c) if git.has_object(&c.tree_oid) => c.tree_oid,
            _ => match git.resolve("@{upstream}^{tree}").ok().flatten() {
                Some(t) => t,
                None => git.empty_tree()?,
            },
        };
        let mut to_push = Vec::new();
        for c in &candidates {
            let findings = secrets::scan_trees(&git, &base, &c.tree_oid)?;
            self.db().replace_findings(project_id, &c.id, &findings)?;
            let blocking = findings.iter().any(|f| f.severity == Severity::Blocking);
            let mut unapproved = 0;
            for f in findings.iter().filter(|f| f.severity == Severity::Warning) {
                if !self.db().is_allowed(project_id, &f.fingerprint)? {
                    unapproved += 1;
                }
            }
            if blocking {
                self.db().set_push_state(project_id, &c.id, "quarantined")?;
                report.quarantined.push(c.id.clone());
                self.log(
                    Some(project_id),
                    "error",
                    "secret_blocked",
                    &format!("Checkpoint {} contém credenciais e NÃO será enviado. Remova-as ou adicione ao .gitignore.", c.id),
                );
            } else if unapproved > 0 {
                if c.push_state != "needs_review" {
                    self.db().set_push_state(project_id, &c.id, "needs_review")?;
                    self.log(
                        Some(project_id),
                        "warn",
                        "secret_review",
                        &format!("Checkpoint {}: {unapproved} possível(is) segredo(s) aguardando sua confirmação", c.id),
                    );
                }
                report.needs_review.push(c.id.clone());
            } else {
                to_push.push(c.clone());
                base = c.tree_oid.clone();
            }
        }
        if to_push.is_empty() {
            return Ok(report);
        }
        let refs: Vec<String> = to_push.iter().map(|c| c.refname.clone()).collect();
        transport::push_refs(&git, &p.remote, &refs, &auth)?;
        {
            let db = self.db();
            for c in &to_push {
                db.set_push_state(project_id, &c.id, "pushed")?;
            }
            db.set_project_time(project_id, "last_push_ms", now_ms())?;
            db.set_project_error(project_id, None)?;
        }
        report.pushed = refs;
        self.log(Some(project_id), "info", "push", &format!("{} checkpoint(s) enviado(s) para {}", to_push.len(), p.remote));
        Ok(report)
    }

    /// Fetch checkpoint refs and import their metadata.
    pub fn fetch(&self, project_id: &str) -> Result<FetchReport> {
        let lock = lock_for(&self.net_locks, project_id);
        let _g = guard(&lock);
        let p = self.db().project(project_id)?;
        let git = self.open_git(&p)?;
        let auth = self.auth_for(&p, &git)?;
        transport::fetch_checkpoints(&git, &p.remote, &auth)?;
        let imported = self.import_refs(&p, &git)?;
        self.db().set_project_time(project_id, "last_fetch_ms", now_ms())?;
        let incoming = self.incoming(project_id)?.len();
        if imported > 0 {
            self.log(Some(project_id), "info", "fetch", &format!("{imported} checkpoint(s) recebido(s); {incoming} atualização(ões) disponível(is)"));
        }
        Ok(FetchReport { imported, incoming })
    }

    fn import_refs(&self, p: &Project, git: &Git) -> Result<usize> {
        let local_dev = self.device().id;
        let mut n = 0;
        for rc in snapshot::list_local(git)? {
            let existing = self.db().checkpoint_opt(&p.id, &rc.meta.id)?;
            if let Some(e) = existing {
                if e.pruned {
                    // Re-fetched after local retention: drop the ref again.
                    let _ = git.delete_ref(&rc.refname, &rc.commit);
                }
                continue;
            }
            let own = rc.meta.device_id == local_dev;
            let row = CheckpointRow {
                project_id: p.id.clone(),
                id: rc.meta.id.clone(),
                device_id: rc.meta.device_id.clone(),
                device_name: rc.meta.device_name.clone(),
                refname: rc.refname.clone(),
                commit_oid: rc.commit.clone(),
                tree_oid: rc.tree.clone(),
                head_oid: rc.meta.head.clone(),
                branch: rc.meta.branch.clone(),
                index_commit: rc.meta.index_commit.clone(),
                created_ms: rc.meta.created_ms,
                trigger: rc.meta.trigger.as_str().into(),
                parents: rc.meta.parents.clone(),
                summary: rc.meta.summary.clone(),
                files_changed: None,
                origin: if own { "local".into() } else { "remote".into() },
                push_state: if own { "pushed".into() } else { "remote".into() },
                applied_ms: None,
                pruned: false,
            };
            let db = self.db();
            if db.insert_checkpoint(&row)? {
                n += 1;
                db.touch_device(&row.device_id, &row.device_name, row.created_ms)?;
            }
        }
        Ok(n)
    }

    // ---- lineage --------------------------------------------------------

    /// Ancestors of `id` in the lineage graph (including itself), BFS order.
    fn lineage_ancestors(&self, project_id: &str, id: &str) -> Result<Vec<String>> {
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        let mut q = VecDeque::from([id.to_string()]);
        while let Some(cur) = q.pop_front() {
            if !seen.insert(cur.clone()) || order.len() > 20_000 {
                continue;
            }
            order.push(cur.clone());
            if let Some(row) = self.db().checkpoint_opt(project_id, &cur)? {
                q.extend(row.parents);
            }
        }
        Ok(order)
    }

    /// Checkpoints from other devices that this device has not incorporated.
    pub fn incoming(&self, project_id: &str) -> Result<Vec<Incoming>> {
        let p = self.db().project(project_id)?;
        let me = self.device().id;
        let all = self.db().checkpoints(project_id, false, 5000)?;
        let mine: HashSet<String> = match &p.lineage_head {
            Some(h) => self.lineage_ancestors(project_id, h)?.into_iter().collect(),
            None => HashSet::new(),
        };
        let head_tree = p.lineage_head.as_ref().and_then(|h| all.iter().find(|c| &c.id == h)).map(|c| c.tree_oid.clone());
        let mut latest: HashMap<String, CheckpointRow> = HashMap::new();
        for c in all.into_iter().filter(|c| c.device_id != me) {
            match latest.get(&c.device_id) {
                Some(x) if x.id >= c.id => {}
                _ => {
                    latest.insert(c.device_id.clone(), c);
                }
            }
        }
        let git = self.open_git(&p).ok();
        let cur_branch = git.as_ref().and_then(|g| g.head_branch().ok().flatten());
        let mut v: Vec<Incoming> = latest
            .into_values()
            .filter(|c| c.applied_ms.is_none() && !mine.contains(&c.id) && head_tree.as_deref() != Some(c.tree_oid.as_str()))
            .map(|c| Incoming { same_branch: c.branch == cur_branch, checkpoint: c })
            .collect();
        v.sort_by(|a, b| b.checkpoint.id.cmp(&a.checkpoint.id));
        Ok(v)
    }

    fn base_tree(&self, p: &Project, git: &Git, ours: &WorkState, theirs: &CheckpointRow) -> Result<String> {
        if let Some(h) = &p.lineage_head {
            let mine: HashSet<String> = self.lineage_ancestors(&p.id, h)?.into_iter().collect();
            for anc in self.lineage_ancestors(&p.id, &theirs.id)? {
                if mine.contains(&anc)
                    && let Some(row) = self.db().checkpoint_opt(&p.id, &anc)?
                        && git.has_object(&row.tree_oid) {
                            return Ok(row.tree_oid);
                        }
            }
        }
        if let (Some(a), Some(b)) = (&ours.head, &theirs.head_oid)
            && let Some(mb) = git.merge_base(a, b)? {
                return git.tree_of(&mb);
            }
        git.empty_tree()
    }

    fn load_theirs(&self, git: &Git, row: &CheckpointRow) -> Result<RefCheckpoint> {
        let rc = snapshot::read_checkpoint(git, &row.refname)?;
        if rc.commit != row.commit_oid {
            return Err(Error::Conflict("a referência do checkpoint mudou inesperadamente".into()));
        }
        Ok(rc)
    }

    pub fn ide_check(&self, project_id: &str) -> IdeCheck {
        let mut c = self.cfg.ide_probe.check();
        let quiet = self.settings().map(|s| s.quiet_secs).unwrap_or(120) * 1000;
        if let Some(t) = self.last_fs_event(project_id)
            && now_ms() - t < quiet {
                c.safe = false;
                c.reason = format!("{}; arquivos modificados há menos de {}s", c.reason, quiet / 1000);
            }
        c
    }

    pub fn plan_apply(&self, project_id: &str, checkpoint_id: &str, opts: &ApplyOptions) -> Result<ApplyPreview> {
        let lock = lock_for(&self.work_locks, project_id);
        let _g = guard(&lock);
        let p = self.db().project(project_id)?;
        let git = self.open_git(&p)?;
        let row = self.db().checkpoint(project_id, checkpoint_id)?;
        let theirs = self.load_theirs(&git, &row)?;
        let ours = snapshot::capture(&git)?;
        let base = self.base_tree(&p, &git, &ours, &row)?;
        let plan = apply::plan(&git, &ours, &theirs, &base, opts)?;
        Ok(ApplyPreview { auto_eligible: plan.auto_eligible(), plan, checkpoint: row, ide: self.ide_check(project_id) })
    }

    /// Apply an incoming checkpoint. With `auto`, only proceeds when the plan
    /// is a clean fast-forward and the IDE check passes.
    pub fn apply(&self, project_id: &str, checkpoint_id: &str, opts: &ApplyOptions, auto: bool) -> Result<ApplyOutcome> {
        let lock = lock_for(&self.work_locks, project_id);
        let _g = guard(&lock);
        let p = self.db().project(project_id)?;
        if !self.db().conflicts(Some(project_id), true)?.is_empty() {
            return Err(Error::Conflict("há um conflito pendente neste projeto; resolva ou descarte primeiro".into()));
        }
        let git = self.open_git(&p)?;
        let row = self.db().checkpoint(project_id, checkpoint_id)?;
        if row.device_id == self.device().id {
            return Err(Error::Invalid("use a recuperação para checkpoints deste dispositivo".into()));
        }
        let theirs = self.load_theirs(&git, &row)?;

        if auto {
            let ide = self.ide_check(project_id);
            if !ide.safe {
                return Ok(ApplyOutcome::NeedsManual { reason: ide.reason });
            }
            // Dry plan first so an ineligible auto-apply leaves no trace.
            let ours = snapshot::capture(&git)?;
            let base = self.base_tree(&p, &git, &ours, &row)?;
            let plan = apply::plan(&git, &ours, &theirs, &base, opts)?;
            if !plan.auto_eligible() {
                let mut reasons = plan.blockers.clone();
                if plan.local_changes {
                    reasons.push("há alterações locais desde a última sincronização".into());
                }
                if !plan.conflicts.is_empty() {
                    reasons.push(format!("{} conflito(s)", plan.conflicts.len()));
                }
                if !plan.skipped.is_empty() {
                    reasons.push(format!("{} arquivo(s) não aplicável(is)", plan.skipped.len()));
                }
                return Ok(ApplyOutcome::NeedsManual { reason: reasons.join("; ") });
            }
        }

        // Safety checkpoint of the current state (or the lineage head if
        // nothing changed since it).
        let (created, ours) = self.checkpoint_locked(&p, &git, Trigger::PreApply, &[])?;
        let p = self.db().project(project_id)?;
        let safety = match created {
            Some(c) => c.id,
            None => p.lineage_head.clone().ok_or_else(|| Error::Other("sem checkpoint de segurança".into()))?,
        };
        let base = self.base_tree(&p, &git, &ours, &row)?;
        let plan = apply::plan(&git, &ours, &theirs, &base, opts)?;
        if auto && !plan.auto_eligible() {
            return Ok(ApplyOutcome::NeedsManual { reason: "o estado local mudou durante a verificação".into() });
        }
        if !plan.blockers.is_empty() {
            return Ok(ApplyOutcome::Blocked { plan });
        }
        if !plan.conflicts.is_empty() {
            let id = uuid::Uuid::new_v4().to_string();
            let count = plan.conflicts.len();
            self.db().insert_conflict(&ConflictRow {
                id: id.clone(),
                project_id: project_id.into(),
                checkpoint_id: checkpoint_id.into(),
                safety_checkpoint: safety.clone(),
                plan_json: serde_json::to_string(&plan)?,
                options_json: serde_json::to_string(opts)?,
                state: "open".into(),
                created_ms: now_ms(),
                resolved_ms: None,
            })?;
            self.log(
                Some(project_id),
                "warn",
                "conflict",
                &format!("{count} conflito(s) ao receber {} de {}; nada foi alterado até você decidir", row.id, row.device_name),
            );
            return Ok(ApplyOutcome::Conflicts { conflict_id: id, count });
        }
        if plan.is_noop() {
            self.finish_apply(&p, &git, &row, &safety)?;
            return Ok(ApplyOutcome::NothingToDo { safety_checkpoint: safety });
        }
        let op = uuid::Uuid::new_v4().to_string();
        self.db().op_start(&op, project_id, "apply", Some(&safety), &format!("checkpoint {}", row.id))?;
        let preserve_dir = git.private_dir()?.join("preserved").join(&op);
        let result = match apply::execute(&git, &plan, &plan.changes, &[], &preserve_dir) {
            Ok(r) => r,
            Err(e) => {
                self.db().op_finish(&op, "failed", Some(&e.to_string()))?;
                self.log(Some(project_id), "error", "apply_failed", &format!("Falha ao aplicar {}: {e}. Checkpoint de segurança: {safety}", row.id));
                return Err(e);
            }
        };
        self.db().op_finish(&op, "done", None)?;
        let lineage = self.finish_apply(&p, &git, &row, &safety)?;
        self.log(
            Some(project_id),
            "info",
            if auto { "auto_apply" } else { "apply" },
            &format!(
                "Atualização de {} aplicada: {} escrito(s), {} removido(s){}",
                row.device_name,
                result.written.len(),
                result.deleted.len(),
                if result.integrity_failures.is_empty() { String::new() } else { format!(", {} falha(s) de integridade", result.integrity_failures.len()) }
            ),
        );
        Ok(ApplyOutcome::Applied { result, safety_checkpoint: safety, lineage_head: lineage })
    }

    /// Update lineage after an apply: adopt `theirs` when the tree now equals
    /// it exactly, otherwise record a merge checkpoint.
    fn finish_apply(&self, p: &Project, git: &Git, theirs: &CheckpointRow, safety: &str) -> Result<String> {
        let now = snapshot::capture(git)?;
        let exact = now.tree == theirs.tree_oid
            && now.head == theirs.head_oid
            && now.branch == theirs.branch
            && git.tree_of(&theirs.index_commit).ok().as_deref() == Some(now.index_tree.as_str());
        self.db().mark_applied(&p.id, &theirs.id)?;
        if exact {
            self.db().set_lineage_head(&p.id, Some(&theirs.id))?;
            return Ok(theirs.id.clone());
        }
        let parents = vec![safety.to_string(), theirs.id.clone()];
        // checkpoint_locked prepends the current lineage head (== safety).
        let (row, _) = self.checkpoint_locked(p, git, Trigger::Merge, &parents)?;
        Ok(row.map(|r| r.id).unwrap_or_else(|| safety.to_string()))
    }

    // ---- conflicts ------------------------------------------------------

    pub fn conflicts(&self, project_id: Option<&str>) -> Result<Vec<ConflictRow>> {
        self.db().conflicts(project_id, true)
    }

    fn view_tree_entry(&self, git: &Git, e: Option<&crate::git::TreeEntry>) -> Result<FileView> {
        let Some(e) = e else {
            return Ok(FileView { exists: false, binary: false, size: 0, oid: None, text: None, truncated: false });
        };
        if e.mode == apply::MODE_GITLINK {
            return Ok(FileView { exists: true, binary: true, size: 0, oid: Some(e.oid.clone()), text: None, truncated: false });
        }
        let size = git.object_size(&e.oid)?;
        let data = if size <= VIEW_LIMIT { git.blob_raw(&e.oid)? } else { Vec::new() };
        Ok(bytes_view(&data, size, Some(e.oid.clone())))
    }

    pub fn conflict_detail(&self, conflict_id: &str) -> Result<ConflictDetail> {
        let c = self.db().conflict(conflict_id)?;
        let p = self.db().project(&c.project_id)?;
        let git = self.open_git(&p)?;
        let plan: ApplyPlan = serde_json::from_str(&c.plan_json)?;
        let theirs = self.db().checkpoint(&c.project_id, &c.checkpoint_id)?;
        let mut files = Vec::new();
        for e in &plan.conflicts {
            let ours = if e.reason == "local_untracked" {
                disk_view(&git.workdir, &e.path)?
            } else {
                self.view_tree_entry(&git, e.ours.as_ref())?
            };
            files.push(ConflictFileView {
                path: e.path.clone(),
                reason: e.reason.clone(),
                ours,
                theirs: self.view_tree_entry(&git, e.theirs.as_ref())?,
                base: self.view_tree_entry(&git, e.base.as_ref())?,
            });
        }
        Ok(ConflictDetail { conflict: c, plan, theirs_device: theirs.device_name, files })
    }

    pub fn resolve_conflict(&self, conflict_id: &str, resolutions: &HashMap<String, Resolution>) -> Result<ApplyOutcome> {
        let c = self.db().conflict(conflict_id)?;
        if c.state != "open" {
            return Err(Error::Invalid("conflito já encerrado".into()));
        }
        let lock = lock_for(&self.work_locks, &c.project_id);
        let _g = guard(&lock);
        let p = self.db().project(&c.project_id)?;
        let git = self.open_git(&p)?;
        let plan: ApplyPlan = serde_json::from_str(&c.plan_json)?;
        let theirs = self.db().checkpoint(&c.project_id, &c.checkpoint_id)?;
        let now = snapshot::capture(&git)?;
        if now.tree != plan.ours_tree {
            return Err(Error::Conflict(
                "os arquivos locais mudaram desde a detecção do conflito; descarte este conflito e aplique novamente".into(),
            ));
        }
        let (changes, preserve) = apply::resolve(&plan, resolutions, &theirs.device_name)?;
        let op = uuid::Uuid::new_v4().to_string();
        self.db().op_start(&op, &p.id, "resolve_conflict", Some(&c.safety_checkpoint), conflict_id)?;
        let preserve_dir = git.private_dir()?.join("preserved").join(conflict_id);
        let result = match apply::execute(&git, &plan, &changes, &preserve, &preserve_dir) {
            Ok(r) => r,
            Err(e) => {
                self.db().op_finish(&op, "failed", Some(&e.to_string()))?;
                return Err(e);
            }
        };
        self.db().op_finish(&op, "done", None)?;
        self.db().set_conflict_state(conflict_id, "resolved")?;
        let lineage = self.finish_apply(&p, &git, &theirs, &c.safety_checkpoint)?;
        self.log(
            Some(&p.id),
            "info",
            "conflict_resolved",
            &format!(
                "Conflito resolvido ({} arquivo(s)); versões não escolhidas preservadas em {}",
                plan.conflicts.len(),
                result.preserved_dir.as_deref().unwrap_or("checkpoints")
            ),
        );
        Ok(ApplyOutcome::Applied { result, safety_checkpoint: c.safety_checkpoint, lineage_head: lineage })
    }

    pub fn abort_conflict(&self, conflict_id: &str) -> Result<()> {
        let c = self.db().conflict(conflict_id)?;
        self.db().set_conflict_state(conflict_id, "aborted")?;
        self.log(Some(&c.project_id), "info", "conflict_aborted", "Conflito descartado; nenhum arquivo foi alterado");
        Ok(())
    }

    // ---- compare / view -------------------------------------------------

    fn side_tree(&self, p: &Project, git: &Git, side: &Side) -> Result<Option<String>> {
        Ok(match side {
            Side::Checkpoint { id } => Some(self.db().checkpoint(&p.id, id)?.tree_oid),
            Side::Head => match git.head_commit()? {
                Some(h) => Some(git.tree_of(&h)?),
                None => Some(git.empty_tree()?),
            },
            Side::Current => None,
        })
    }

    /// Files that differ between two sides (renames detected for display).
    pub fn compare(&self, project_id: &str, from: &Side, to: &Side) -> Result<Vec<crate::git::DiffEntry>> {
        let p = self.db().project(project_id)?;
        let git = self.open_git(&p)?;
        let resolve = |s: &Side| -> Result<String> {
            match self.side_tree(&p, &git, s)? {
                Some(t) => Ok(t),
                None => Ok(snapshot::capture(&git)?.tree),
            }
        };
        let a = resolve(from)?;
        let b = resolve(to)?;
        git.diff_trees_display(&a, &b)
    }

    pub fn file_view(&self, project_id: &str, side: &Side, path: &str) -> Result<FileView> {
        crate::paths::validate_rel_path(path)?;
        let p = self.db().project(project_id)?;
        let git = self.open_git(&p)?;
        match self.side_tree(&p, &git, side)? {
            None => disk_view(&git.workdir, path),
            Some(tree) => {
                let entries = git.ls_tree(&tree)?;
                self.view_tree_entry(&git, entries.get(path))
            }
        }
    }

    // ---- recovery -------------------------------------------------------

    pub fn restore_files(&self, project_id: &str, checkpoint_id: &str, rel_paths: &[String]) -> Result<ApplyOutcome> {
        if rel_paths.is_empty() {
            return Err(Error::Invalid("nenhum arquivo selecionado".into()));
        }
        let lock = lock_for(&self.work_locks, project_id);
        let _g = guard(&lock);
        let p = self.db().project(project_id)?;
        let git = self.open_git(&p)?;
        let row = self.db().checkpoint(project_id, checkpoint_id)?;
        if !git.has_object(&row.tree_oid) {
            return Err(Error::NotFound("objetos do checkpoint não estão disponíveis localmente".into()));
        }
        let (created, current) = self.checkpoint_locked(&p, &git, Trigger::PreRestore, &[])?;
        let safety = match created {
            Some(c) => c.id,
            None => self.db().project(project_id)?.lineage_head.unwrap_or_default(),
        };
        let plan = restore::plan_in_place(&git, &current, &row.tree_oid, rel_paths)?;
        let op = uuid::Uuid::new_v4().to_string();
        self.db().op_start(&op, project_id, "restore", Some(&safety), &format!("{} arquivo(s) de {}", rel_paths.len(), row.id))?;
        let preserve_dir = git.private_dir()?.join("preserved").join(&op);
        let result = match restore::execute_in_place(&git, &plan, &preserve_dir) {
            Ok(r) => r,
            Err(e) => {
                self.db().op_finish(&op, "failed", Some(&e.to_string()))?;
                return Err(e);
            }
        };
        self.db().op_finish(&op, "done", None)?;
        self.log(
            Some(project_id),
            "info",
            "restore",
            &format!(
                "{} arquivo(s) restaurado(s) de {} (integridade: {} falha(s)); estado anterior em {}",
                result.written.len() + result.deleted.len(),
                row.id,
                result.integrity_failures.len(),
                safety
            ),
        );
        Ok(ApplyOutcome::Applied { result, safety_checkpoint: safety.clone(), lineage_head: safety })
    }

    pub fn restore_to_dir(&self, project_id: &str, checkpoint_id: &str, target: &str) -> Result<RestoreReport> {
        let p = self.db().project(project_id)?;
        let git = self.open_git(&p)?;
        let row = self.db().checkpoint(project_id, checkpoint_id)?;
        let rep = restore::restore_to_dir(&git, &row.tree_oid, Path::new(target))?;
        self.log(
            Some(project_id),
            if rep.integrity_failures.is_empty() { "info" } else { "error" },
            "restore_dir",
            &format!(
                "Checkpoint {} recuperado em {}: {} arquivo(s), {} verificado(s), {} falha(s)",
                row.id,
                rep.target,
                rep.files_written,
                rep.verified,
                rep.integrity_failures.len()
            ),
        );
        Ok(rep)
    }

    pub fn acknowledge_operation(&self, op_id: &str) -> Result<()> {
        self.db().ack_operation(op_id)
    }

    // ---- secrets --------------------------------------------------------

    pub fn findings(&self, project_id: &str) -> Result<Vec<crate::db::FindingRow>> {
        let held: HashSet<String> = {
            let db = self.db();
            let mut s: HashSet<String> = db.checkpoints_by_state(project_id, "needs_review")?.into_iter().map(|c| c.id).collect();
            s.extend(db.checkpoints_by_state(project_id, "quarantined")?.into_iter().map(|c| c.id));
            s
        };
        Ok(self.db().findings(project_id, None)?.into_iter().filter(|f| held.contains(&f.checkpoint_id)).collect())
    }

    /// Confirm specific warning-level findings. Blocking ones cannot be approved.
    pub fn approve_findings(&self, project_id: &str, fingerprints: &[String]) -> Result<usize> {
        let all = self.db().findings(project_id, None)?;
        let mut n = 0;
        for fp in fingerprints {
            let Some(f) = all.iter().find(|f| &f.fingerprint == fp) else { continue };
            if Severity::parse(&f.severity) == Severity::Blocking {
                return Err(Error::Invalid(format!("{} ({}) é bloqueante e não pode ser liberado", f.path, f.rule)));
            }
            self.db().allow_fingerprint(project_id, fp)?;
            n += 1;
        }
        if n > 0 {
            self.db().enqueue(project_id, "push", "")?;
            self.db().queue_retry_now(Some(project_id))?;
            self.log(Some(project_id), "warn", "secret_approved", &format!("{n} alerta(s) de segredo confirmado(s) pelo usuário"));
        }
        Ok(n)
    }

    // ---- retention ------------------------------------------------------

    pub fn run_retention(&self, project_id: &str) -> Result<RetentionReport> {
        let lock = lock_for(&self.work_locks, project_id);
        let _g = guard(&lock);
        let settings = self.settings()?;
        let p = self.db().project(project_id)?;
        let git = self.open_git(&p)?;
        let me = self.device().id;
        let rows = self.db().checkpoints(project_id, false, 100_000)?;

        let mut protected: HashSet<String> = HashSet::new();
        if let Some(h) = &p.lineage_head {
            protected.insert(h.clone());
            if let Some(r) = rows.iter().find(|r| &r.id == h) {
                protected.extend(r.parents.iter().cloned());
            }
        }
        for c in self.db().conflicts(Some(project_id), true)? {
            protected.insert(c.safety_checkpoint);
            protected.insert(c.checkpoint_id);
        }
        for op in self.db().operations(project_id, 200)? {
            if let Some(s) = op.safety_checkpoint
                && (op.state == "running" || op.state == "interrupted") {
                    protected.insert(s);
                }
        }
        if let Some(lp) = self.db().last_pushed_own(project_id, &me)? {
            protected.insert(lp.id);
        }
        let mut latest_branch: HashMap<(String, Option<String>), String> = HashMap::new();
        for r in &rows {
            let k = (r.device_id.clone(), r.branch.clone());
            let e = latest_branch.entry(k).or_insert_with(|| r.id.clone());
            if r.id > *e {
                *e = r.id.clone();
            }
        }
        protected.extend(latest_branch.into_values());
        for inc in self.incoming(project_id)? {
            protected.insert(inc.checkpoint.id);
        }

        let items: Vec<retention::Item> = rows
            .iter()
            .map(|r| retention::Item {
                id: r.id.clone(),
                device_id: r.device_id.clone(),
                created_ms: r.created_ms,
                protected: protected.contains(&r.id) || r.push_state == "pending",
            })
            .collect();
        let prune: HashSet<String> = retention::select_prunable(&items, now_ms(), &settings.retention).into_iter().collect();
        let mut rep = RetentionReport { kept: rows.len() - prune.len(), ..Default::default() };
        for r in rows.iter().filter(|r| prune.contains(&r.id)) {
            if r.device_id == me && r.push_state == "pushed" && settings.prune_remote {
                self.db().enqueue(project_id, "delete_remote", &r.refname)?;
                rep.queued_remote += 1;
            } else {
                if let Some(v) = git.ref_value(&r.refname)? {
                    git.delete_ref(&r.refname, &v)?;
                }
                self.db().mark_pruned(project_id, &r.id)?;
                rep.pruned_local += 1;
            }
        }
        if rep.pruned_local + rep.queued_remote > 0 {
            self.log(
                Some(project_id),
                "info",
                "retention",
                &format!("Retenção: {} removido(s) localmente, {} agendado(s) para remoção remota, {} mantido(s)", rep.pruned_local, rep.queued_remote, rep.kept),
            );
        }
        Ok(rep)
    }

    fn delete_remote_checkpoint(&self, project_id: &str, refname: &str) -> Result<()> {
        let lock = lock_for(&self.net_locks, project_id);
        let _g = guard(&lock);
        let p = self.db().project(project_id)?;
        let git = self.open_git(&p)?;
        let Some((dev, id)) = checkpoint::parse_ref(refname) else {
            return Err(Error::Invalid("ref inválida".into()));
        };
        if dev != self.device().id {
            return Err(Error::Invalid("só é permitido remover checkpoints deste dispositivo do remoto".into()));
        }
        let auth = self.auth_for(&p, &git)?;
        transport::delete_remote_refs(&git, &p.remote, &[refname.to_string()], &auth)?;
        if let Some(v) = git.ref_value(refname)? {
            git.delete_ref(refname, &v)?;
        }
        self.db().mark_pruned(project_id, &id)?;
        Ok(())
    }

    fn push_branch(&self, project_id: &str, branch: &str) -> Result<()> {
        if !branch.starts_with("refs/heads/") {
            return Err(Error::Invalid("branch inválida".into()));
        }
        let lock = lock_for(&self.net_locks, project_id);
        let _g = guard(&lock);
        let p = self.db().project(project_id)?;
        let git = self.open_git(&p)?;
        let auth = self.auth_for(&p, &git)?;
        transport::push_branch(&git, &p.remote, branch, &auth)?;
        self.log(Some(project_id), "info", "push_branch", &format!("Branch {} enviada", checkpoint::short_branch(branch)));
        Ok(())
    }

    // ---- queue ----------------------------------------------------------

    /// Process due queue items. Returns how many succeeded.
    pub fn process_queue(&self) -> Result<usize> {
        let due = self.db().due_queue(now_ms())?;
        let mut ok = 0;
        for item in due {
            let found = self.db().project(&item.project_id);
            let p = match found {
                Ok(p) => p,
                Err(_) => {
                    self.db().queue_done(item.id)?;
                    continue;
                }
            };
            if p.paused {
                continue;
            }
            let res = match item.kind.as_str() {
                "push" if !p.auto_push => Ok(()),
                "push" => self.push_pending(&item.project_id).map(|_| ()),
                "delete_remote" => self.delete_remote_checkpoint(&item.project_id, &item.payload),
                "push_branch" => self.push_branch(&item.project_id, &item.payload),
                _ => Ok(()),
            };
            if let (Err(e), "push_branch") = (&res, item.kind.as_str())
                && e.failure_kind() == FailureKind::Rejected {
                    // Never forced: the remote branch moved, the user must integrate it.
                    self.db().queue_done(item.id)?;
                    self.log(
                        Some(&p.id),
                        "error",
                        "push_branch_rejected",
                        &format!("O remoto recusou a branch {} (divergiu). Integre as mudanças manualmente; nada foi forçado.", item.payload),
                    );
                    continue;
                }
            match res {
                Ok(()) => {
                    self.db().queue_done(item.id)?;
                    if item.attempts > 0 {
                        self.log(Some(&p.id), "info", "queue_recovered", "Conexão restabelecida; operações pendentes concluídas");
                    }
                    ok += 1;
                }
                Err(e) => {
                    let kind = match &e {
                        Error::Invalid(_) | Error::NotFound(_) => FailureKind::Other,
                        _ => e.failure_kind(),
                    };
                    self.db().queue_failed(item.id, &e.to_string(), kind)?;
                    self.db().set_project_error(&p.id, Some(&e.to_string()))?;
                    if item.attempts == 0 {
                        let msg = match kind {
                            FailureKind::Offline => format!("Sem conexão com o remoto; envio ficará na fila e será repetido ({e})"),
                            FailureKind::Auth => format!("Credenciais recusadas pelo remoto ({e})"),
                            _ => format!("Falha ao sincronizar: {e}"),
                        };
                        self.log(Some(&p.id), "warn", "queue_failed", &msg);
                    }
                }
            }
        }
        Ok(ok)
    }

    pub fn queue(&self) -> Result<Vec<crate::db::QueueItem>> {
        self.db().queue(None)
    }

    // ---- sync orchestration ----------------------------------------------

    /// Push what is pending, fetch, and auto-apply when allowed and safe.
    pub fn sync_project(&self, project_id: &str, allow_auto_apply: bool) -> Result<FetchReport> {
        let p = self.db().project(project_id)?;
        self.db().queue_retry_now(Some(project_id))?;
        self.process_queue()?;
        let rep = match self.fetch(project_id) {
            Ok(r) => r,
            Err(e) => {
                self.db().set_project_error(project_id, Some(&e.to_string()))?;
                return Err(e);
            }
        };
        if allow_auto_apply && p.trusted && !p.paused {
            self.try_auto_apply(project_id)?;
        }
        Ok(rep)
    }

    pub fn try_auto_apply(&self, project_id: &str) -> Result<Option<ApplyOutcome>> {
        let Some(next) = self.incoming(project_id)?.into_iter().next() else { return Ok(None) };
        let out = self.apply(project_id, &next.checkpoint.id, &ApplyOptions::default(), true)?;
        if let ApplyOutcome::NeedsManual { reason } = &out {
            let mut n = self.notified.lock().unwrap();
            if n.get(project_id) != Some(&next.checkpoint.id) {
                n.insert(project_id.to_string(), next.checkpoint.id.clone());
                drop(n);
                self.log(
                    Some(project_id),
                    "warn",
                    "update_needs_manual",
                    &format!("Atualização de {} disponível, aplicação manual necessária: {reason}", next.checkpoint.device_name),
                );
            }
        }
        Ok(Some(out))
    }

    /// Checkpoint and push every active project, waiting for the result.
    pub fn sync_and_shutdown(&self) -> Result<Vec<ShutdownReport>> {
        let mut out = Vec::new();
        for p in self.projects()? {
            if p.paused {
                continue;
            }
            let mut r = ShutdownReport { project_id: p.id.clone(), name: p.name.clone(), checkpoint: None, pushed: 0, held: 0, error: None };
            match self.checkpoint_now(&p.id, Trigger::Shutdown) {
                Ok(c) => r.checkpoint = c.map(|c| c.id),
                Err(e) => r.error = Some(e.to_string()),
            }
            if p.auto_push {
                match self.push_pending(&p.id) {
                    Ok(pr) => {
                        r.pushed = pr.pushed.len();
                        r.held = pr.needs_review.len() + pr.quarantined.len();
                        if r.error.is_none() {
                            // Bind first: the DB guard must be released before
                            // queue_done locks it again.
                            let queued = self.db().queue(Some(&p.id)).unwrap_or_default();
                            for i in queued.iter().filter(|i| i.kind == "push") {
                                let _ = self.db().queue_done(i.id);
                            }
                        }
                    }
                    Err(e) => r.error = Some(e.to_string()),
                }
            }
            out.push(r);
        }
        self.log(None, "info", "sync_and_quit", &format!("Sincronizar e encerrar: {} projeto(s) processado(s)", out.len()));
        Ok(out)
    }

    // ---- GitHub ---------------------------------------------------------

    pub fn github_token(&self) -> Result<Option<String>> {
        self.cfg.secrets.get(credentials::GITHUB_ACCOUNT)
    }

    pub fn github_set_token(&self, token: &str) -> Result<crate::github::GithubUser> {
        let token = token.trim();
        if token.is_empty() || token.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(Error::Invalid("token inválido".into()));
        }
        let user = crate::github::user(token)?;
        self.cfg.secrets.set(credentials::GITHUB_ACCOUNT, token)?;
        self.db().set_setting("github_login", &user.login)?;
        self.log(None, "info", "github_connected", &format!("Conta GitHub conectada: {}", user.login));
        Ok(user)
    }

    pub fn github_logout(&self) -> Result<()> {
        self.cfg.secrets.delete(credentials::GITHUB_ACCOUNT)?;
        self.db().set_setting("github_login", "")?;
        self.log(None, "info", "github_disconnected", "Conta GitHub desconectada (token removido do cofre)");
        Ok(())
    }

    pub fn github_login(&self) -> Result<Option<String>> {
        Ok(self.db().setting("github_login")?.filter(|s| !s.is_empty()))
    }

    pub fn askpass_available(&self) -> bool {
        self.cfg.askpass_exe.is_some()
    }
}

fn bytes_view(data: &[u8], size: u64, oid: Option<String>) -> FileView {
    let binary = data.iter().take(8000).any(|b| *b == 0) || std::str::from_utf8(data).is_err() && size <= VIEW_LIMIT;
    let truncated = size > VIEW_LIMIT;
    let text = (!binary && !truncated).then(|| String::from_utf8_lossy(data).into_owned());
    FileView { exists: true, binary, size, oid, text, truncated }
}

fn disk_view(root: &Path, rel: &str) -> Result<FileView> {
    let path = crate::paths::join_rel(root, rel)?;
    match std::fs::symlink_metadata(&path) {
        Ok(md) if md.is_file() => {
            let size = md.len();
            let data = if size <= VIEW_LIMIT { std::fs::read(&path)? } else { Vec::new() };
            Ok(bytes_view(&data, size, None))
        }
        Ok(md) if md.file_type().is_symlink() => {
            let t = std::fs::read_link(&path)?.display().to_string();
            Ok(FileView { exists: true, binary: false, size: t.len() as u64, oid: None, text: Some(t), truncated: false })
        }
        _ => Ok(FileView { exists: false, binary: false, size: 0, oid: None, text: None, truncated: false }),
    }
}
