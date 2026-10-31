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
    check_effective_urls(git, remote)?;
    let mut args: Vec<&OsStr> = vec![OsStr::new("push"), OsStr::new("--porcelain"), OsStr::new("--no-verify"), OsStr::new(remote)];
    for s in specs {
        args.push(OsStr::new(s));
    }
    run_push(git, &args, auth)
}

/// Delete our own checkpoint refs on the remote (retention). A deletion is not a
/// force push: it can only remove the exact refs named.
pub fn delete_remote_refs(git: &Git, remote: &str, refs: &[String], auth: &NetAuth) -> Result<()> {
    if refs.is_empty() {
        return Ok(());
    }
    check_effective_urls(git, remote)?;
    let mut args: Vec<&OsStr> = vec![OsStr::new("push"), OsStr::new("--porcelain"), OsStr::new("--no-verify"), OsStr::new(remote)];
    let specs: Vec<String> = refs.iter().filter(|r| r.starts_with(REF_PREFIX)).map(|r| format!(":{r}")).collect();
    for s in &specs {
        args.push(OsStr::new(s));
    }
    run_push(git, &args, auth)
}

/// Run a push. When the remote refuses refs, Git's error line only says
/// "failed to push some refs"; the reason per ref is in the `--porcelain`
/// output on stdout, so it is attached to the error.
fn run_push(git: &Git, args: &[&OsStr], auth: &NetAuth) -> Result<()> {
    let out = git.run(args, auth.apply(RunOpts::default().ok_code(1)))?;
    if out.code == 0 {
        return Ok(());
    }
    Err(Error::Git {
        args: "push".into(),
        code: Some(out.code),
        stderr: crate::util::redact_url_credentials(&push_failure_text(&String::from_utf8_lossy(&out.stdout), &out.stderr)),
    })
}

fn push_failure_text(porcelain: &str, stderr: &str) -> String {
    // Porcelain: `!<TAB><from>:<to><TAB>[remote rejected] (reason)`.
    let reasons: Vec<&str> = porcelain
        .lines()
        .filter(|l| l.starts_with('!'))
        .filter_map(|l| l.split('\t').nth(2))
        .map(str::trim)
        .collect();
    let mut text = String::new();
    if reasons.iter().any(|r| r.contains("`workflow` scope")) {
        text.push_str(
            "O GitHub exige a permissão \"workflow\" para enviar alterações em .github/workflows. Em Ajustes → GitHub, desconecte e entre de novo para concedê-la. ",
        );
    }
    text.push_str(stderr.trim());
    for r in reasons {
        text.push('\n');
        text.push_str(r);
    }
    text
}

/// Fetch every checkpoint ref from the remote into the same local namespace.
/// No `+`: an existing local checkpoint ref is never rewritten.
pub fn fetch_checkpoints(git: &Git, remote: &str, auth: &NetAuth) -> Result<()> {
    check_effective_urls(git, remote)?;
    let spec = format!("{REF_PREFIX}*:{REF_PREFIX}*");
    git.run(
        os_args!["fetch", "--no-tags", "--no-write-fetch-head", "--no-auto-gc", "--quiet", remote, spec],
        auth.apply(RunOpts::default()),
    )?;
    Ok(())
}

/// Cheap connectivity probe.
pub fn probe_remote(git: &Git, remote: &str, auth: &NetAuth) -> Result<()> {
    check_effective_urls(git, remote)?;
    git.run(os_args!["ls-remote", "--heads", remote], auth.apply(RunOpts::default()))?;
    Ok(())
}

pub fn validate_remote_name(remote: &str) -> Result<()> {
    if remote.is_empty() || remote.starts_with('-') || remote.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::Invalid(format!("nome de remoto inválido: {remote:?}")));
    }
    Ok(())
}

/// The URLs Git will really contact for `remote`, after `url.*.insteadOf` /
/// `pushInsteadOf` rewriting, must pass the same transport checks as a URL
/// typed by the user. A repository's config cannot turn a remote into
/// `ext::<command>` (also refused by [`crate::git::ALLOWED_PROTOCOLS`]).
pub fn check_effective_urls(git: &Git, remote: &str) -> Result<()> {
    validate_remote_name(remote)?;
    for push in [false, true] {
        let args: &[&OsStr] = if push {
            os_args!["remote", "get-url", "--push", "--all", remote]
        } else {
            os_args!["remote", "get-url", "--all", remote]
        };
        for url in git.run_text(args)?.lines().filter(|l| !l.trim().is_empty()) {
            check_transport(url)?;
        }
    }
    Ok(())
}

/// URL validation for user-provided remotes: no option injection, no remote
/// helpers such as `ext::` (which runs arbitrary commands), and no credentials
/// in the URL (they would end up in `.git/config`, argv and logs; GitHub access
/// goes through the OS vault instead).
pub fn validate_remote_url(url: &str) -> Result<()> {
    check_transport(url)?;
    if url_has_credentials(url.trim()) {
        return Err(Error::Invalid(
            "a URL contém usuário ou token; use a URL sem credenciais e conecte o GitHub em Ajustes (o token fica no cofre do sistema)".into(),
        ));
    }
    Ok(())
}

fn check_transport(url: &str) -> Result<()> {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    if u.is_empty() || u.starts_with('-') || u.chars().any(|c| c.is_control()) {
        return Err(Error::Invalid("URL de remoto inválida".into()));
    }
    // `<helper>::<address>` selects a remote helper (ext::, fd::, ...).
    if let Some(i) = lower.find("::")
        && i > 0 && lower[..i].chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
            return Err(Error::Invalid("transporte Git não permitido".into()));
        }
    Ok(())
}

/// `scheme://userinfo@host/...` where the userinfo can carry a secret: any
/// userinfo for HTTP(S)/FTP (a token alone is a credential), a password for
/// SSH (`ssh://git@host` is just the login name). scp-like `git@host:path`
/// cannot carry a password and is accepted.
fn url_has_credentials(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let Some((userinfo, _)) = authority.rsplit_once('@') else { return false };
    match scheme.to_ascii_lowercase().as_str() {
        "ssh" | "git+ssh" | "ssh+git" => userinfo.contains(':'),
        _ => true,
    }
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

    #[test]
    fn push_rejection_keeps_the_remote_reason() {
        let porcelain = "To https://github.com/a/b.git\n=\trefs/x:refs/x\t[up to date]\n!\trefs/y:refs/y\t[remote rejected] (refusing to allow an OAuth App to create or update workflow `.github/workflows/ci.yml` without `workflow` scope)\nDone\n";
        let t = push_failure_text(porcelain, "error: failed to push some refs to 'https://github.com/a/b.git'\n");
        assert!(t.starts_with("O GitHub exige a permissão \"workflow\""), "{t}");
        assert!(t.contains("[remote rejected] (refusing to allow an OAuth App"), "{t}");
        assert!(!t.contains("up to date"), "{t}");
        assert_eq!(crate::error::classify_git_stderr(&t), crate::error::FailureKind::Rejected);
        let plain = push_failure_text("!\trefs/z:refs/z\t[rejected] (fetch first)\n", "error: failed to push some refs");
        assert_eq!(plain, "error: failed to push some refs\n[rejected] (fetch first)");
    }

    #[test]
    fn audit3_finding2_rejects_credentials_in_urls() {
        for bad in [
            "https://user:ghp_secret@github.com/a/b.git",
            "https://ghp_secret@github.com/a/b.git",
            "HTTP://u:p@example.com/r",
            "ssh://git:hunter2@example.com/r.git",
        ] {
            let e = validate_remote_url(bad).unwrap_err().to_string();
            assert!(e.contains("credenciais") || e.contains("token"), "{bad}: {e}");
            assert!(!e.contains("secret") && !e.contains("hunter2"), "a mensagem não pode repetir o segredo: {e}");
        }
        for ok in ["ssh://git@github.com/a/b.git", "git@github.com:a/b.git", "https://github.com/a/b@v1", "file:///tmp/r.git"] {
            assert!(validate_remote_url(ok).is_ok(), "{ok}");
        }
    }
}
