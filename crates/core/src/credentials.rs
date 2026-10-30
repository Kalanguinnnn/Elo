//! Token storage in the OS credential vault and the GIT_ASKPASS bridge.
//!
//! Tokens never touch disk in plain text, never appear in argv or URLs: git
//! obtains them by running our own binary as `GIT_ASKPASS`, which reads the
//! vault and prints the secret on stdout (a pipe owned by git).

use crate::error::{Error, Result};

pub const SERVICE: &str = "continuity";
pub const GITHUB_ACCOUNT: &str = "github-token";

pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    fn delete(&self, account: &str) -> Result<()>;
}

pub struct KeyringStore;

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        let e = keyring::Entry::new(SERVICE, account).map_err(|e| Error::Credentials(e.to_string()))?;
        match e.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Credentials(e.to_string())),
        }
    }
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        let e = keyring::Entry::new(SERVICE, account).map_err(|e| Error::Credentials(e.to_string()))?;
        e.set_password(secret).map_err(|e| Error::Credentials(e.to_string()))
    }
    fn delete(&self, account: &str) -> Result<()> {
        let e = keyring::Entry::new(SERVICE, account).map_err(|e| Error::Credentials(e.to_string()))?;
        match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Credentials(e.to_string())),
        }
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemoryStore(std::sync::Mutex<std::collections::HashMap<String, String>>);
impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.0.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<()> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

/// Whether a remote URL is GitHub over HTTPS (where our token applies).
pub fn is_github_https(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("https://github.com/") || u.starts_with("https://www.github.com/")
}

/// Entry point for `GIT_ASKPASS` mode. Returns the line to print, or None.
///
/// Only answers prompts for github.com, so a hostile remote URL cannot
/// harvest the token.
pub fn askpass_answer(prompt: &str, store: &dyn SecretStore) -> Option<String> {
    let p = prompt.to_ascii_lowercase();
    // Prompt shape: `Username for 'https://github.com': ` /
    // `Password for 'https://x-access-token@github.com': `.
    let quoted = p.split_once('\'').and_then(|(_, rest)| rest.rsplit_once('\'')).map(|(q, _)| q)?;
    let authority = quoted.strip_prefix("https://")?;
    let authority = authority.split('/').next().unwrap_or("");
    let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    if host != "github.com" {
        return None;
    }
    if p.starts_with("username") {
        Some("x-access-token".into())
    } else if p.starts_with("password") {
        store.get(GITHUB_ACCOUNT).ok().flatten()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn askpass_only_answers_github() {
        let s = MemoryStore::default();
        s.set(GITHUB_ACCOUNT, "tok").unwrap();
        assert_eq!(askpass_answer("Username for 'https://github.com': ", &s).as_deref(), Some("x-access-token"));
        assert_eq!(
            askpass_answer("Password for 'https://x-access-token@github.com': ", &s).as_deref(),
            Some("tok")
        );
        assert_eq!(askpass_answer("Password for 'https://evil.example': ", &s), None);
        assert_eq!(askpass_answer("Password for 'https://github.com.evil.io': ", &s), None);
    }
}
