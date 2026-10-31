//! Minimal GitHub REST client: token validation, OAuth device flow, repository
//! listing and creation. Only called on explicit user action.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const API: &str = "https://api.github.com";
const UA: &str = concat!("continuity/", env!("CARGO_PKG_VERSION"));

fn agent() -> ureq::Agent {
    agent_with(Duration::from_secs(30))
}

fn agent_with(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(timeout).user_agent(UA).build()
}

fn map_err(e: ureq::Error) -> Error {
    match e {
        ureq::Error::Status(401, _) => Error::Credentials("token do GitHub inválido ou expirado".into()),
        ureq::Error::Status(code, r) => {
            let body = r.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
                .unwrap_or(body);
            Error::Network(format!("GitHub respondeu {code}: {msg}"))
        }
        ureq::Error::Transport(t) => Error::Network(t.to_string()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubUser {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repo {
    pub full_name: String,
    pub private: bool,
    pub clone_url: String,
    pub ssh_url: String,
    pub html_url: String,
    pub default_branch: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// ISO 8601, as GitHub sends it; the list is already sorted by it.
    #[serde(default)]
    pub updated_at: Option<String>,
}

pub fn user(token: &str) -> Result<GithubUser> {
    user_with(agent(), token)
}

/// Whether GitHub still accepts `token`: `Some(false)` when it is rejected
/// (revoked or expired), `None` when GitHub could not be reached. Short
/// timeout: it runs whenever the UI shows the connection.
pub fn token_accepted(token: &str) -> Option<bool> {
    match user_with(agent_with(Duration::from_secs(8)), token) {
        Ok(_) => Some(true),
        Err(Error::Credentials(_)) => Some(false),
        Err(_) => None,
    }
}

fn user_with(agent: ureq::Agent, token: &str) -> Result<GithubUser> {
    agent
        .get(&format!("{API}/user"))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(map_err)?
        .into_json()
        .map_err(|e| Error::Network(e.to_string()))
}

pub fn list_repos(token: &str) -> Result<Vec<Repo>> {
    let mut all = Vec::new();
    for page in 1..=5 {
        let v: Vec<Repo> = agent()
            .get(&format!("{API}/user/repos?per_page=100&sort=updated&page={page}"))
            .set("Authorization", &format!("Bearer {token}"))
            .set("Accept", "application/vnd.github+json")
            .call()
            .map_err(map_err)?
            .into_json()
            .map_err(|e| Error::Network(e.to_string()))?;
        let n = v.len();
        all.extend(v);
        if n < 100 {
            break;
        }
    }
    Ok(all)
}

pub fn create_repo(token: &str, name: &str, private: bool, description: &str) -> Result<Repo> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
        return Err(Error::Invalid("nome de repositório inválido".into()));
    }
    agent()
        .post(&format!("{API}/user/repos"))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/vnd.github+json")
        .send_json(serde_json::json!({ "name": name, "private": private, "description": description, "auto_init": false }))
        .map_err(map_err)?
        .into_json()
        .map_err(|e| Error::Network(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// Start the OAuth device flow. Requires the client id of an OAuth App
/// registered by the distributor (no client secret is needed).
pub fn device_start(client_id: &str) -> Result<DeviceCode> {
    agent()
        .post("https://github.com/login/device/code")
        .set("Accept", "application/json")
        // `workflow`: GitHub refuses pushes that touch .github/workflows without it,
        // and a checkpoint carries whatever the user edited there.
        .send_form(&[("client_id", client_id), ("scope", "repo workflow")])
        .map_err(map_err)?
        .into_json()
        .map_err(|e| Error::Network(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DevicePoll {
    Pending,
    SlowDown,
    Done { token: String },
    Failed { error: String },
}

pub fn device_poll(client_id: &str, device_code: &str) -> Result<DevicePoll> {
    let v: serde_json::Value = agent()
        .post("https://github.com/login/oauth/access_token")
        .set("Accept", "application/json")
        .send_form(&[
            ("client_id", client_id),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .map_err(map_err)?
        .into_json()
        .map_err(|e| Error::Network(e.to_string()))?;
    if let Some(t) = v.get("access_token").and_then(|t| t.as_str()) {
        return Ok(DevicePoll::Done { token: t.to_string() });
    }
    Ok(match v.get("error").and_then(|e| e.as_str()).unwrap_or("unknown") {
        "authorization_pending" => DevicePoll::Pending,
        "slow_down" => DevicePoll::SlowDown,
        other => DevicePoll::Failed { error: other.to_string() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_list_entries_parse_with_and_without_optional_fields() {
        let v: Vec<Repo> = serde_json::from_str(
            r#"[{"full_name":"a/elo","private":true,"clone_url":"https://github.com/a/elo.git","ssh_url":"git@github.com:a/elo.git",
                 "html_url":"https://github.com/a/elo","default_branch":"main","description":"Sincroniza","updated_at":"2026-10-31T12:00:00Z","id":1},
                {"full_name":"a/x","private":false,"clone_url":"https://github.com/a/x.git","ssh_url":"git@github.com:a/x.git",
                 "html_url":"https://github.com/a/x","default_branch":null,"description":null}]"#,
        )
        .unwrap();
        assert_eq!(v[0].description.as_deref(), Some("Sincroniza"));
        assert_eq!(v[0].updated_at.as_deref(), Some("2026-10-31T12:00:00Z"));
        assert!(v[1].description.is_none() && v[1].updated_at.is_none());
    }
}
