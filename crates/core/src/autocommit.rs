//! Optional "conventional commits" mode: besides checkpoints, record the work
//! as regular commits on the current branch. Off by default because it moves
//! the branch and rewrites the staging area to match the new commit.

use crate::db::Project;
use crate::engine::Engine;
use crate::error::{Error, Result};
use crate::git::{os_args, Git, RunOpts};
use crate::snapshot;

pub fn commit_message(device: &str, files: usize) -> String {
    format!(
        "chore(continuity): salvamento automático ({files} arquivo(s))\n\nCriado automaticamente pelo Continuity em {}.\n",
        crate::util::sanitize_line(device, 80)
    )
}

pub fn auto_commit(engine: &Engine, p: &Project, git: &Git) -> Result<Option<String>> {
    if let Some(b) = snapshot::git_busy(git) {
        return Err(Error::Busy(b));
    }
    let Some(branch) = git.head_branch()? else {
        return Err(Error::Invalid("HEAD destacado".into()));
    };
    let head = git.head_commit()?;
    let head_tree = match &head {
        Some(h) => git.tree_of(h)?,
        None => git.empty_tree()?,
    };
    let state = snapshot::capture(git)?;
    if state.index_tree != head_tree {
        // Respect a partial staging the user prepared by hand.
        return Err(Error::Invalid("há alterações no staging; commit automático adiado".into()));
    }
    if state.tree == head_tree {
        return Ok(None);
    }
    let files = snapshot::change_count(git, &head_tree, &state.tree)?;
    let parents: Vec<&str> = head.as_deref().into_iter().collect();
    let commit = snapshot::commit_tree(git, &state.tree, &parents, &commit_message(&engine.device().name, files))?;
    git.cas_ref(&branch, &commit, head.as_deref())?;
    git.run(os_args!["read-tree", state.tree.as_str()], RunOpts::default())?;
    let _ = git.run(os_args!["update-index", "-q", "--refresh"], RunOpts::default().ok_code(1));
    if p.auto_push && git.config_get(&format!("remote.{}.url", p.remote))?.is_some() {
        engine.db().enqueue(&p.id, "push_branch", &branch)?;
    }
    engine.db().log(
        Some(&p.id),
        "info",
        "autocommit",
        &format!("Commit automático {} em {}", &commit[..10], crate::checkpoint::short_branch(&branch)),
    )?;
    Ok(Some(commit))
}
