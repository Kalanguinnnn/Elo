//! Preventive credential detection, run on every object a push would publish.
//!
//! The scan is object-based: `git rev-list --objects <tips> --not <known>`
//! lists every blob reachable from what is about to be pushed and not already
//! on the remote — the worktree tree, the staging-area commit, intermediate
//! commits and branch history alike. Nothing is skipped silently: content up
//! to [`LARGE_FILE_WARN`] is fully inspected, and anything larger produces a
//! finding that needs an explicit decision.

use crate::error::{Error, Result};
use crate::git::{os_args, Git, RunOpts};
use regex::bytes::Regex;
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
    /// Generic heuristics are too noisy on minified/one-line files.
    generic: bool,
    re: Regex,
}

fn rules() -> &'static [Rule] {
    static R: OnceLock<Vec<Rule>> = OnceLock::new();
    R.get_or_init(|| {
        let r = |id, severity, generic, pat: &str| Rule { id, severity, generic, re: Regex::new(pat).expect("valid regex") };
        use Severity::*;
        vec![
            r("private-key", Blocking, false, r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY(?: BLOCK)?-----"),
            r("aws-access-key-id", Blocking, false, r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
            r("aws-secret-access-key", Blocking, false, r#"(?i)aws_?secret_?access_?key\s*[:=]\s*['"]?[A-Za-z0-9/+=]{40}\b"#),
            r("github-token", Blocking, false, r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b"),
            r("github-fine-grained-token", Blocking, false, r"\bgithub_pat_[A-Za-z0-9_]{22,}\b"),
            r("gitlab-token", Blocking, false, r"\bglpat-[A-Za-z0-9_\-]{20,}\b"),
            r("slack-token", Blocking, false, r"\bxox[abposr]-[A-Za-z0-9-]{10,}"),
            r("stripe-live-key", Blocking, false, r"\b(?:sk|rk)_live_[A-Za-z0-9]{20,}\b"),
            r("anthropic-api-key", Blocking, false, r"\bsk-ant-[A-Za-z0-9_\-]{20,}"),
            r("openai-api-key", Blocking, false, r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{32,}"),
            r("npm-token", Blocking, false, r"\bnpm_[A-Za-z0-9]{36}\b"),
            r("google-api-key", Warning, false, r"\bAIza[0-9A-Za-z_\-]{35}\b"),
            r("jwt", Warning, true, r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
            r(
                "generic-secret-assignment",
                Warning,
                true,
                r#"(?i)\b(?:password|passwd|pwd|secret|api[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret)\b["']?\s*[:=]\s*["'][^"'\s]{8,}["']"#,
            ),
            r(
                "credentials-in-url",
                Warning,
                false,
                r"(?i)\b(?:postgres|postgresql|mysql|mongodb(?:\+srv)?|redis|amqp|https?)://[^\s:/@]+:[^\s@/]{3,}@",
            ),
        ]
    })
}

/// Lines longer than this skip only the *generic* heuristics.
const LONG_LINE: usize = 4000;
/// Content up to this size is fully scanned; above it a finding is raised.
pub const LARGE_FILE_WARN: u64 = 50 * 1024 * 1024;
pub const LARGE_FILE_BLOCK: u64 = 100 * 1024 * 1024;
/// Memory budget per `cat-file --batch` call.
const BATCH_BYTES: u64 = 64 * 1024 * 1024;

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

fn fingerprint(path: &str, rule: &str, secret: &[u8]) -> String {
    let mut data = format!("{path}\0{rule}\0").into_bytes();
    data.extend_from_slice(secret);
    crate::util::sha256_hex(&data)[..32].to_string()
}

/// Scan one blob's content. Binary content is checked against the
/// high-confidence (non-generic) rules only.
pub fn scan_bytes(path: &str, content: &[u8]) -> Vec<Finding> {
    let binary = content.iter().take(8000).any(|b| *b == 0);
    let mut out = Vec::new();
    for rule in rules() {
        if binary && rule.generic {
            continue;
        }
        for m in rule.re.find_iter(content) {
            let line_start = content[..m.start()].iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0);
            let line_end = content[m.end()..].iter().position(|b| *b == b'\n').map(|i| m.end() + i).unwrap_or(content.len());
            if rule.generic && line_end - line_start > LONG_LINE {
                continue;
            }
            let line = (!binary).then(|| content[..m.start()].iter().filter(|b| **b == b'\n').count() + 1);
            out.push(Finding {
                path: path.to_string(),
                line,
                rule: rule.id.to_string(),
                severity: rule.severity,
                excerpt: redact(&String::from_utf8_lossy(m.as_bytes())),
                fingerprint: fingerprint(path, rule.id, m.as_bytes()),
            });
        }
    }
    out
}

/// Text convenience wrapper (tests, callers holding a string).
pub fn scan_text(path: &str, content: &[u8]) -> Vec<Finding> {
    scan_bytes(path, content)
}

struct BlobRef {
    oid: String,
    path: String,
    size: u64,
}

/// Every blob reachable from `tips` but not from `exclude` (commits already
/// on the remote), with the first path under which it appears.
fn new_blobs(git: &Git, tips: &[String], exclude: &[String]) -> Result<Vec<BlobRef>> {
    let mut input = String::new();
    for t in tips {
        input.push_str(t);
        input.push('\n');
    }
    for e in exclude {
        input.push('^');
        input.push_str(e);
        input.push('\n');
    }
    let listed = git.run(os_args!["rev-list", "--objects", "--stdin"], RunOpts::default().stdin(input.into_bytes()))?;
    let mut candidates: Vec<(String, String)> = Vec::new();
    for line in String::from_utf8_lossy(&listed.stdout).lines() {
        if let Some((oid, path)) = line.split_once(' ')
            && !path.is_empty()
        {
            candidates.push((oid.to_string(), path.to_string()));
        }
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut q = String::new();
    for (oid, _) in &candidates {
        q.push_str(oid);
        q.push('\n');
    }
    let checked = git.run(
        os_args!["cat-file", "--batch-check=%(objectname) %(objecttype) %(objectsize)"],
        RunOpts::default().stdin(q.into_bytes()),
    )?;
    let mut out = Vec::new();
    for ((oid, path), line) in candidates.into_iter().zip(String::from_utf8_lossy(&checked.stdout).lines()) {
        let f: Vec<&str> = line.split(' ').collect();
        if f.len() == 3 && f[0] == oid && f[1] == "blob" {
            out.push(BlobRef { oid, path, size: f[2].parse().unwrap_or(0) });
        }
    }
    Ok(out)
}

/// Read blobs in bounded batches through one `git cat-file --batch` per batch.
fn read_blobs(git: &Git, blobs: &[&BlobRef], mut f: impl FnMut(&BlobRef, &[u8])) -> Result<()> {
    let mut start = 0;
    while start < blobs.len() {
        let mut end = start;
        let mut budget = 0;
        while end < blobs.len() && (end == start || budget + blobs[end].size <= BATCH_BYTES) {
            budget += blobs[end].size;
            end += 1;
        }
        let mut q = String::new();
        for b in &blobs[start..end] {
            q.push_str(&b.oid);
            q.push('\n');
        }
        let out = git.run(os_args!["cat-file", "--batch"], RunOpts::default().stdin(q.into_bytes()))?.stdout;
        let mut pos = 0;
        for b in &blobs[start..end] {
            let nl = out[pos..].iter().position(|c| *c == b'\n').ok_or_else(|| Error::Other("cat-file truncado".into()))?;
            let header = String::from_utf8_lossy(&out[pos..pos + nl]).into_owned();
            pos += nl + 1;
            let size: usize = header.rsplit(' ').next().and_then(|s| s.parse().ok()).unwrap_or(0);
            if !header.starts_with(&b.oid) || pos + size > out.len() {
                return Err(Error::Other("resposta inesperada do cat-file".into()));
            }
            f(b, &out[pos..pos + size]);
            pos += size + 1; // content + trailing newline
        }
        start = end;
    }
    Ok(())
}

/// Scan every blob that pushing `tips` would publish, given the commits the
/// remote already has (`exclude`).
pub fn scan_new_objects(git: &Git, tips: &[String], exclude: &[String]) -> Result<Vec<Finding>> {
    let blobs = new_blobs(git, tips, exclude)?;
    let mut findings = Vec::new();
    let mut to_read = Vec::new();
    for b in &blobs {
        if let Some((rule, sev)) = file_name_rule(&b.path) {
            findings.push(Finding {
                path: b.path.clone(),
                line: None,
                rule: rule.into(),
                severity: sev,
                excerpt: b.path.rsplit('/').next().unwrap_or("").into(),
                fingerprint: fingerprint(&b.path, rule, b.oid.as_bytes()),
            });
        }
        if b.size > LARGE_FILE_WARN {
            // Not inspected: never treated as clean.
            let sev = if b.size >= LARGE_FILE_BLOCK { Severity::Blocking } else { Severity::Warning };
            findings.push(Finding {
                path: b.path.clone(),
                line: None,
                rule: "large-file-not-scanned".into(),
                severity: sev,
                excerpt: format!("{:.1} MB", b.size as f64 / 1_048_576.0),
                fingerprint: fingerprint(&b.path, "large-file-not-scanned", b.oid.as_bytes()),
            });
        } else {
            to_read.push(b);
        }
    }
    read_blobs(git, &to_read, |b, content| findings.extend(scan_bytes(&b.path, content)))?;
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    findings.dedup_by(|a, b| a.fingerprint == b.fingerprint);
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
    }

    #[test]
    fn long_lines_and_binary_content_are_still_scanned() {
        // Audit finding 4: a token after byte 4000 of a single line.
        let key = format!("ghp_{}", "Zz09".repeat(9));
        let mut minified = "x".repeat(10_000);
        minified.push_str(&format!(";t=\"{key}\";"));
        let f = scan_text("bundle.min.js", minified.as_bytes());
        assert!(f.iter().any(|x| x.rule == "github-token"), "{f:?}");
        // Generic heuristics stay off on such lines (noise), specific rules stay on.
        let noisy = format!("{}password=\"abcdefghij\"", "y".repeat(5000));
        assert!(scan_text("x.js", noisy.as_bytes()).is_empty());
        // Binary blob with an embedded key.
        let mut bin = vec![0u8; 100];
        bin.extend_from_slice(b"AKIAABCDEFGHIJKLMNOP");
        assert_eq!(scan_bytes("data.bin", &bin)[0].rule, "aws-access-key-id");
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
