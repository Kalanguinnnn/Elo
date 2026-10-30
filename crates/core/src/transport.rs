//! Moving checkpoint refs between clones. Never uses `--force` or `+refspecs`
//! on push: checkpoint refs are immutable, so every push only creates refs (or
//! deletes our own, for retention).

use crate::checkpoint::REF_PREFIX;
use crate::error::{Error, Result};
use crate::git::{os_args, Git, RunOpts};
use std::ffi::{OsStr, OsString};
use std::time::Duration;

pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(180);

/// How to authenticate network operations for a remote.
#[derive(Debug, Clone, Default)]
pub struct NetAuth {
    /// When set, git asks this program for credentials (`GIT_ASKPASS`), and
    /// configured credential helpers are bypassed.
    pub askpass: Option<OsString>,
    pub extra_env: Vec<(OsString, OsString)>,
}

impl NetAuth {
    pub fn apply(&self, mut o: RunOpts) -> RunOpts {
        o = o.timeout(NETWORK_TIMEOUT);
        if let Some(p) = &self.askpass {
            o = o
                .config("credential.helper", "")
                .env("GIT_ASKPASS", p.clone())
                .env("SSH_ASKPASS", p.clone())
                .env("CONTINUITY_ASKPASS", "1");
        }
        if std::env::var_os("GIT_SSH_COMMAND").is_none() {
            // Never block on an interactive SSH prompt from a background process.
            o = o.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=20");
        }
        for (k, v) in &self.extra_env {
            o = o.env(k.clone(), v.clone());
        }
        o
    }
}

/// Push refs to the remote. Each refspec is `ref:ref`, never prefixed by `+`.
pub fn push_refs(git: &Git, remote: &str, refs: &[String], auth: &NetAuth) -> Result<()> {
    if refs.is_empty() {
        return Ok(());
    }
    let specs: Vec<String> = refs
        .iter()
        .map(|r| {
            debug_assert!(r.starts_with(REF_PREFIX) || r.starts_with("refs/heads/"));
            format!("{r}:{r}")
        })
        .collect();
    push_specs(git, remote, &specs, auth)
}

/// Push a branch fast-forward only (used by the optional auto-commit mode).
pub fn push_branch(git: &Git, remote: &str, branch_ref: &str, auth: &NetAuth) -> Result<()> {
    push_specs(git, remote, &[format!("{branch_ref}:{branch_ref}")], auth)
}

fn push_specs(git: &Git, remote: &str, specs: &[String], auth: &NetAuth) -> Result<()> {
    for s in specs {
        if s.starts_with('+') || s.starts_with('-') {
            return Err(Error::Invalid(format!("refspec proibida: {s}")));
        }
    }
    validate_remote_name(remote)?;
    let mut args: Vec<&OsStr> = vec![OsStr::new("push"), OsStr::new("--porcelain"), OsStr::new("--no-verify"), OsStr::new(remote)];
    for s in specs {
        args.push(OsStr::new(s));
    }
    git.run(&args, auth.apply(RunOpts::default()))?;
    Ok(())
}

/// Delete our own checkpoint refs on the remote (retention). A deletion is not a
/// force push: it can only remove the exact refs named.
pub fn delete_remote_refs(git: &Git, remote: &str, refs: &[String], auth: &NetAuth) -> Result<()> {
    if refs.is_empty() {
        return Ok(());
    }
    validate_remote_name(remote)?;
    let mut args: Vec<&OsStr> = vec![OsStr::new("push"), OsStr::new("--porcelain"), OsStr::new("--no-verify"), OsStr::new(remote)];
    let specs: Vec<String> = refs.iter().filter(|r| r.starts_with(REF_PREFIX)).map(|r| format!(":{r}")).collect();
    for s in &specs {
        args.push(OsStr::new(s));
    }
    git.run(&args, auth.apply(RunOpts::default()))?;
    Ok(())
}

/// Fetch every checkpoint ref from the remote into the same local namespace.
/// No `+`: an existing local checkpoint ref is never rewritten.
pub fn fetch_checkpoints(git: &Git, remote: &str, auth: &NetAuth) -> Result<()> {
    validate_remote_name(remote)?;
    let spec = format!("{REF_PREFIX}*:{REF_PREFIX}*");
    git.run(
        os_args!["fetch", "--no-tags", "--no-write-fetch-head", "--no-auto-gc", "--quiet", remote, spec],
        auth.apply(RunOpts::default()),
    )?;
    Ok(())
}

/// Cheap connectivity probe.
pub fn probe_remote(git: &Git, remote: &str, auth: &NetAuth) -> Result<()> {
    validate_remote_name(remote)?;
    git.run(os_args!["ls-remote", "--heads", remote], auth.apply(RunOpts::default()))?;
    Ok(())
}

pub fn validate_remote_name(remote: &str) -> Result<()> {
    if remote.is_empty() || remote.starts_with('-') || remote.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::Invalid(format!("nome de remoto inválido: {remote:?}")));
    }
    Ok(())
}

/// URL validation for user-provided remotes: no option injection, no `ext::`
/// transport (which runs arbitrary commands).
pub fn validate_remote_url(url: &str) -> Result<()> {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    if u.is_empty() || u.starts_with('-') || u.chars().any(|c| c.is_control()) {
        return Err(Error::Invalid("URL de remoto inválida".into()));
    }
    // `<helper>::<address>` selects a remote helper (ext::, fd::, ...).
    if let Some(i) = lower.find("::") {
        if i > 0 && lower[..i].chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
            return Err(Error::Invalid("transporte Git não permitido".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dangerous_remote_urls() {
        assert!(validate_remote_url("https://github.com/a/b.git").is_ok());
        assert!(validate_remote_url("git@github.com:a/b.git").is_ok());
        assert!(validate_remote_url("/tmp/remote.git").is_ok());
        assert!(validate_remote_url("ext::sh -c touch% /tmp/pwned").is_err());
        assert!(validate_remote_url("--upload-pack=evil").is_err());
        assert!(validate_remote_url("fd::3").is_err());
    }
}
