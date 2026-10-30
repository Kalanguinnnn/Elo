//! Protected application of a checkpoint from another device onto this
//! working tree.
//!
//! The plan is a per-file three-way merge (base / ours / theirs). Nothing is
//! written until the plan has no unresolved conflicts, every target file is
//! re-verified against what the plan expects, and the caller has created a
//! safety checkpoint of the current state.

use crate::checkpoint::short_branch;
use crate::error::{Error, Result};
use crate::git::{os_args, Git, RunOpts, TreeEntry};
use crate::paths;
use crate::snapshot::{RefCheckpoint, WorkState};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub const MODE_SYMLINK: &str = "120000";
pub const MODE_GITLINK: &str = "160000";
pub const MODE_EXEC: &str = "100755";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyOptions {
    /// Explicit consent to move HEAD to another branch.
    #[serde(default)]
    pub allow_branch_switch: bool,
    /// Apply only file contents; never touch HEAD, branch or staging.
    #[serde(default)]
    pub files_only: bool,
}

/// What the file on disk must currently be for a change to proceed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expect {
    Absent,
    Entry { entry: TreeEntry },
    /// An untracked/ignored local file the user agreed to replace; it is
    /// backed up to the preserved directory first.
    BackupExisting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileAction {
    Write { entry: TreeEntry },
    WriteContent { content_b64: String },
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub action: FileAction,
    pub expect: Expect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictEntry {
    pub path: String,
    pub base: Option<TreeEntry>,
    pub ours: Option<TreeEntry>,
    pub theirs: Option<TreeEntry>,
    /// `both_modified`, `local_untracked`, ...
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedEntry {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HeadAction {
    Keep,
    FastForward { branch: String, from: Option<String>, to: String },
    SwitchBranch { branch: String, create: bool, from: Option<String>, to: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyPlan {
    pub base_tree: String,
    pub ours_tree: String,
    pub theirs_tree: String,
    pub changes: Vec<FileChange>,
    pub conflicts: Vec<ConflictEntry>,
    pub skipped: Vec<SkippedEntry>,
    pub head: HeadAction,
    /// Tree to load into the real index after applying, if any.
    pub index_tree: Option<String>,
    pub local_changes: bool,
    pub ours_branch: Option<String>,
    pub theirs_branch: Option<String>,
    pub needs_branch_confirmation: bool,
    /// Reasons why the plan cannot be executed as requested.
    pub blockers: Vec<String>,
    pub notes: Vec<String>,
}

impl ApplyPlan {
    /// Safe to run without a human: pure fast-forward, nothing local to merge.
    pub fn auto_eligible(&self) -> bool {
        !self.local_changes
            && self.conflicts.is_empty()
            && self.blockers.is_empty()
            && self.skipped.is_empty()
            && !self.needs_branch_confirmation
            && matches!(self.head, HeadAction::Keep | HeadAction::FastForward { .. })
    }
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty() && self.conflicts.is_empty() && self.head == HeadAction::Keep && self.index_tree.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "choice", rename_all = "snake_case")]
pub enum Resolution {
    /// Keep the local version.
    Ours,
    /// Take the incoming version.
    Theirs,
    /// Keep local at the path, write incoming next to it.
    Both,
    /// Content edited by the user (base64).
    Custom { content_b64: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyResult {
    pub written: Vec<String>,
    pub deleted: Vec<String>,
    pub preserved: Vec<String>,
    pub preserved_dir: Option<String>,
    pub integrity_failures: Vec<String>,
    pub head_updated: bool,
    pub index_updated: bool,
}

/// path -> (entry in , entry in ) for every changed path.
type DiffMap = BTreeMap<String, (Option<TreeEntry>, Option<TreeEntry>)>;

fn diff_map(git: &Git, a: &str, b: &str) -> Result<DiffMap> {
    let mut m = BTreeMap::new();
    if a == b {
        return Ok(m);
    }
    for d in git.diff_trees(a, b)? {
        m.insert(d.path.clone(), (d.old_entry(), d.new_entry()));
    }
    Ok(m)
}

fn tree_or_empty(git: &Git, commit: Option<&str>) -> Result<String> {
    match commit {
        Some(c) => git.tree_of(c),
        None => git.empty_tree(),
    }
}

pub fn plan(git: &Git, ours: &WorkState, theirs: &RefCheckpoint, base_tree: &str, opts: &ApplyOptions) -> Result<ApplyPlan> {
    let d_ours = diff_map(git, base_tree, &ours.tree)?;
    let d_theirs = diff_map(git, base_tree, &theirs.tree)?;

    let mut changes = Vec::new();
    let mut conflicts = Vec::new();
    let mut skipped = Vec::new();
    let mut notes = Vec::new();
    let mut blockers = Vec::new();

    // Paths present in ours, case-folded, to catch collisions on
    // case-insensitive filesystems.
    let ours_entries = git.ls_tree(&ours.tree)?;
    let ours_folded: HashMap<String, String> = ours_entries.keys().map(|k| (k.to_lowercase(), k.clone())).collect();

    for (path, (base_e, theirs_e)) in &d_theirs {
        if let Err(e) = paths::validate_rel_path(path) {
            skipped.push(SkippedEntry { path: path.clone(), reason: e.to_string() });
            continue;
        }
        if theirs_e.as_ref().is_some_and(|t| t.mode == MODE_GITLINK) || base_e.as_ref().is_some_and(|b| b.mode == MODE_GITLINK) {
            skipped.push(SkippedEntry { path: path.clone(), reason: "submódulo (gitlink) não é sincronizado".into() });
            continue;
        }
        if let Some(t) = theirs_e
            && t.mode == MODE_SYMLINK {
                let target = String::from_utf8_lossy(&git.blob_raw(&t.oid)?).into_owned();
                if !paths::symlink_target_is_contained(path, &target) {
                    skipped.push(SkippedEntry { path: path.clone(), reason: format!("symlink inseguro -> {target}") });
                    continue;
                }
            }
        if let Some((ours_base, ours_e)) = d_ours.get(path) {
            debug_assert_eq!(ours_base, base_e);
            if ours_e == theirs_e {
                continue; // same change on both sides
            }
            conflicts.push(ConflictEntry {
                path: path.clone(),
                base: base_e.clone(),
                ours: ours_e.clone(),
                theirs: theirs_e.clone(),
                reason: "both_modified".into(),
            });
            continue;
        }
        // Unchanged locally: ours == base.
        let expect = match base_e {
            Some(e) => Expect::Entry { entry: e.clone() },
            None => Expect::Absent,
        };
        if expect == Expect::Absent {
            let disk = paths::join_rel(&git.workdir, path)?;
            let exists = std::fs::symlink_metadata(&disk).is_ok();
            let folded = ours_folded.get(&path.to_lowercase()).filter(|p| *p != path);
            if exists || folded.is_some() {
                conflicts.push(ConflictEntry {
                    path: path.clone(),
                    base: None,
                    ours: None,
                    theirs: theirs_e.clone(),
                    reason: if folded.is_some() { "case_collision".into() } else { "local_untracked".into() },
                });
                continue;
            }
        }
        let action = match theirs_e {
            Some(t) => FileAction::Write { entry: t.clone() },
            None => FileAction::Delete,
        };
        changes.push(FileChange { path: path.clone(), action, expect });
    }

    // HEAD / branch.
    let mut needs_branch_confirmation = false;
    let mut head = HeadAction::Keep;
    if !opts.files_only {
        match (&theirs.meta.branch, &theirs.meta.head) {
            (_, None) => {}
            (None, Some(th)) => {
                if ours.head.as_deref() != Some(th) {
                    blockers.push("o outro dispositivo está com HEAD destacado; use 'aplicar apenas arquivos'".into());
                }
            }
            (Some(tb), Some(th)) if ours.branch.as_deref() == Some(tb.as_str()) => {
                match ours.head.as_deref() {
                    Some(oh) if oh == th => {}
                    None => head = HeadAction::FastForward { branch: tb.clone(), from: None, to: th.clone() },
                    Some(oh) => {
                        if git.is_ancestor(oh, th)? {
                            head = HeadAction::FastForward { branch: tb.clone(), from: Some(oh.into()), to: th.clone() };
                        } else if git.is_ancestor(th, oh)? {
                            notes.push("o HEAD local está à frente do checkpoint; HEAD mantido".into());
                        } else {
                            blockers.push(format!(
                                "a branch {} divergiu entre os dispositivos; use 'aplicar apenas arquivos'",
                                short_branch(tb)
                            ));
                        }
                    }
                }
            }
            (Some(tb), Some(th)) => {
                if !opts.allow_branch_switch {
                    needs_branch_confirmation = true;
                    blockers.push(format!(
                        "o checkpoint está na branch {}, aqui é {}; troca exige confirmação",
                        short_branch(tb),
                        ours.branch.as_deref().map(short_branch).unwrap_or("(HEAD destacado)")
                    ));
                } else {
                    let local = git.ref_value(tb)?;
                    match local.as_deref() {
                        None => head = HeadAction::SwitchBranch { branch: tb.clone(), create: true, from: None, to: th.clone() },
                        Some(l) if l == th || git.is_ancestor(l, th)? => {
                            head = HeadAction::SwitchBranch { branch: tb.clone(), create: false, from: Some(l.into()), to: th.clone() }
                        }
                        Some(_) => blockers.push(format!(
                            "a branch local {} divergiu do outro dispositivo; não será sobrescrita",
                            short_branch(tb)
                        )),
                    }
                }
            }
        }
    }

    // Staging.
    let ours_head_tree = tree_or_empty(git, ours.head.as_deref())?;
    let local_staged = ours.index_tree != ours_head_tree;
    let theirs_index_tree = git.tree_of(&theirs.meta.index_commit)?;
    let mut index_tree = None;
    if !opts.files_only {
        let head_moves = !matches!(head, HeadAction::Keep);
        if head_moves {
            if local_staged {
                blockers.push("há alterações no staging local que seriam perdidas; use 'aplicar apenas arquivos'".into());
            } else {
                index_tree = Some(theirs_index_tree);
            }
        } else if ours.head == theirs.meta.head && theirs.meta.head.is_some() || ours.head.is_none() && theirs.meta.head.is_none() {
            if !local_staged {
                if theirs_index_tree != ours.index_tree {
                    index_tree = Some(theirs_index_tree);
                }
            } else if theirs_index_tree != ours.index_tree {
                notes.push("staging local preservado (não foi substituído pelo do outro dispositivo)".into());
            }
        }
    }

    Ok(ApplyPlan {
        base_tree: base_tree.to_string(),
        ours_tree: ours.tree.clone(),
        theirs_tree: theirs.tree.clone(),
        local_changes: ours.tree != base_tree,
        changes,
        conflicts,
        skipped,
        head,
        index_tree,
        ours_branch: ours.branch.clone(),
        theirs_branch: theirs.meta.branch.clone(),
        needs_branch_confirmation,
        blockers,
        notes,
    })
}

/// Name used for the "keep both" copy of an incoming file.
pub fn sidecar_path(path: &str, label: &str) -> String {
    let label: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let label = label.trim_matches('-');
    let label = if label.is_empty() { "remoto" } else { label };
    let (dir, file) = match path.rsplit_once('/') {
        Some((d, f)) => (format!("{d}/"), f),
        None => (String::new(), path),
    };
    match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{dir}{stem}.conflict-{label}.{ext}"),
        _ => format!("{dir}{file}.conflict-{label}"),
    }
}

fn b64_decode(s: &str) -> Result<Vec<u8>> {
    crate::b64::decode(s).ok_or_else(|| Error::Invalid("base64 inválido".into()))
}

/// Versions to keep aside: (relative path under the preserved dir, entry).
pub type Preserve = Vec<(String, TreeEntry)>;

/// Turn a plan plus conflict resolutions into the final list of changes, and
/// list what must be preserved (the version that was not chosen).
pub fn resolve(plan: &ApplyPlan, resolutions: &HashMap<String, Resolution>, theirs_label: &str) -> Result<(Vec<FileChange>, Preserve)> {
    let mut changes = plan.changes.clone();
    let mut preserve = Vec::new();
    for c in &plan.conflicts {
        let r = resolutions
            .get(&c.path)
            .ok_or_else(|| Error::Conflict(format!("conflito sem resolução: {}", c.path)))?;
        let ours_expect = match (&c.ours, c.reason.as_str()) {
            (_, "local_untracked") => Expect::BackupExisting,
            (Some(o), _) => Expect::Entry { entry: o.clone() },
            (None, _) => Expect::Absent,
        };
        match r {
            Resolution::Ours => {
                if let Some(t) = &c.theirs {
                    preserve.push((format!("theirs/{}", c.path), t.clone()));
                }
            }
            Resolution::Theirs => {
                if let Some(o) = &c.ours {
                    preserve.push((format!("ours/{}", c.path), o.clone()));
                }
                let action = match &c.theirs {
                    Some(t) => FileAction::Write { entry: t.clone() },
                    None => FileAction::Delete,
                };
                changes.push(FileChange { path: c.path.clone(), action, expect: ours_expect });
            }
            Resolution::Both => {
                if let Some(t) = &c.theirs {
                    let side = sidecar_path(&c.path, theirs_label);
                    paths::validate_rel_path(&side)?;
                    changes.push(FileChange { path: side, action: FileAction::Write { entry: t.clone() }, expect: Expect::Absent });
                }
            }
            Resolution::Custom { content_b64 } => {
                b64_decode(content_b64)?;
                if let Some(o) = &c.ours {
                    preserve.push((format!("ours/{}", c.path), o.clone()));
                }
                if let Some(t) = &c.theirs {
                    preserve.push((format!("theirs/{}", c.path), t.clone()));
                }
                changes.push(FileChange {
                    path: c.path.clone(),
                    action: FileAction::WriteContent { content_b64: content_b64.clone() },
                    expect: ours_expect,
                });
            }
        }
    }
    Ok((changes, preserve))
}

/// Check that every target is still exactly what the plan saw.
pub fn verify_expectations(git: &Git, changes: &[FileChange]) -> Result<()> {
    let root = &git.workdir;
    let mut hash_paths = Vec::new();
    let mut hash_expect = Vec::new();
    for c in changes {
        let disk = paths::join_rel(root, &c.path)?;
        paths::check_no_symlink_ancestors(root, &disk)?;
        match &c.expect {
            Expect::Absent => {
                if std::fs::symlink_metadata(&disk).is_ok() {
                    return Err(Error::Conflict(format!("{} surgiu localmente durante a aplicação", c.path)));
                }
            }
            Expect::BackupExisting => {}
            Expect::Entry { entry } => {
                let md = std::fs::symlink_metadata(&disk)
                    .map_err(|_| Error::Conflict(format!("{} foi removido localmente durante a aplicação", c.path)))?;
                if md.file_type().is_symlink() {
                    #[cfg(unix)]
                    {
                        let target = std::fs::read_link(&disk)?;
                        let blob = git.blob_raw(&entry.oid)?;
                        if target.as_os_str().as_encoded_bytes() != blob.as_slice() {
                            return Err(Error::Conflict(format!("{} mudou localmente", c.path)));
                        }
                    }
                    continue;
                }
                if md.is_dir() {
                    return Err(Error::Conflict(format!("{} virou diretório localmente", c.path)));
                }
                hash_paths.push(c.path.clone());
                hash_expect.push(entry.oid.clone());
            }
        }
    }
    let got = git.hash_paths(root, &hash_paths)?;
    for ((p, want), have) in hash_paths.iter().zip(&hash_expect).zip(&got) {
        if want != have {
            return Err(Error::Conflict(format!("{p} mudou localmente durante a aplicação")));
        }
    }
    Ok(())
}

/// Write one change into `root` (the working tree or a restore directory).
pub fn write_change(git: &Git, root: &Path, c: &FileChange) -> Result<()> {
    let disk = paths::join_rel(root, &c.path)?;
    match &c.action {
        FileAction::Delete => paths::remove_file_and_empty_parents(root, &disk),
        FileAction::WriteContent { content_b64 } => paths::atomic_write(root, &disk, &b64_decode(content_b64)?, false),
        FileAction::Write { entry } => write_entry(git, root, &c.path, entry),
    }
}

pub fn write_entry(git: &Git, root: &Path, rel: &str, entry: &TreeEntry) -> Result<()> {
    let disk = paths::join_rel(root, rel)?;
    match entry.mode.as_str() {
        MODE_SYMLINK => {
            let target = String::from_utf8_lossy(&git.blob_raw(&entry.oid)?).into_owned();
            paths::write_symlink(root, &disk, rel, &target)
        }
        MODE_GITLINK => Err(Error::Invalid(format!("{rel}: submódulo não suportado"))),
        mode => {
            let data = git.blob_for_worktree(&entry.oid, rel)?;
            paths::atomic_write(root, &disk, &data, mode == MODE_EXEC)
        }
    }
}

/// Execute a fully resolved plan. The caller must hold the project lock and
/// must already have created a safety checkpoint.
pub fn execute(
    git: &Git,
    plan: &ApplyPlan,
    changes: &[FileChange],
    preserve: &[(String, TreeEntry)],
    preserve_dir: &Path,
) -> Result<ApplyResult> {
    if !plan.blockers.is_empty() {
        return Err(Error::NeedsConfirmation(plan.blockers.join("; ")));
    }
    verify_expectations(git, changes)?;
    let mut res = ApplyResult::default();

    // Preserve versions that were not chosen, and local files we replace.
    let mut backups: Vec<(String, PathBuf)> = Vec::new();
    for c in changes.iter().filter(|c| c.expect == Expect::BackupExisting) {
        backups.push((format!("local/{}", c.path), paths::join_rel(&git.workdir, &c.path)?));
    }
    if !preserve.is_empty() || !backups.is_empty() {
        std::fs::create_dir_all(preserve_dir)?;
        for (rel, entry) in preserve {
            write_entry(git, preserve_dir, rel, entry)?;
            res.preserved.push(rel.clone());
        }
        for (rel, src) in &backups {
            if src.is_file() {
                let dst = paths::join_rel(preserve_dir, rel)?;
                paths::atomic_write(preserve_dir, &dst, &std::fs::read(src)?, false)?;
                res.preserved.push(rel.clone());
            }
        }
        res.preserved_dir = Some(preserve_dir.display().to_string());
    }

    // HEAD / branch, with compare-and-swap so a concurrent commit is never lost.
    match &plan.head {
        HeadAction::Keep => {}
        HeadAction::FastForward { branch, from, to } => {
            git.cas_ref(branch, to, from.as_deref())?;
            res.head_updated = true;
        }
        HeadAction::SwitchBranch { branch, from, to, .. } => {
            git.cas_ref(branch, to, from.as_deref())?;
            git.run(os_args!["symbolic-ref", "HEAD", branch.as_str()], RunOpts::default())?;
            res.head_updated = true;
        }
    }

    for c in changes {
        write_change(git, &git.workdir, c)?;
        match c.action {
            FileAction::Delete => res.deleted.push(c.path.clone()),
            _ => res.written.push(c.path.clone()),
        }
    }

    if let Some(tree) = &plan.index_tree {
        git.run(os_args!["read-tree", tree.as_str()], RunOpts::default())?;
        let _ = git.run(os_args!["update-index", "-q", "--refresh"], RunOpts::default().ok_code(1));
        res.index_updated = true;
    }

    // Integrity: every regular file written from a blob must hash back to it.
    let check: Vec<(&String, &TreeEntry)> = changes
        .iter()
        .filter_map(|c| match &c.action {
            FileAction::Write { entry } if entry.mode != MODE_SYMLINK => Some((&c.path, entry)),
            _ => None,
        })
        .collect();
    let paths_only: Vec<String> = check.iter().map(|(p, _)| (*p).clone()).collect();
    let got = git.hash_paths(&git.workdir, &paths_only)?;
    for ((p, e), h) in check.iter().zip(got) {
        if e.oid != h {
            res.integrity_failures.push((*p).clone());
        }
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_names() {
        assert_eq!(sidecar_path("src/a.rs", "Laptop Pedro"), "src/a.conflict-laptop-pedro.rs");
        assert_eq!(sidecar_path("Makefile", "pc"), "Makefile.conflict-pc");
        assert_eq!(sidecar_path(".env", "pc"), ".env.conflict-pc");
    }
}
