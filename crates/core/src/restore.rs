//! Recovery: restore individual files in place, or a whole checkpoint into a
//! separate directory, with integrity verification.

use crate::apply::{self, ApplyPlan, ApplyResult, Expect, FileAction, FileChange, HeadAction, MODE_GITLINK, MODE_SYMLINK};
use crate::error::{Error, Result};
use crate::git::Git;
use crate::paths;
use crate::snapshot::WorkState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RestoreReport {
    pub target: String,
    pub files_written: usize,
    pub skipped: Vec<String>,
    pub integrity_failures: Vec<String>,
    pub verified: usize,
}

/// Restore the full tree of a checkpoint into `target`, which must be absent or
/// an empty directory outside the project.
pub fn restore_to_dir(git: &Git, tree: &str, target: &Path) -> Result<RestoreReport> {
    let target = absolute(target)?;
    // Compare canonical forms: short (8.3) names, case and `..` must not let
    // a path inside the project slip through.
    let canon = canonical_prefix(&target)?;
    if canon.starts_with(canonical_prefix(&git.workdir)?) || canon.starts_with(canonical_prefix(&git.common_dir)?) {
        return Err(Error::Invalid("o destino da recuperação deve ficar fora do projeto".into()));
    }
    if target.exists() {
        if !target.is_dir() || std::fs::read_dir(&target)?.next().is_some() {
            return Err(Error::Invalid("o destino deve ser um diretório vazio ou inexistente".into()));
        }
    } else {
        std::fs::create_dir_all(&target)?;
    }
    let entries = git.ls_tree(tree)?;
    let mut rep = RestoreReport { target: target.display().to_string(), ..Default::default() };
    let mut to_verify = Vec::new();
    let mut expected = Vec::new();
    let mut sorted: Vec<_> = entries.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, entry) in sorted {
        if entry.mode == MODE_GITLINK {
            rep.skipped.push(format!("{path} (submódulo)"));
            continue;
        }
        match apply::write_entry(git, &target, &path, &entry) {
            Ok(()) => {
                rep.files_written += 1;
                if entry.mode != MODE_SYMLINK {
                    to_verify.push(path.clone());
                    expected.push(entry.oid.clone());
                }
            }
            Err(e) => rep.skipped.push(format!("{path} ({e})")),
        }
    }
    let got = git.hash_paths(&target, &to_verify)?;
    for ((p, want), have) in to_verify.iter().zip(&expected).zip(&got) {
        if want == have {
            rep.verified += 1;
        } else {
            rep.integrity_failures.push(p.clone());
        }
    }
    Ok(rep)
}

/// Plan an in-place restore of `paths` to their content in `checkpoint_tree`.
/// A path missing from the checkpoint is deleted locally.
pub fn plan_in_place(git: &Git, current: &WorkState, checkpoint_tree: &str, rel_paths: &[String]) -> Result<ApplyPlan> {
    let target = git.ls_tree(checkpoint_tree)?;
    let ours = git.ls_tree(&current.tree)?;
    let mut changes = Vec::new();
    let mut skipped = Vec::new();
    for p in rel_paths {
        paths::validate_rel_path(p)?;
        let want = target.get(p);
        let have = ours.get(p);
        if want == have {
            continue;
        }
        if want.is_some_and(|w| w.mode == MODE_GITLINK) {
            skipped.push(apply::SkippedEntry { path: p.clone(), reason: "submódulo".into() });
            continue;
        }
        let expect = match have {
            Some(h) => Expect::Entry { entry: h.clone() },
            None if std::fs::symlink_metadata(paths::join_rel(&git.workdir, p)?).is_ok() => Expect::BackupExisting,
            None => Expect::Absent,
        };
        let action = match want {
            Some(w) => FileAction::Write { entry: w.clone() },
            None => FileAction::Delete,
        };
        changes.push(FileChange { path: p.clone(), action, expect });
    }
    Ok(ApplyPlan {
        base_tree: current.tree.clone(),
        ours_tree: current.tree.clone(),
        theirs_tree: checkpoint_tree.to_string(),
        changes,
        conflicts: Vec::new(),
        skipped,
        head: HeadAction::Keep,
        index_tree: None,
        local_changes: false,
        ours_branch: current.branch.clone(),
        theirs_branch: current.branch.clone(),
        needs_branch_confirmation: false,
        blockers: Vec::new(),
        notes: Vec::new(),
    })
}

pub fn execute_in_place(git: &Git, plan: &ApplyPlan, preserve_dir: &Path) -> Result<ApplyResult> {
    apply::execute(git, plan, &plan.changes, &[], preserve_dir)
}

/// Canonicalize the longest existing ancestor of `p` and re-append the rest,
/// so paths that do not exist yet can still be compared reliably.
fn canonical_prefix(p: &Path) -> Result<PathBuf> {
    let mut existing = p.to_path_buf();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => break,
        }
    }
    let mut out = std::fs::canonicalize(&existing)?;
    for comp in rest.into_iter().rev() {
        out.push(comp);
    }
    Ok(out)
}

fn absolute(p: &Path) -> Result<PathBuf> {
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(p))
    }
}
