//! SQLite persistence. One database per user profile, WAL mode, versioned
//! migrations. All access goes through [`Db`].

use crate::error::{Error, Result};
use crate::util::now_ms;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE device (
        id TEXT PRIMARY KEY, name TEXT NOT NULL, created_ms INTEGER NOT NULL
    );
    CREATE TABLE devices (
        id TEXT PRIMARY KEY, name TEXT NOT NULL, first_seen_ms INTEGER NOT NULL,
        last_seen_ms INTEGER NOT NULL
    );
    CREATE TABLE projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        path TEXT NOT NULL UNIQUE,
        remote TEXT NOT NULL DEFAULT 'origin',
        trusted INTEGER NOT NULL DEFAULT 0,
        paused INTEGER NOT NULL DEFAULT 0,
        interval_secs INTEGER NOT NULL DEFAULT 600,
        auto_push INTEGER NOT NULL DEFAULT 1,
        autocommit INTEGER NOT NULL DEFAULT 0,
        lineage_head TEXT,
        last_checkpoint_ms INTEGER,
        last_fetch_ms INTEGER,
        last_push_ms INTEGER,
        last_error TEXT,
        created_ms INTEGER NOT NULL
    );
    CREATE TABLE checkpoints (
        project_id TEXT NOT NULL,
        id TEXT NOT NULL,
        device_id TEXT NOT NULL,
        device_name TEXT NOT NULL,
        refname TEXT NOT NULL,
        commit_oid TEXT NOT NULL,
        tree_oid TEXT NOT NULL,
        head_oid TEXT,
        branch TEXT,
        index_commit TEXT NOT NULL,
        created_ms INTEGER NOT NULL,
        trigger TEXT NOT NULL,
        parents TEXT NOT NULL DEFAULT '',
        summary TEXT NOT NULL DEFAULT '',
        files_changed INTEGER,
        origin TEXT NOT NULL,             -- 'local' | 'remote'
        push_state TEXT NOT NULL,         -- 'pending' | 'pushed' | 'needs_review' | 'quarantined' | 'remote' | 'local_only'
        applied_ms INTEGER,
        pruned INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (project_id, id)
    );
    CREATE INDEX checkpoints_by_device ON checkpoints(project_id, device_id, created_ms);
    CREATE TABLE queue (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id TEXT NOT NULL,
        kind TEXT NOT NULL,               -- 'push' | 'delete_remote'
        payload TEXT NOT NULL DEFAULT '',
        attempts INTEGER NOT NULL DEFAULT 0,
        next_attempt_ms INTEGER NOT NULL,
        last_error TEXT,
        failure_kind TEXT,
        created_ms INTEGER NOT NULL,
        UNIQUE (project_id, kind, payload)
    );
    CREATE TABLE activity (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id TEXT,
        ts_ms INTEGER NOT NULL,
        level TEXT NOT NULL,
        kind TEXT NOT NULL,
        message TEXT NOT NULL
    );
    CREATE INDEX activity_by_ts ON activity(ts_ms);
    CREATE TABLE operations (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        state TEXT NOT NULL,              -- 'running' | 'done' | 'failed' | 'interrupted'
        safety_checkpoint TEXT,
        details TEXT NOT NULL DEFAULT '',
        started_ms INTEGER NOT NULL,
        finished_ms INTEGER
    );
    CREATE TABLE conflicts (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL,
        checkpoint_id TEXT NOT NULL,
        safety_checkpoint TEXT NOT NULL,
        plan_json TEXT NOT NULL,
        options_json TEXT NOT NULL,
        state TEXT NOT NULL,              -- 'open' | 'resolved' | 'aborted'
        created_ms INTEGER NOT NULL,
        resolved_ms INTEGER
    );
    CREATE TABLE secret_findings (
        project_id TEXT NOT NULL,
        checkpoint_id TEXT NOT NULL,
        fingerprint TEXT NOT NULL,
        path TEXT NOT NULL,
        line INTEGER,
        rule TEXT NOT NULL,
        severity TEXT NOT NULL,
        excerpt TEXT NOT NULL,
        PRIMARY KEY (project_id, checkpoint_id, fingerprint)
    );
    CREATE TABLE secret_allow (
        project_id TEXT NOT NULL,
        fingerprint TEXT NOT NULL,
        created_ms INTEGER NOT NULL,
        PRIMARY KEY (project_id, fingerprint)
    );
    CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    "#,
    // v2: checkpoint signatures (audit finding 3)
    r#"
    ALTER TABLE devices ADD COLUMN public_key TEXT;
    ALTER TABLE devices ADD COLUMN approved INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE checkpoints ADD COLUMN verified INTEGER NOT NULL DEFAULT 0;
    "#,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub remote: String,
    pub trusted: bool,
    pub paused: bool,
    pub interval_secs: i64,
    pub auto_push: bool,
    pub autocommit: bool,
    pub lineage_head: Option<String>,
    pub last_checkpoint_ms: Option<i64>,
    pub last_fetch_ms: Option<i64>,
    pub last_push_ms: Option<i64>,
    pub last_error: Option<String>,
    pub created_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRow {
    pub project_id: String,
    pub id: String,
    pub device_id: String,
    pub device_name: String,
    pub refname: String,
    pub commit_oid: String,
    pub tree_oid: String,
    pub head_oid: Option<String>,
    pub branch: Option<String>,
    pub index_commit: String,
    pub created_ms: i64,
    pub trigger: String,
    pub parents: Vec<String>,
    pub summary: String,
    pub files_changed: Option<i64>,
    pub origin: String,
    pub push_state: String,
    pub applied_ms: Option<i64>,
    pub pruned: bool,
    /// Signature valid and made with the key pinned for `device_id`.
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub is_local: bool,
    pub public_key: Option<String>,
    pub fingerprint: Option<String>,
    /// Approved by the user for automatic apply.
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: i64,
    pub project_id: String,
    pub kind: String,
    pub payload: String,
    pub attempts: i64,
    pub next_attempt_ms: i64,
    pub last_error: Option<String>,
    pub failure_kind: Option<String>,
    pub created_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub id: i64,
    pub project_id: Option<String>,
    pub ts_ms: i64,
    pub level: String,
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub state: String,
    pub safety_checkpoint: Option<String>,
    pub details: String,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictRow {
    pub id: String,
    pub project_id: String,
    pub checkpoint_id: String,
    pub safety_checkpoint: String,
    pub plan_json: String,
    pub options_json: String,
    pub state: String,
    pub created_ms: i64,
    pub resolved_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingRow {
    pub checkpoint_id: String,
    pub fingerprint: String,
    pub path: String,
    pub line: Option<i64>,
    pub rule: String,
    pub severity: String,
    pub excerpt: String,
    pub allowed: bool,
}

pub struct Db {
    conn: Connection,
}

fn project_from_row(r: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get("id")?,
        name: r.get("name")?,
        path: r.get("path")?,
        remote: r.get("remote")?,
        trusted: r.get("trusted")?,
        paused: r.get("paused")?,
        interval_secs: r.get("interval_secs")?,
        auto_push: r.get("auto_push")?,
        autocommit: r.get("autocommit")?,
        lineage_head: r.get("lineage_head")?,
        last_checkpoint_ms: r.get("last_checkpoint_ms")?,
        last_fetch_ms: r.get("last_fetch_ms")?,
        last_push_ms: r.get("last_push_ms")?,
        last_error: r.get("last_error")?,
        created_ms: r.get("created_ms")?,
    })
}

fn checkpoint_from_row(r: &Row) -> rusqlite::Result<CheckpointRow> {
    let parents: String = r.get("parents")?;
    Ok(CheckpointRow {
        project_id: r.get("project_id")?,
        id: r.get("id")?,
        device_id: r.get("device_id")?,
        device_name: r.get("device_name")?,
        refname: r.get("refname")?,
        commit_oid: r.get("commit_oid")?,
        tree_oid: r.get("tree_oid")?,
        head_oid: r.get("head_oid")?,
        branch: r.get("branch")?,
        index_commit: r.get("index_commit")?,
        created_ms: r.get("created_ms")?,
        trigger: r.get("trigger")?,
        parents: parents.split_whitespace().map(String::from).collect(),
        summary: r.get("summary")?,
        files_changed: r.get("files_changed")?,
        origin: r.get("origin")?,
        push_state: r.get("push_state")?,
        applied_ms: r.get("applied_ms")?,
        pruned: r.get("pruned")?,
        verified: r.get("verified")?,
    })
}

fn queue_from_row(r: &Row) -> rusqlite::Result<QueueItem> {
    Ok(QueueItem {
        id: r.get("id")?,
        project_id: r.get("project_id")?,
        kind: r.get("kind")?,
        payload: r.get("payload")?,
        attempts: r.get("attempts")?,
        next_attempt_ms: r.get("next_attempt_ms")?,
        last_error: r.get("last_error")?,
        failure_kind: r.get("failure_kind")?,
        created_ms: r.get("created_ms")?,
    })
}

fn op_from_row(r: &Row) -> rusqlite::Result<Operation> {
    Ok(Operation {
        id: r.get("id")?,
        project_id: r.get("project_id")?,
        kind: r.get("kind")?,
        state: r.get("state")?,
        safety_checkpoint: r.get("safety_checkpoint")?,
        details: r.get("details")?,
        started_ms: r.get("started_ms")?,
        finished_ms: r.get("finished_ms")?,
    })
}

fn conflict_from_row(r: &Row) -> rusqlite::Result<ConflictRow> {
    Ok(ConflictRow {
        id: r.get("id")?,
        project_id: r.get("project_id")?,
        checkpoint_id: r.get("checkpoint_id")?,
        safety_checkpoint: r.get("safety_checkpoint")?,
        plan_json: r.get("plan_json")?,
        options_json: r.get("options_json")?,
        state: r.get("state")?,
        created_ms: r.get("created_ms")?,
        resolved_ms: r.get("resolved_ms")?,
    })
}

/// Backoff for the retry queue: 15 s doubling up to 30 min (auth: up to 2 h).
pub fn backoff_ms(attempts: i64, auth: bool) -> i64 {
    let base: i64 = 15_000;
    let cap: i64 = if auth { 7_200_000 } else { 1_800_000 };
    let exp = base.saturating_mul(1i64 << attempts.clamp(0, 20));
    exp.min(cap)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub remote: Option<String>,
    pub trusted: Option<bool>,
    pub paused: Option<bool>,
    pub interval_secs: Option<i64>,
    pub auto_push: Option<bool>,
    pub autocommit: Option<bool>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Db> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db> {
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let v = i as i64 + 1;
            if v > version {
                let tx = self.conn.unchecked_transaction()?;
                tx.execute_batch(sql)?;
                tx.pragma_update(None, "user_version", v)?;
                tx.commit()?;
            }
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    // ---- local device -------------------------------------------------

    pub fn local_device(&self) -> Result<Option<(String, String)>> {
        Ok(self.conn.query_row("SELECT id, name FROM device LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
    }

    pub fn create_local_device(&self, id: &str, name: &str) -> Result<()> {
        self.conn.execute("INSERT INTO device (id, name, created_ms) VALUES (?1, ?2, ?3)", params![id, name, now_ms()])?;
        Ok(())
    }

    pub fn rename_local_device(&self, name: &str) -> Result<()> {
        self.conn.execute("UPDATE device SET name = ?1", params![name])?;
        Ok(())
    }

    pub fn touch_device(&self, id: &str, name: &str, seen_ms: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO devices (id, name, first_seen_ms, last_seen_ms) VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(id) DO UPDATE SET
               name = CASE WHEN excluded.last_seen_ms >= devices.last_seen_ms THEN excluded.name ELSE devices.name END,
               last_seen_ms = MAX(devices.last_seen_ms, excluded.last_seen_ms),
               first_seen_ms = MIN(devices.first_seen_ms, excluded.first_seen_ms)",
            params![id, name, seen_ms],
        )?;
        Ok(())
    }

    pub fn devices(&self, local_id: &str) -> Result<Vec<DeviceRow>> {
        let mut st = self
            .conn
            .prepare("SELECT id, name, first_seen_ms, last_seen_ms, public_key, approved FROM devices ORDER BY last_seen_ms DESC")?;
        let rows = st.query_map([], |r| {
            let id: String = r.get(0)?;
            let public_key: Option<String> = r.get(4)?;
            Ok(DeviceRow {
                is_local: id == local_id,
                id,
                name: r.get(1)?,
                first_seen_ms: r.get(2)?,
                last_seen_ms: r.get(3)?,
                fingerprint: public_key.as_deref().map(crate::signing::fingerprint),
                public_key,
                approved: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// (pinned public key, approved) for a device.
    pub fn device_trust(&self, id: &str) -> Result<(Option<String>, bool)> {
        Ok(self
            .conn
            .query_row("SELECT public_key, approved FROM devices WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .unwrap_or((None, false)))
    }

    /// Pin a key on first sight; never replaces an existing pin.
    pub fn pin_device_key(&self, id: &str, key: &str) -> Result<()> {
        self.conn.execute("UPDATE devices SET public_key = ?1 WHERE id = ?2 AND public_key IS NULL", params![key, id])?;
        Ok(())
    }

    pub fn set_device_approved(&self, id: &str, approved: bool) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE devices SET approved = ?1 WHERE id = ?2 AND (public_key IS NOT NULL OR ?1 = 0)",
            params![approved, id],
        )?;
        if n == 0 {
            return Err(Error::Invalid("dispositivo desconhecido ou sem chave de assinatura".into()));
        }
        Ok(())
    }

    // ---- projects -----------------------------------------------------

    pub fn insert_project(&self, p: &Project) -> Result<()> {
        self.conn.execute(
            "INSERT INTO projects (id, name, path, remote, trusted, paused, interval_secs, auto_push, autocommit, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![p.id, p.name, p.path, p.remote, p.trusted, p.paused, p.interval_secs, p.auto_push, p.autocommit, p.created_ms],
        )?;
        Ok(())
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut st = self.conn.prepare("SELECT * FROM projects ORDER BY name COLLATE NOCASE")?;
        let rows = st.query_map([], project_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn project(&self, id: &str) -> Result<Project> {
        self.conn
            .query_row("SELECT * FROM projects WHERE id = ?1", [id], project_from_row)
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("projeto {id}")))
    }

    pub fn project_by_path(&self, path: &str) -> Result<Option<Project>> {
        Ok(self.conn.query_row("SELECT * FROM projects WHERE path = ?1", [path], project_from_row).optional()?)
    }

    pub fn update_project(&self, id: &str, patch: &ProjectPatch) -> Result<Project> {
        let tx = self.conn.unchecked_transaction()?;
        if let Some(v) = &patch.name {
            tx.execute("UPDATE projects SET name = ?1 WHERE id = ?2", params![v, id])?;
        }
        if let Some(v) = &patch.remote {
            tx.execute("UPDATE projects SET remote = ?1 WHERE id = ?2", params![v, id])?;
        }
        if let Some(v) = patch.trusted {
            tx.execute("UPDATE projects SET trusted = ?1 WHERE id = ?2", params![v, id])?;
        }
        if let Some(v) = patch.paused {
            tx.execute("UPDATE projects SET paused = ?1 WHERE id = ?2", params![v, id])?;
        }
        if let Some(v) = patch.interval_secs {
            tx.execute("UPDATE projects SET interval_secs = ?1 WHERE id = ?2", params![v.clamp(60, 86_400), id])?;
        }
        if let Some(v) = patch.auto_push {
            tx.execute("UPDATE projects SET auto_push = ?1 WHERE id = ?2", params![v, id])?;
        }
        if let Some(v) = patch.autocommit {
            tx.execute("UPDATE projects SET autocommit = ?1 WHERE id = ?2", params![v, id])?;
        }
        tx.commit()?;
        self.project(id)
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for t in ["checkpoints", "queue", "operations", "conflicts", "secret_findings", "secret_allow"] {
            tx.execute(&format!("DELETE FROM {t} WHERE project_id = ?1"), [id])?;
        }
        tx.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_lineage_head(&self, project_id: &str, id: Option<&str>) -> Result<()> {
        self.conn.execute("UPDATE projects SET lineage_head = ?1 WHERE id = ?2", params![id, project_id])?;
        Ok(())
    }

    pub fn set_project_time(&self, project_id: &str, column: &str, ms: i64) -> Result<()> {
        let col = match column {
            "last_checkpoint_ms" | "last_fetch_ms" | "last_push_ms" => column,
            _ => return Err(Error::Invalid(column.into())),
        };
        self.conn.execute(&format!("UPDATE projects SET {col} = ?1 WHERE id = ?2"), params![ms, project_id])?;
        Ok(())
    }

    pub fn set_project_error(&self, project_id: &str, err: Option<&str>) -> Result<()> {
        self.conn.execute("UPDATE projects SET last_error = ?1 WHERE id = ?2", params![err, project_id])?;
        Ok(())
    }

    // ---- checkpoints --------------------------------------------------

    /// Insert if absent. Returns true when a new row was added.
    pub fn insert_checkpoint(&self, c: &CheckpointRow) -> Result<bool> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO checkpoints (project_id, id, device_id, device_name, refname, commit_oid, tree_oid, head_oid,
              branch, index_commit, created_ms, trigger, parents, summary, files_changed, origin, push_state, applied_ms, pruned, verified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            params![
                c.project_id, c.id, c.device_id, c.device_name, c.refname, c.commit_oid, c.tree_oid, c.head_oid, c.branch,
                c.index_commit, c.created_ms, c.trigger, c.parents.join(" "), c.summary, c.files_changed, c.origin,
                c.push_state, c.applied_ms, c.pruned, c.verified
            ],
        )?;
        Ok(n > 0)
    }

    pub fn checkpoint(&self, project_id: &str, id: &str) -> Result<CheckpointRow> {
        self.conn
            .query_row("SELECT * FROM checkpoints WHERE project_id = ?1 AND id = ?2", [project_id, id], checkpoint_from_row)
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("checkpoint {id}")))
    }

    pub fn checkpoint_opt(&self, project_id: &str, id: &str) -> Result<Option<CheckpointRow>> {
        Ok(self
            .conn
            .query_row("SELECT * FROM checkpoints WHERE project_id = ?1 AND id = ?2", [project_id, id], checkpoint_from_row)
            .optional()?)
    }

    pub fn checkpoints(&self, project_id: &str, include_pruned: bool, limit: i64) -> Result<Vec<CheckpointRow>> {
        let mut st = self.conn.prepare(
            "SELECT * FROM checkpoints WHERE project_id = ?1 AND (?2 OR pruned = 0) ORDER BY created_ms DESC, id DESC LIMIT ?3",
        )?;
        let rows = st.query_map(params![project_id, include_pruned, limit], checkpoint_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn checkpoints_by_state(&self, project_id: &str, state: &str) -> Result<Vec<CheckpointRow>> {
        let mut st = self
            .conn
            .prepare("SELECT * FROM checkpoints WHERE project_id = ?1 AND push_state = ?2 AND pruned = 0 ORDER BY id")?;
        let rows = st.query_map(params![project_id, state], checkpoint_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_push_state(&self, project_id: &str, id: &str, state: &str) -> Result<()> {
        self.conn
            .execute("UPDATE checkpoints SET push_state = ?1 WHERE project_id = ?2 AND id = ?3", params![state, project_id, id])?;
        Ok(())
    }

    pub fn mark_applied(&self, project_id: &str, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE checkpoints SET applied_ms = ?1 WHERE project_id = ?2 AND id = ?3", params![now_ms(), project_id, id])?;
        Ok(())
    }

    pub fn mark_pruned(&self, project_id: &str, id: &str) -> Result<()> {
        self.conn.execute("UPDATE checkpoints SET pruned = 1 WHERE project_id = ?1 AND id = ?2", params![project_id, id])?;
        Ok(())
    }

    pub fn last_pushed_own(&self, project_id: &str, device_id: &str) -> Result<Option<CheckpointRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT * FROM checkpoints WHERE project_id = ?1 AND device_id = ?2 AND push_state = 'pushed'
                 ORDER BY id DESC LIMIT 1",
                params![project_id, device_id],
                checkpoint_from_row,
            )
            .optional()?)
    }

    // ---- queue --------------------------------------------------------

    pub fn enqueue(&self, project_id: &str, kind: &str, payload: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO queue (project_id, kind, payload, next_attempt_ms, created_ms) VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(project_id, kind, payload) DO UPDATE SET next_attempt_ms = MIN(queue.next_attempt_ms, excluded.next_attempt_ms)",
            params![project_id, kind, payload, now_ms()],
        )?;
        Ok(())
    }

    pub fn due_queue(&self, now: i64) -> Result<Vec<QueueItem>> {
        let mut st = self.conn.prepare("SELECT * FROM queue WHERE next_attempt_ms <= ?1 ORDER BY next_attempt_ms LIMIT 50")?;
        let rows = st.query_map([now], queue_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn queue(&self, project_id: Option<&str>) -> Result<Vec<QueueItem>> {
        let mut st = self.conn.prepare("SELECT * FROM queue WHERE (?1 IS NULL OR project_id = ?1) ORDER BY next_attempt_ms")?;
        let rows = st.query_map([project_id], queue_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn queue_done(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM queue WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn queue_failed(&self, id: i64, err: &str, kind: crate::error::FailureKind) -> Result<()> {
        let attempts: i64 = self.conn.query_row("SELECT attempts FROM queue WHERE id = ?1", [id], |r| r.get(0))?;
        let next = now_ms() + backoff_ms(attempts, kind == crate::error::FailureKind::Auth);
        self.conn.execute(
            "UPDATE queue SET attempts = attempts + 1, next_attempt_ms = ?1, last_error = ?2, failure_kind = ?3 WHERE id = ?4",
            params![next, err, kind.to_string(), id],
        )?;
        Ok(())
    }

    /// Make every queued item due now (user pressed "sync now", network back).
    pub fn queue_retry_now(&self, project_id: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE queue SET next_attempt_ms = ?1 WHERE (?2 IS NULL OR project_id = ?2)",
            params![now_ms(), project_id],
        )?;
        Ok(())
    }

    // ---- activity -----------------------------------------------------

    pub fn log(&self, project_id: Option<&str>, level: &str, kind: &str, message: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO activity (project_id, ts_ms, level, kind, message) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project_id, now_ms(), level, kind, message],
        )?;
        // Bounded log.
        self.conn
            .execute("DELETE FROM activity WHERE id <= (SELECT MAX(id) - 5000 FROM activity)", [])?;
        Ok(())
    }

    pub fn activity(&self, project_id: Option<&str>, limit: i64) -> Result<Vec<Activity>> {
        let mut st = self.conn.prepare(
            "SELECT id, project_id, ts_ms, level, kind, message FROM activity WHERE (?1 IS NULL OR project_id = ?1)
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![project_id, limit], |r| {
            Ok(Activity {
                id: r.get(0)?,
                project_id: r.get(1)?,
                ts_ms: r.get(2)?,
                level: r.get(3)?,
                kind: r.get(4)?,
                message: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- operations journal --------------------------------------------

    pub fn op_start(&self, id: &str, project_id: &str, kind: &str, safety: Option<&str>, details: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO operations (id, project_id, kind, state, safety_checkpoint, details, started_ms) VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6)",
            params![id, project_id, kind, safety, details, now_ms()],
        )?;
        Ok(())
    }

    pub fn op_finish(&self, id: &str, state: &str, details: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE operations SET state = ?1, finished_ms = ?2, details = COALESCE(?3, details) WHERE id = ?4",
            params![state, now_ms(), details, id],
        )?;
        Ok(())
    }

    /// Called at startup: anything still `running` was cut short by a crash.
    pub fn mark_interrupted_ops(&self) -> Result<Vec<Operation>> {
        let mut st = self.conn.prepare("SELECT * FROM operations WHERE state = 'running'")?;
        let ops: Vec<Operation> = st.query_map([], op_from_row)?.collect::<rusqlite::Result<_>>()?;
        self.conn
            .execute("UPDATE operations SET state = 'interrupted', finished_ms = ?1 WHERE state = 'running'", [now_ms()])?;
        Ok(ops)
    }

    pub fn operations(&self, project_id: &str, limit: i64) -> Result<Vec<Operation>> {
        let mut st = self.conn.prepare("SELECT * FROM operations WHERE project_id = ?1 ORDER BY started_ms DESC LIMIT ?2")?;
        let rows = st.query_map(params![project_id, limit], op_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn ack_operation(&self, id: &str) -> Result<()> {
        self.conn.execute("UPDATE operations SET state = 'acknowledged' WHERE id = ?1 AND state = 'interrupted'", [id])?;
        Ok(())
    }

    // ---- conflicts ----------------------------------------------------

    pub fn insert_conflict(&self, c: &ConflictRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO conflicts (id, project_id, checkpoint_id, safety_checkpoint, plan_json, options_json, state, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7)",
            params![c.id, c.project_id, c.checkpoint_id, c.safety_checkpoint, c.plan_json, c.options_json, c.created_ms],
        )?;
        Ok(())
    }

    pub fn conflict(&self, id: &str) -> Result<ConflictRow> {
        self.conn
            .query_row("SELECT * FROM conflicts WHERE id = ?1", [id], conflict_from_row)
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("conflito {id}")))
    }

    pub fn conflicts(&self, project_id: Option<&str>, only_open: bool) -> Result<Vec<ConflictRow>> {
        let mut st = self.conn.prepare(
            "SELECT * FROM conflicts WHERE (?1 IS NULL OR project_id = ?1) AND (?2 = 0 OR state = 'open') ORDER BY created_ms DESC",
        )?;
        let rows = st.query_map(params![project_id, only_open], conflict_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_conflict_state(&self, id: &str, state: &str) -> Result<()> {
        self.conn.execute("UPDATE conflicts SET state = ?1, resolved_ms = ?2 WHERE id = ?3", params![state, now_ms(), id])?;
        Ok(())
    }

    // ---- secrets ------------------------------------------------------

    pub fn replace_findings(&self, project_id: &str, checkpoint_id: &str, f: &[crate::secrets::Finding]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM secret_findings WHERE project_id = ?1 AND checkpoint_id = ?2", [project_id, checkpoint_id])?;
        for x in f {
            tx.execute(
                "INSERT OR IGNORE INTO secret_findings (project_id, checkpoint_id, fingerprint, path, line, rule, severity, excerpt)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![project_id, checkpoint_id, x.fingerprint, x.path, x.line.map(|l| l as i64), x.rule, x.severity.as_str(), x.excerpt],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn findings(&self, project_id: &str, checkpoint_id: Option<&str>) -> Result<Vec<FindingRow>> {
        let mut st = self.conn.prepare(
            "SELECT f.*, (a.fingerprint IS NOT NULL) AS allowed FROM secret_findings f
             LEFT JOIN secret_allow a ON a.project_id = f.project_id AND a.fingerprint = f.fingerprint
             WHERE f.project_id = ?1 AND (?2 IS NULL OR f.checkpoint_id = ?2) ORDER BY f.checkpoint_id DESC, f.path",
        )?;
        let rows = st.query_map(params![project_id, checkpoint_id], |r| {
            Ok(FindingRow {
                checkpoint_id: r.get("checkpoint_id")?,
                fingerprint: r.get("fingerprint")?,
                path: r.get("path")?,
                line: r.get("line")?,
                rule: r.get("rule")?,
                severity: r.get("severity")?,
                excerpt: r.get("excerpt")?,
                allowed: r.get("allowed")?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn allow_fingerprint(&self, project_id: &str, fp: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO secret_allow (project_id, fingerprint, created_ms) VALUES (?1, ?2, ?3)",
            params![project_id, fp, now_ms()],
        )?;
        Ok(())
    }

    pub fn is_allowed(&self, project_id: &str, fp: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM secret_allow WHERE project_id = ?1 AND fingerprint = ?2", [project_id, fp], |_| Ok(()))
            .optional()?
            .is_some())
    }

    // ---- settings -----------------------------------------------------

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.db");
        {
            let db = Db::open(&path).unwrap();
            assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
            db.create_local_device("dev", "PC").unwrap();
            db.set_setting("k", "v").unwrap();
            db.enqueue("p", "push", "").unwrap();
            db.enqueue("p", "push", "").unwrap(); // coalesced
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.local_device().unwrap().unwrap().1, "PC");
        assert_eq!(db.setting("k").unwrap().as_deref(), Some("v"));
        assert_eq!(db.queue(None).unwrap().len(), 1);
    }

    #[test]
    fn queue_backoff_grows_and_caps() {
        assert_eq!(backoff_ms(0, false), 15_000);
        assert_eq!(backoff_ms(1, false), 30_000);
        assert_eq!(backoff_ms(30, false), 1_800_000);
        assert_eq!(backoff_ms(30, true), 7_200_000);
        let db = Db::open_in_memory().unwrap();
        db.enqueue("p", "push", "").unwrap();
        let item = &db.due_queue(now_ms() + 1).unwrap()[0];
        db.queue_failed(item.id, "offline", crate::error::FailureKind::Offline).unwrap();
        assert!(db.due_queue(now_ms() + 1).unwrap().is_empty());
        let q = &db.queue(None).unwrap()[0];
        assert_eq!(q.attempts, 1);
        db.queue_retry_now(None).unwrap();
        assert_eq!(db.due_queue(now_ms() + 1).unwrap().len(), 1);
    }

    #[test]
    fn interrupted_operations_are_detected() {
        let db = Db::open_in_memory().unwrap();
        db.op_start("op1", "p", "apply", Some("cp"), "").unwrap();
        let ops = db.mark_interrupted_ops().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(db.operations("p", 10).unwrap()[0].state, "interrupted");
    }
}
