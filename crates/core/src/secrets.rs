//! Preventive credential detection, run on the content a push would publish.

use crate::error::Result;
use crate::git::{os_args, Git, RunOpts};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// High-confidence credential: never pushed; the content must be removed
    /// or ignored.
    Blocking,
    /// Likely sensitive: pushed only after the user confirms this finding.
    Warning,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Blocking => "blocking",
            Severity::Warning => "warning",
        }
    }
    pub fn parse(s: &str) -> Severity {
        if s == "blocking" { Severity::Blocking } else { Severity::Warning }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub path: String,
    pub line: Option<usize>,
    pub rule: String,
    pub severity: Severity,
    /// Redacted excerpt; the secret itself is never stored.
    pub excerpt: String,
    /// Stable id: sha256(path, rule, secret).
    pub fingerprint: String,
}

struct Rule {
    id: &'static str,
    severity: Severity,
    re: Regex,
}

fn rules() -> &'static [Rule] {
    static R: OnceLock<Vec<Rule>> = OnceLock::new();
    R.get_or_init(|| {
        let r = |id, severity, pat: &str| Rule { id, severity, re: Regex::new(pat).expect("valid regex") };
        use Severity::*;
        vec![
            r("private-key", Blocking, r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY(?: BLOCK)?-----"),
            r("aws-access-key-id", Blocking, r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
            r("aws-secret-access-key", Blocking, r#"(?i)aws_?secret_?access_?key\s*[:=]\s*['"]?[A-Za-z0-9/+=]{40}\b"#),
            r("github-token", Blocking, r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b"),
            r("github-fine-grained-token", Blocking, r"\bgithub_pat_[A-Za-z0-9_]{22,}\b"),
            r("gitlab-token", Blocking, r"\bglpat-[A-Za-z0-9_\-]{20,}\b"),
            r("slack-token", Blocking, r"\bxox[abposr]-[A-Za-z0-9-]{10,}"),
            r("stripe-live-key", Blocking, r"\b(?:sk|rk)_live_[A-Za-z0-9]{20,}\b"),
            r("anthropic-api-key", Blocking, r"\bsk-ant-[A-Za-z0-9_\-]{20,}"),
            r("openai-api-key", Blocking, r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{32,}"),
            r("npm-token", Blocking, r"\bnpm_[A-Za-z0-9]{36}\b"),
            r("google-api-key", Warning, r"\bAIza[0-9A-Za-z_\-]{35}\b"),
            r("jwt", Warning, r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
            r(
                "generic-secret-assignment",
                Warning,
                r#"(?i)\b(?:password|passwd|pwd|secret|api[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret)\b["']?\s*[:=]\s*["'][^"'\s]{8,}["']"#,
            ),
            r(
                "credentials-in-url",
                Warning,
                r"(?i)\b(?:postgres|postgresql|mysql|mongodb(?:\+srv)?|redis|amqp|https?)://[^\s:/@]+:[^\s@/]{3,}@",
            ),
        ]
    })
}

const MAX_SCAN_BYTES: u64 = 1024 * 1024;
pub const LARGE_FILE_WARN: u64 = 50 * 1024 * 1024;
pub const LARGE_FILE_BLOCK: u64 = 100 * 1024 * 1024;

fn file_name_rule(path: &str) -> Option<(&'static str, Severity)> {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let blocking = ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519", ".npmrc", ".pypirc", ".netrc", "_netrc", ".git-credentials"];
    if blocking.contains(&name.as_str()) {
        return Some(("sensitive-file", Severity::Blocking));
    }
    if name == ".env" || name.starts_with(".env.") && !["example", "sample", "template", "dist", "defaults"].iter().any(|s| name.ends_with(s)) {
        return Some(("env-file", Severity::Warning));
    }
    for ext in [".pem", ".key", ".p12", ".pfx", ".keystore", ".jks", ".kdbx", ".ppk"] {
        if name.ends_with(ext) {
            return Some(("key-material-file", Severity::Warning));
        }
    }
    if name == "credentials.json" || name == "service-account.json" || name.ends_with(".tfstate") {
        return Some(("credentials-file", Severity::Warning));
    }
    None
}

fn redact(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len());
    }
    let head: String = chars[..4].iter().collect();
    format!("{head}…({} caracteres)", chars.len())
}

fn fingerprint(path: &str, rule: &str, secret: &str) -> String {
    crate::util::sha256_hex(format!("{path}\0{rule}\0{secret}").as_bytes())[..32].to_string()
}

/// Scan one blob's text content.
pub fn scan_text(path: &str, content: &[u8]) -> Vec<Finding> {
    let mut out = Vec::new();
    if content.iter().take(8000).any(|b| *b == 0) {
        return out; // binary
    }
    let text = String::from_utf8_lossy(content);
    for (i, line) in text.lines().enumerate() {
        if line.len() > 4000 {
            continue; // minified bundles: too noisy
        }
        for rule in rules() {
            for m in rule.re.find_iter(line) {
                out.push(Finding {
                    path: path.to_string(),
                    line: Some(i + 1),
                    rule: rule.id.to_string(),
                    severity: rule.severity,
                    excerpt: redact(m.as_str()),
                    fingerprint: fingerprint(path, rule.id, m.as_str()),
                });
            }
        }
    }
    out
}

/// Scan every blob that differs between `base_tree` and `new_tree`.
pub fn scan_trees(git: &Git, base_tree: &str, new_tree: &str) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for d in git.diff_trees(base_tree, new_tree)? {
        let Some(new) = d.new() else { continue };
        if new.mode == crate::apply::MODE_GITLINK || new.mode == crate::apply::MODE_SYMLINK {
            continue;
        }
        if let Some((rule, sev)) = file_name_rule(&d.path) {
            findings.push(Finding {
                path: d.path.clone(),
                line: None,
                rule: rule.into(),
                severity: sev,
                excerpt: d.path.rsplit('/').next().unwrap_or("").into(),
                fingerprint: fingerprint(&d.path, rule, &new.oid),
            });
        }
        let size = git.object_size(&new.oid)?;
        if size >= LARGE_FILE_WARN {
            let sev = if size >= LARGE_FILE_BLOCK { Severity::Blocking } else { Severity::Warning };
            findings.push(Finding {
                path: d.path.clone(),
                line: None,
                rule: "large-file".into(),
                severity: sev,
                excerpt: format!("{:.1} MB", size as f64 / 1_048_576.0),
                fingerprint: fingerprint(&d.path, "large-file", &new.oid),
            });
            continue;
        }
        if size > MAX_SCAN_BYTES {
            continue;
        }
        let blob = git.run(os_args!["cat-file", "blob", new.oid.as_str()], RunOpts::default())?.stdout;
        findings.extend(scan_text(&d.path, &blob));
    }
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_credentials() {
        let key = format!("ghp_{}", "a1B2".repeat(9));
        let f = scan_text("cfg.yml", format!("token: {key}\n").as_bytes());
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "github-token");
        assert_eq!(f[0].severity, Severity::Blocking);
        assert!(!f[0].excerpt.contains(&key[6..]));

        let f = scan_text("k", b"-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n");
        assert_eq!(f[0].rule, "private-key");

        let f = scan_text("a.py", b"password = \"hunter2hunter2\"\n");
        assert_eq!(f[0].severity, Severity::Warning);

        let f = scan_text("a.rs", b"let x = 1; // nothing here\n");
        assert!(f.is_empty());
        assert!(scan_text("bin", b"AKIA\0AAAAAAAAAAAAAAAA").is_empty());
    }

    #[test]
    fn fingerprints_are_stable() {
        let a = scan_text("x", b"AKIAABCDEFGHIJKLMNOP");
        let b = scan_text("x", b"  AKIAABCDEFGHIJKLMNOP  ");
        assert_eq!(a[0].fingerprint, b[0].fingerprint);
    }

    #[test]
    fn file_name_rules() {
        assert_eq!(file_name_rule("home/.ssh/id_rsa").unwrap().1, Severity::Blocking);
        assert_eq!(file_name_rule(".env").unwrap().1, Severity::Warning);
        assert!(file_name_rule(".env.example").is_none());
        assert!(file_name_rule("src/env.rs").is_none());
    }
}
