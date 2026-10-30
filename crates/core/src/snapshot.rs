//! Snapshot Engine: capture the working tree into Git objects through an
//! isolated index, never touching the real index, HEAD or branch.

use crate::checkpoint::{self, CheckpointMeta};
use crate::error::{Error, Result};
use crate::git::{os_args, Git, RunOpts};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkState {
    /// Tree of the working tree (tracked + untracked, minus ignored).
    pub tree: String,
    /// Tree of the staging area.
    pub index_tree: String,
    pub head: Option<String>,
    pub branch: Option<String>,
}

/// Temp index file removed on drop.
struct TempIndex(PathBuf);
impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let mut lock = self.0.clone().into_os_string();
        lock.push(".lock");
        let _ = std::fs::remove_file(PathBuf::from(lock));
    }
}

/// Remove scratch index files left behind by a crash.
pub fn cleanup_stale_temp(git: &Git) {
    if let Ok(rd) = std::fs::read_dir(git.git_dir.join("continuity")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("index-") && (n.ends_with(".tmp") || n.ends_with(".tmp.lock")) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

pub fn git_busy(git: &Git) -> Option<String> {
    for f in ["index.lock", "MERGE_HEAD", "REBASE_HEAD", "CHERRY_PICK_HEAD", "rebase-merge", "rebase-apply", "BISECT_LOG"] {
        if git.git_dir.join(f).exists() {
            return Some(f.to_string());
        }
    }
    None
}

/// Capture the current state. Fails with `Busy` while git holds the index lock.
pub fn capture(git: &Git) -> Result<WorkState> {
    if git.git_dir.join("index.lock").exists() {
        return Err(Error::Busy("index.lock presente (operação Git em andamento)".into()));
    }
    let dir = git.private_dir()?;
    let tmp = TempIndex(dir.join(format!("index-{}.tmp", crate::util::random_hex(6))));
    let real = git.git_dir.join("index");
    if real.exists() {
        std::fs::copy(&real, &tmp.0)?;
    }
    let env = |o: RunOpts| o.env("GIT_INDEX_FILE", tmp.0.as_os_str());

    let index_tree = match git.run(os_args!["write-tree"], env(RunOpts::default())) {
        Ok(o) => o.text(),
        // Unmerged entries (merge in progress): record HEAD's tree as staging.
        Err(_) => match git.head_commit()? {
            Some(h) => git.tree_of(&h)?,
            None => git.empty_tree()?,
        },
    };
    git.run(
        os_args!["add", "-A", "--ignore-errors", "--", "."],
        env(RunOpts::default().config("advice.addEmbeddedRepo", "false").config("core.safecrlf", "false").ok_code(1)),
    )?;
    let tree = git.run(os_args!["write-tree"], env(RunOpts::default()))?.text();
    Ok(WorkState { tree, index_tree, head: git.head_commit()?, branch: git.head_branch()? })
}

fn identity_env(git: &Git, opts: RunOpts) -> Result<RunOpts> {
    let mut o = opts;
    if git.config_get("user.name")?.is_none() {
        o = o.env("GIT_AUTHOR_NAME", "Continuity").env("GIT_COMMITTER_NAME", "Continuity");
    }
    if git.config_get("user.email")?.is_none() {
        o = o.env("GIT_AUTHOR_EMAIL", "continuity@localhost").env("GIT_COMMITTER_EMAIL", "continuity@localhost");
    }
    Ok(o)
}

pub fn commit_tree(git: &Git, tree: &str, parents: &[&str], message: &str) -> Result<String> {
    let mut args: Vec<&OsStr> = vec![OsStr::new("commit-tree"), OsStr::new("--no-gpg-sign"), OsStr::new(tree)];
    for p in parents {
        args.push(OsStr::new("-p"));
        args.push(OsStr::new(p));
    }
    args.push(OsStr::new("-F"));
    args.push(OsStr::new("-"));
    let opts = identity_env(git, RunOpts::default().stdin(message.as_bytes().to_vec()))?;
    Ok(git.run(&args, opts)?.text())
}

/// Write the index commit and checkpoint commit and create its immutable ref.
/// `meta.index_commit` is filled in here. Returns the checkpoint commit oid.
pub fn write_checkpoint(
    git: &Git,
    state: &WorkState,
    meta: &mut CheckpointMeta,
    signer: Option<&crate::signing::DeviceKey>,
) -> Result<String> {
    let head_parent: Vec<&str> = state.head.as_deref().into_iter().collect();
    let idx_msg = format!("Continuity index {}\n\nContinuity-Index-Of: {}\n", meta.id, meta.id);
    let index_commit = commit_tree(git, &state.index_tree, &head_parent, &idx_msg)?;
    meta.index_commit = index_commit.clone();
    meta.head = state.head.clone();
    meta.branch = state.branch.clone();
    if let Some(key) = signer {
        meta.public_key = Some(key.public_key());
        meta.signature = Some(key.sign(&crate::signing::payload(meta, &state.tree)));
    }
    let mut parents = head_parent.clone();
    parents.push(&index_commit);
    let commit = commit_tree(git, &state.tree, &parents, &meta.to_message())?;
    git.create_ref(&checkpoint::ref_name(&meta.device_id, &meta.id), &commit)?;
    Ok(commit)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefCheckpoint {
    pub refname: String,
    pub commit: String,
    pub tree: String,
    pub meta: CheckpointMeta,
}

/// All checkpoint refs present locally, with parsed metadata.
pub fn list_local(git: &Git) -> Result<Vec<RefCheckpoint>> {
    let out = git.run(
        os_args!["for-each-ref", "--format=%(refname)%00%(objectname)%00%(tree)%00%(contents)%1e", checkpoint::REF_PREFIX],
        RunOpts::default(),
    )?;
    let mut v = Vec::new();
    for rec in out.stdout.split(|b| *b == 0x1e) {
        let rec = String::from_utf8_lossy(rec);
        let rec = rec.trim_start_matches('\n');
        let mut f = rec.splitn(4, '\0');
        let (Some(r), Some(c), Some(t), Some(msg)) = (f.next(), f.next(), f.next(), f.next()) else { continue };
        let Some((dev, id)) = checkpoint::parse_ref(r) else { continue };
        let Ok(meta) = CheckpointMeta::parse_message(msg) else { continue };
        // The ref path must agree with the signed-in metadata.
        if meta.device_id != dev || meta.id != id {
            continue;
        }
        v.push(RefCheckpoint { refname: r.to_string(), commit: c.to_string(), tree: t.to_string(), meta });
    }
    Ok(v)
}

pub fn read_checkpoint(git: &Git, refname: &str) -> Result<RefCheckpoint> {
    let commit = git.ref_value(refname)?.ok_or_else(|| Error::NotFound(refname.into()))?;
    let msg = git.run_text(os_args!["cat-file", "commit", commit.as_str()])?;
    let body = msg.split_once("\n\n").map(|(_, b)| b).unwrap_or("");
    Ok(RefCheckpoint {
        refname: refname.to_string(),
        tree: git.tree_of(&commit)?,
        commit,
        meta: CheckpointMeta::parse_message(body)?,
    })
}

/// Count of changed paths between two trees (for summaries).
pub fn change_count(git: &Git, from: &str, to: &str) -> Result<usize> {
    Ok(git.diff_trees(from, to)?.len())
}

pub fn workdir_rel(git: &Git, p: &Path) -> Option<String> {
    p.strip_prefix(&git.workdir).ok().map(|r| r.to_string_lossy().replace('\\', "/"))
}
