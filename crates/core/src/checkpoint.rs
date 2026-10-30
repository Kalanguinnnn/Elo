//! Checkpoint identity, ref naming and commit-message metadata.

use crate::error::{Error, Result};
use crate::util;
use serde::{Deserialize, Serialize};

pub const REF_PREFIX: &str = "refs/continuity/checkpoints/";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Manual,
    Timer,
    Watcher,
    Shutdown,
    PreApply,
    Merge,
    PreRestore,
}

impl Trigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            Trigger::Manual => "manual",
            Trigger::Timer => "timer",
            Trigger::Watcher => "watcher",
            Trigger::Shutdown => "shutdown",
            Trigger::PreApply => "pre_apply",
            Trigger::Merge => "merge",
            Trigger::PreRestore => "pre_restore",
        }
    }
    pub fn parse(s: &str) -> Trigger {
        match s {
            "timer" => Trigger::Timer,
            "watcher" => Trigger::Watcher,
            "shutdown" => Trigger::Shutdown,
            "pre_apply" => Trigger::PreApply,
            "merge" => Trigger::Merge,
            "pre_restore" => Trigger::PreRestore,
            _ => Trigger::Manual,
        }
    }
}

/// Metadata carried in the checkpoint commit message as `Continuity-*` trailers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointMeta {
    pub id: String,
    pub device_id: String,
    pub device_name: String,
    /// Full ref of the branch (`refs/heads/main`), `None` when detached.
    pub branch: Option<String>,
    pub head: Option<String>,
    /// Commit whose tree is the staging area at capture time.
    pub index_commit: String,
    pub created_ms: i64,
    pub trigger: Trigger,
    /// Lineage parents (checkpoint ids).
    pub parents: Vec<String>,
    pub summary: String,
}

/// `<unix_ms:013>-<6 hex>`: lexicographic order == chronological order.
pub fn new_id(ms: i64) -> String {
    format!("{:013}-{}", ms, util::random_hex(3))
}

pub fn is_valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 20
        && b[..13].iter().all(u8::is_ascii_digit)
        && b[13] == b'-'
        && b[14..].iter().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

pub fn is_valid_device_id(d: &str) -> bool {
    uuid::Uuid::parse_str(d).is_ok() && d.len() == 36 && d == d.to_ascii_lowercase()
}

pub fn id_time_ms(id: &str) -> i64 {
    id.get(..13).and_then(|s| s.parse().ok()).unwrap_or(0)
}

pub fn ref_name(device_id: &str, id: &str) -> String {
    format!("{REF_PREFIX}{device_id}/{id}")
}

/// Parse and strictly validate a ref from our namespace. Anything else
/// (including refs a hostile remote may have planted) is ignored.
pub fn parse_ref(refname: &str) -> Option<(String, String)> {
    let rest = refname.strip_prefix(REF_PREFIX)?;
    let (dev, id) = rest.split_once('/')?;
    (is_valid_device_id(dev) && is_valid_id(id)).then(|| (dev.to_string(), id.to_string()))
}

impl CheckpointMeta {
    pub fn to_message(&self) -> String {
        let mut m = format!("Continuity checkpoint {}\n\n", self.id);
        if !self.summary.is_empty() {
            m.push_str(&util::sanitize_line(&self.summary, 200));
            m.push_str("\n\n");
        }
        let kv = |k: &str, v: &str| format!("Continuity-{k}: {v}\n");
        m.push_str(&kv("Version", &FORMAT_VERSION.to_string()));
        m.push_str(&kv("Id", &self.id));
        m.push_str(&kv("Device", &self.device_id));
        m.push_str(&kv("Device-Name", &util::sanitize_line(&self.device_name, 80)));
        m.push_str(&kv("Branch", self.branch.as_deref().unwrap_or("")));
        m.push_str(&kv("Head", self.head.as_deref().unwrap_or("")));
        m.push_str(&kv("Index", &self.index_commit));
        m.push_str(&kv("Created", &util::ts_to_rfc3339(self.created_ms)));
        m.push_str(&kv("Created-Ms", &self.created_ms.to_string()));
        m.push_str(&kv("Trigger", self.trigger.as_str()));
        m.push_str(&kv("Parents", &self.parents.join(" ")));
        m
    }

    pub fn parse_message(msg: &str) -> Result<CheckpointMeta> {
        let mut get = std::collections::HashMap::new();
        for line in msg.lines() {
            if let Some(rest) = line.strip_prefix("Continuity-")
                && let Some((k, v)) = rest.split_once(':') {
                    get.insert(k.trim().to_string(), v.trim().to_string());
                }
        }
        let field = |k: &str| get.get(k).cloned().unwrap_or_default();
        let id = field("Id");
        let device_id = field("Device");
        if !is_valid_id(&id) || !is_valid_device_id(&device_id) {
            return Err(Error::Invalid("metadados de checkpoint inválidos".into()));
        }
        let opt = |s: String| (!s.is_empty()).then_some(s);
        let branch = opt(field("Branch")).filter(|b| b.starts_with("refs/heads/") && crate::git::validate_ref_component(&b["refs/heads/".len()..]).is_ok());
        let head = opt(field("Head")).filter(|h| is_hex_oid(h));
        let index_commit = field("Index");
        if !is_hex_oid(&index_commit) {
            return Err(Error::Invalid("commit de índice inválido".into()));
        }
        let parents = field("Parents").split_whitespace().filter(|p| is_valid_id(p)).map(String::from).collect();
        let summary = msg.lines().nth(2).filter(|l| !l.starts_with("Continuity-")).unwrap_or("").to_string();
        Ok(CheckpointMeta {
            created_ms: field("Created-Ms").parse().unwrap_or_else(|_| id_time_ms(&id)),
            id,
            device_id,
            device_name: util::sanitize_line(&field("Device-Name"), 80),
            branch,
            head,
            index_commit,
            trigger: Trigger::parse(&field("Trigger")),
            parents,
            summary,
        })
    }
}

pub fn is_hex_oid(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|c| c.is_ascii_hexdigit())
}

pub fn short_branch(full: &str) -> &str {
    full.strip_prefix("refs/heads/").unwrap_or(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sort_chronologically_and_validate() {
        let a = new_id(1_700_000_000_000);
        let b = new_id(1_700_000_000_001);
        assert!(a < b);
        assert!(is_valid_id(&a));
        assert!(!is_valid_id("../../etc"));
        assert_eq!(id_time_ms(&a), 1_700_000_000_000);
    }

    #[test]
    fn message_roundtrip() {
        let m = CheckpointMeta {
            id: new_id(1_700_000_000_000),
            device_id: uuid::Uuid::new_v4().to_string(),
            device_name: "Laptop\nInjected: x".into(),
            branch: Some("refs/heads/feature/x".into()),
            head: Some("a".repeat(40)),
            index_commit: "b".repeat(40),
            created_ms: 1_700_000_000_000,
            trigger: Trigger::Timer,
            parents: vec![new_id(1_600_000_000_000)],
            summary: "3 arquivos".into(),
        };
        let parsed = CheckpointMeta::parse_message(&m.to_message()).unwrap();
        assert_eq!(parsed.id, m.id);
        assert_eq!(parsed.branch, m.branch);
        assert_eq!(parsed.parents, m.parents);
        assert_eq!(parsed.device_name, "LaptopInjected: x");
        assert_eq!(parsed.trigger, Trigger::Timer);
    }

    #[test]
    fn ref_parsing_is_strict() {
        let dev = uuid::Uuid::new_v4().to_string();
        let id = new_id(1);
        assert_eq!(parse_ref(&ref_name(&dev, &id)), Some((dev.clone(), id.clone())));
        assert!(parse_ref(&format!("{REF_PREFIX}{dev}/../x")).is_none());
        assert!(parse_ref("refs/heads/main").is_none());
    }
}
