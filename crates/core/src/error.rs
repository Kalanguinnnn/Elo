use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("git {args} falhou (código {code:?}): {stderr}")]
    Git {
        args: String,
        code: Option<i32>,
        stderr: String,
    },
    #[error("git não encontrado no PATH: {0}")]
    GitMissing(String),
    #[error("operação expirou após {0}s")]
    Timeout(u64),
    #[error("repositório ocupado: {0}")]
    Busy(String),
    #[error("caminho inseguro: {0}")]
    UnsafePath(String),
    #[error("não encontrado: {0}")]
    NotFound(String),
    #[error("entrada inválida: {0}")]
    Invalid(String),
    #[error("conflito: {0}")]
    Conflict(String),
    #[error("ação requer confirmação: {0}")]
    NeedsConfirmation(String),
    #[error("banco de dados: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("credenciais: {0}")]
    Credentials(String),
    #[error("rede: {0}")]
    Network(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Coarse classification of a failed remote operation, used by the retry queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Network unreachable, DNS, timeouts: retry with backoff.
    Offline,
    /// Credentials rejected: retry slowly and surface to the user.
    Auth,
    /// Remote refused the update (e.g. ref exists, hook rejected): needs attention.
    Rejected,
    Other,
}

impl fmt::Display for FailureKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            FailureKind::Offline => "offline",
            FailureKind::Auth => "auth",
            FailureKind::Rejected => "rejected",
            FailureKind::Other => "other",
        };
        f.write_str(s)
    }
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Error::Git { .. } => "git",
            Error::GitMissing(_) => "git_missing",
            Error::Timeout(_) => "timeout",
            Error::Busy(_) => "busy",
            Error::UnsafePath(_) => "unsafe_path",
            Error::NotFound(_) => "not_found",
            Error::Invalid(_) => "invalid",
            Error::Conflict(_) => "conflict",
            Error::NeedsConfirmation(_) => "needs_confirmation",
            Error::Db(_) => "db",
            Error::Io(_) => "io",
            Error::Json(_) => "json",
            Error::Credentials(_) => "credentials",
            Error::Network(_) => "network",
            Error::Other(_) => "other",
        }
    }

    /// Classify a remote failure from git's stderr (LC_ALL=C is forced, so the
    /// messages are in English).
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            Error::Timeout(_) | Error::Network(_) => FailureKind::Offline,
            Error::Credentials(_) => FailureKind::Auth,
            Error::Git { stderr, .. } => classify_git_stderr(stderr),
            _ => FailureKind::Other,
        }
    }
}

pub fn classify_git_stderr(stderr: &str) -> FailureKind {
    let s = stderr.to_ascii_lowercase();
    const OFFLINE: &[&str] = &[
        "could not resolve host",
        "could not resolve hostname",
        "unable to access",
        "failed to connect",
        "connection timed out",
        "connection refused",
        "network is unreachable",
        "operation timed out",
        "could not read from remote repository",
        "does not appear to be a git repository",
        "early eof",
        "the remote end hung up",
        "temporary failure in name resolution",
        "ssl_connect",
        "schannel",
    ];
    const AUTH: &[&str] = &[
        "authentication failed",
        "permission denied",
        "invalid username or password",
        "terminal prompts disabled",
        "403",
        "401",
        "could not read username",
        "could not read password",
    ];
    const REJECTED: &[&str] = &["[rejected]", "! [remote rejected]", "failed to push some refs"];
    // Auth markers are checked first: "unable to access ... 403" is an auth issue.
    if AUTH.iter().any(|m| s.contains(m)) {
        FailureKind::Auth
    } else if REJECTED.iter().any(|m| s.contains(m)) {
        FailureKind::Rejected
    } else if OFFLINE.iter().any(|m| s.contains(m)) {
        FailureKind::Offline
    } else {
        FailureKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_git_failures() {
        assert_eq!(
            classify_git_stderr("fatal: unable to access 'https://x/': Could not resolve host: x"),
            FailureKind::Offline
        );
        assert_eq!(
            classify_git_stderr("remote: Invalid username or password.\nfatal: Authentication failed"),
            FailureKind::Auth
        );
        assert_eq!(
            classify_git_stderr(" ! [rejected] main -> main (non-fast-forward)"),
            FailureKind::Rejected
        );
        assert_eq!(classify_git_stderr("something odd"), FailureKind::Other);
    }
}
