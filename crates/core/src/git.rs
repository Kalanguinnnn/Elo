//! Thin, shell-free wrapper around the Git CLI.
//!
//! Every invocation goes through [`Git::run`], which builds a
//! `std::process::Command` with an explicit argument vector (no shell, no string
//! interpolation), scrubs inherited `GIT_*` variables that could redirect the
//! repository, forces `LC_ALL=C` so stderr can be classified, and disables
//! interactive prompts.

use crate::error::{Error, Result};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Variables that would make git operate on a different repository/index than
/// the one we resolved. They are removed from every child environment.
const SCRUBBED_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_CEILING_DIRECTORIES",
];

pub fn git_executable() -> OsString {
    std::env::var_os("CONTINUITY_GIT").unwrap_or_else(|| OsString::from("git"))
}

#[cfg(windows)]
pub fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
pub fn hide_console(_cmd: &mut Command) {}

#[derive(Default, Clone)]
pub struct RunOpts {
    pub env: Vec<(OsString, OsString)>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Option<Duration>,
    /// Exit codes (besides 0) that are not treated as errors.
    pub ok_codes: Vec<i32>,
    /// Extra `-c key=value` pairs placed before the subcommand.
    pub config: Vec<(String, String)>,
}

impl RunOpts {
    pub fn env(mut self, k: impl Into<OsString>, v: impl Into<OsString>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }
    pub fn stdin(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(data.into());
        self
    }
    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = Some(d);
        self
    }
    pub fn ok_code(mut self, c: i32) -> Self {
        self.ok_codes.push(c);
        self
    }
    pub fn config(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.config.push((k.into(), v.into()));
        self
    }
}

#[derive(Debug)]
pub struct Output {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Output {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim_end().to_string()
    }
}

/// A resolved, non-bare repository.
#[derive(Debug, Clone)]
pub struct Git {
    /// Absolute path of the working tree root.
    pub workdir: PathBuf,
    /// Absolute path of the (per-worktree) git directory.
    pub git_dir: PathBuf,
    /// Absolute path of the common git directory (refs/objects).
    pub common_dir: PathBuf,
}

/// Run git outside of any repository (e.g. `git clone`, `git init`, version).
pub fn run_global(cwd: &Path, args: &[&OsStr], opts: RunOpts) -> Result<Output> {
    run_raw(cwd, args, opts)
}

fn run_raw(cwd: &Path, args: &[&OsStr], opts: RunOpts) -> Result<Output> {
    let mut cmd = Command::new(git_executable());
    cmd.current_dir(cwd);
    for k in SCRUBBED_ENV {
        cmd.env_remove(k);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GCM_INTERACTIVE", "never");
    cmd.arg("-c").arg("core.quotepath=off");
    for (k, v) in &opts.config {
        cmd.arg("-c").arg(format!("{k}={v}"));
    }
    cmd.args(args);
    for (k, v) in &opts.env {
        cmd.env(k, v);
    }
    cmd.stdin(if opts.stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_console(&mut cmd);

    let label = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::GitMissing(e.to_string())
        } else {
            Error::Io(e)
        }
    })?;

    // Feed stdin and drain stdout/stderr on threads so large payloads cannot
    // deadlock on full pipe buffers.
    let stdin_thread = match (opts.stdin, child.stdin.take()) {
        (Some(data), Some(mut pipe)) => Some(std::thread::spawn(move || {
            let _ = pipe.write_all(&data);
        })),
        _ => None,
    };
    let mut out_pipe = child.stdout.take().expect("piped stdout");
    let mut err_pipe = child.stderr.take().expect("piped stderr");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });

    let status = match opts.timeout {
        None => child.wait()?,
        Some(limit) => {
            let start = Instant::now();
            loop {
                if let Some(st) = child.try_wait()? {
                    break st;
                }
                if start.elapsed() >= limit {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::Timeout(limit.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    };
    if let Some(t) = stdin_thread {
        let _ = t.join();
    }
    let stdout = out_thread.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err_thread.join().unwrap_or_default()).into_owned();
    let code = status.code().unwrap_or(-1);
    if code != 0 && !opts.ok_codes.contains(&code) {
        return Err(Error::Git {
            args: label,
            code: status.code(),
            stderr: stderr.trim().to_string(),
        });
    }
    Ok(Output { code, stdout, stderr })
}

macro_rules! os_args {
    ($($a:expr),* $(,)?) => { &[$(AsRef::<::std::ffi::OsStr>::as_ref(&$a)),*] };
}
pub(crate) use os_args;

impl Git {
    /// Resolve the repository containing `path`.
    pub fn open(path: &Path) -> Result<Git> {
        if !path.is_dir() {
            return Err(Error::NotFound(format!("diretório {}", path.display())));
        }
        let out = run_raw(
            path,
            os_args!["rev-parse", "--path-format=absolute", "--show-toplevel", "--git-dir", "--git-common-dir"],
            RunOpts::default(),
        )?;
        let text = out.text();
        let mut lines = text.lines();
        let (Some(top), Some(gd), Some(cd)) = (lines.next(), lines.next(), lines.next()) else {
            return Err(Error::Invalid(format!("{} não é um repositório Git com worktree", path.display())));
        };
        Ok(Git {
            workdir: normalize(PathBuf::from(top)),
            git_dir: normalize(PathBuf::from(gd)),
            common_dir: normalize(PathBuf::from(cd)),
        })
    }

    pub fn is_repo(path: &Path) -> bool {
        Git::open(path).is_ok()
    }

    /// `git init` a plain directory (only called after explicit user consent).
    pub fn init(path: &Path, initial_branch: &str) -> Result<Git> {
        validate_ref_component(initial_branch)?;
        run_raw(path, os_args!["init", "-b", initial_branch], RunOpts::default())?;
        Git::open(path)
    }

    pub fn run(&self, args: &[&OsStr], opts: RunOpts) -> Result<Output> {
        run_raw(&self.workdir, args, opts)
    }

    pub fn run_text(&self, args: &[&OsStr]) -> Result<String> {
        Ok(self.run(args, RunOpts::default())?.text())
    }

    /// Directory for Continuity's private, per-worktree scratch files.
    pub fn private_dir(&self) -> Result<PathBuf> {
        let d = self.git_dir.join("continuity");
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }

    pub fn head_commit(&self) -> Result<Option<String>> {
        let out = self.run(os_args!["rev-parse", "-q", "--verify", "HEAD^{commit}"], RunOpts::default().ok_code(1))?;
        Ok(if out.code == 0 { Some(out.text()) } else { None })
    }

    /// Full symbolic ref of HEAD (`refs/heads/x`), `None` when detached.
    pub fn head_branch(&self) -> Result<Option<String>> {
        let out = self.run(os_args!["symbolic-ref", "-q", "HEAD"], RunOpts::default().ok_code(1))?;
        Ok(if out.code == 0 { Some(out.text()) } else { None })
    }

    pub fn resolve(&self, rev: &str) -> Result<Option<String>> {
        let spec = format!("{rev}^{{object}}");
        let out = self.run(os_args!["rev-parse", "-q", "--verify", "--end-of-options", spec], RunOpts::default().ok_code(1))?;
        Ok(if out.code == 0 { Some(out.text()) } else { None })
    }

    pub fn ref_value(&self, refname: &str) -> Result<Option<String>> {
        let out = self.run(os_args!["rev-parse", "-q", "--verify", "--end-of-options", refname], RunOpts::default().ok_code(1))?;
        Ok(if out.code == 0 { Some(out.text()) } else { None })
    }

    pub fn has_object(&self, oid: &str) -> bool {
        self.run(os_args!["cat-file", "-e", oid], RunOpts::default()).is_ok()
    }

    pub fn tree_of(&self, commit: &str) -> Result<String> {
        self.run_text(os_args!["rev-parse", "--verify", format!("{commit}^{{tree}}")])
    }

    pub fn empty_tree(&self) -> Result<String> {
        // Works for both SHA-1 and SHA-256 repositories.
        Ok(self.run(os_args!["mktree"], RunOpts::default().stdin(Vec::new()))?.text())
    }

    pub fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        let out = self.run(os_args!["merge-base", "--is-ancestor", ancestor, descendant], RunOpts::default().ok_code(1))?;
        Ok(out.code == 0)
    }

    pub fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        let out = self.run(os_args!["merge-base", a, b], RunOpts::default().ok_code(1))?;
        Ok(if out.code == 0 { Some(out.text()) } else { None })
    }

    pub fn config_get(&self, key: &str) -> Result<Option<String>> {
        let out = self.run(os_args!["config", "--get", key], RunOpts::default().ok_code(1))?;
        Ok(if out.code == 0 { Some(out.text()) } else { None })
    }

    pub fn remotes(&self) -> Result<Vec<(String, String)>> {
        let text = self.run_text(os_args!["remote"])?;
        let mut v = Vec::new();
        for name in text.lines().filter(|l| !l.is_empty()) {
            let url = self.config_get(&format!("remote.{name}.url"))?.unwrap_or_default();
            v.push((name.to_string(), url));
        }
        Ok(v)
    }

    /// Create a ref that must not already exist.
    pub fn create_ref(&self, refname: &str, oid: &str) -> Result<()> {
        self.run(os_args!["update-ref", "--no-deref", refname, oid, ""], RunOpts::default())?;
        Ok(())
    }

    /// Compare-and-swap update of a ref (`old = None` means it must not exist).
    pub fn cas_ref(&self, refname: &str, new: &str, old: Option<&str>) -> Result<()> {
        let old = old.unwrap_or("");
        self.run(os_args!["update-ref", "--no-deref", refname, new, old], RunOpts::default())?;
        Ok(())
    }

    /// Delete a ref only if it still has the expected value.
    pub fn delete_ref(&self, refname: &str, expected: &str) -> Result<()> {
        self.run(os_args!["update-ref", "--no-deref", "-d", refname, expected], RunOpts::default())?;
        Ok(())
    }

    /// Blob content converted for the working tree (EOL, smudge filters).
    pub fn blob_for_worktree(&self, oid: &str, path: &str) -> Result<Vec<u8>> {
        Ok(self
            .run(os_args!["cat-file", "--filters", format!("--path={path}"), oid], RunOpts::default())?
            .stdout)
    }

    /// Raw blob content as stored in the repository.
    pub fn blob_raw(&self, oid: &str) -> Result<Vec<u8>> {
        Ok(self.run(os_args!["cat-file", "blob", oid], RunOpts::default())?.stdout)
    }

    pub fn object_size(&self, oid: &str) -> Result<u64> {
        self.run_text(os_args!["cat-file", "-s", oid])?
            .parse()
            .map_err(|_| Error::Other("tamanho de objeto inválido".into()))
    }

    /// Hash files (relative to `root`) the way `git add` would, applying clean
    /// filters. `git_dir` lets this run against a restore directory too.
    pub fn hash_paths(&self, root: &Path, paths: &[String]) -> Result<Vec<String>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut input = String::new();
        for p in paths {
            if p.contains('\n') {
                return Err(Error::UnsafePath(p.clone()));
            }
            input.push_str(p);
            input.push('\n');
        }
        let gd = self.git_dir.clone();
        let out = run_raw(
            root,
            os_args![format!("--git-dir={}", gd.display()), format!("--work-tree={}", root.display()), "hash-object", "--stdin-paths"],
            RunOpts::default().stdin(input.into_bytes()),
        )?;
        let text = out.text();
        let v: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        if v.len() != paths.len() {
            return Err(Error::Other("hash-object retornou contagem inesperada".into()));
        }
        Ok(v)
    }

    /// Entries of a tree, recursively: path -> (mode, oid).
    pub fn ls_tree(&self, tree: &str) -> Result<HashMap<String, TreeEntry>> {
        let out = self.run(os_args!["ls-tree", "-r", "-z", "--full-tree", tree], RunOpts::default())?;
        let mut map = HashMap::new();
        for rec in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let rec = String::from_utf8_lossy(rec);
            let Some((meta, path)) = rec.split_once('\t') else { continue };
            let mut it = meta.split(' ');
            let (Some(mode), Some(_kind), Some(oid)) = (it.next(), it.next(), it.next()) else { continue };
            map.insert(path.to_string(), TreeEntry { mode: mode.to_string(), oid: oid.to_string() });
        }
        Ok(map)
    }

    /// Changed paths between two trees (no rename detection).
    pub fn diff_trees(&self, a: &str, b: &str) -> Result<Vec<DiffEntry>> {
        let out = self.run(os_args!["diff-tree", "-r", "-z", "--no-renames", "--no-commit-id", a, b], RunOpts::default())?;
        Ok(parse_raw_diff(&out.stdout))
    }

    /// Changed paths with rename detection, for display only.
    pub fn diff_trees_display(&self, a: &str, b: &str) -> Result<Vec<DiffEntry>> {
        let out = self.run(os_args!["diff-tree", "-r", "-z", "-M", "--no-commit-id", a, b], RunOpts::default())?;
        Ok(parse_raw_diff(&out.stdout))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TreeEntry {
    pub mode: String,
    pub oid: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiffEntry {
    pub status: String,
    pub old_mode: String,
    pub new_mode: String,
    pub old_oid: String,
    pub new_oid: String,
    pub path: String,
    /// Source path for renames/copies.
    pub old_path: Option<String>,
}

impl DiffEntry {
    pub fn is_null_oid(oid: &str) -> bool {
        oid.bytes().all(|b| b == b'0')
    }
    pub fn old(&self) -> Option<TreeEntry> {
        (!Self::is_null_oid(&self.old_oid)).then(|| TreeEntry { mode: self.old_mode.clone(), oid: self.old_oid.clone() })
    }
    pub fn new(&self) -> Option<TreeEntry> {
        (!Self::is_null_oid(&self.new_oid)).then(|| TreeEntry { mode: self.new_mode.clone(), oid: self.new_oid.clone() })
    }
}

/// Parse `git diff-tree -z` raw output:
/// `:<srcmode> <dstmode> <srcoid> <dstoid> <status>\0<path>\0[<path2>\0]`.
fn parse_raw_diff(bytes: &[u8]) -> Vec<DiffEntry> {
    let mut parts = bytes.split(|b| *b == 0).map(|p| String::from_utf8_lossy(p).into_owned());
    let mut v = Vec::new();
    while let Some(meta) = parts.next() {
        let Some(meta) = meta.strip_prefix(':') else { continue };
        let f: Vec<&str> = meta.split(' ').collect();
        if f.len() < 5 {
            continue;
        }
        let status = f[4].to_string();
        let Some(p1) = parts.next() else { break };
        let (path, old_path) = if status.starts_with('R') || status.starts_with('C') {
            let p2 = parts.next().unwrap_or_default();
            (p2, Some(p1))
        } else {
            (p1, None)
        };
        v.push(DiffEntry {
            status: status.chars().next().map(String::from).unwrap_or_default(),
            old_mode: f[0].into(),
            new_mode: f[1].into(),
            old_oid: f[2].into(),
            new_oid: f[3].into(),
            path,
            old_path,
        });
    }
    v
}

/// Reject names that git would refuse or that could escape our namespace.
pub fn validate_ref_component(s: &str) -> Result<()> {
    let bad = s.is_empty()
        || s.starts_with('-')
        || s.starts_with('.')
        || s.ends_with('.')
        || s.ends_with(".lock")
        || s.contains("..")
        || s.contains("@{")
        || s.chars().any(|c| c.is_control() || " ~^:?*[\\".contains(c));
    if bad {
        Err(Error::Invalid(format!("nome de ref inválido: {s:?}")))
    } else {
        Ok(())
    }
}

fn normalize(p: PathBuf) -> PathBuf {
    // git prints forward slashes on Windows; keep a canonical-ish form without
    // resolving symlinks (canonicalize would add \\?\ prefixes on Windows).
    #[cfg(windows)]
    {
        PathBuf::from(p.to_string_lossy().replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raw_diff_with_renames() {
        let raw = b":100644 100644 aaaa bbbb M\0src/a.rs\0:000000 100644 0000 cccc A\0new file.txt\0:100644 100644 dddd dddd R100\0old.txt\0new.txt\0";
        let v = parse_raw_diff(raw);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].status, "M");
        assert_eq!(v[1].path, "new file.txt");
        assert!(v[1].old().is_none());
        assert_eq!(v[2].status, "R");
        assert_eq!(v[2].path, "new.txt");
        assert_eq!(v[2].old_path.as_deref(), Some("old.txt"));
    }

    #[test]
    fn ref_component_validation() {
        assert!(validate_ref_component("main").is_ok());
        assert!(validate_ref_component("feature/x").is_ok());
        for bad in ["", "-x", "a..b", "a b", "x.lock", "a:b", "a\nb", "a@{1}"] {
            assert!(validate_ref_component(bad).is_err(), "{bad}");
        }
    }
}
