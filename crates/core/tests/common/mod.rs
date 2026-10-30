#![allow(dead_code)]

use continuity_core::credentials::MemoryStore;
use continuity_core::engine::{AddProjectOptions, Engine, EngineConfig};
use continuity_core::git::Git;
use continuity_core::ide::FixedProbe;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;

/// Run git with a fixed identity; panics on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env("LC_ALL", "C")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

pub fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git").current_dir(dir).args(args).output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn write(root: &Path, rel: &str, content: impl AsRef<[u8]>) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

pub fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

pub fn exists(root: &Path, rel: &str) -> bool {
    root.join(rel).exists()
}

/// Deterministic local config for test repos.
pub fn configure(dir: &Path) {
    git(dir, &["config", "core.autocrlf", "false"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

pub struct Fixture {
    pub _tmp: TempDir,
    pub remote: PathBuf,
    pub a: PathBuf,
    pub b: PathBuf,
    pub data_a: PathBuf,
    pub data_b: PathBuf,
}

/// Bare remote with one commit on `main`, cloned twice (A and B).
pub fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let remote = root.join("remote.git");
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "--bare", "-b", "main"]);
    let seed = root.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git(&seed, &["init", "-b", "main"]);
    configure(&seed);
    write(&seed, "README.md", "# demo\n");
    write(&seed, "src/lib.rs", "pub fn a() {}\n");
    write(&seed, ".gitignore", "target/\n*.log\n");
    write(&seed, "old.txt", "old\n");
    git(&seed, &["add", "-A"]);
    git(&seed, &["commit", "-m", "init"]);
    git(&seed, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&seed, &["push", "origin", "main"]);
    let a = root.join("a");
    let b = root.join("b");
    for dst in [&a, &b] {
        git(&root, &["clone", "-c", "core.autocrlf=false", remote.to_str().unwrap(), dst.to_str().unwrap()]);
    }
    configure(&a);
    configure(&b);
    Fixture { remote, a, b, data_a: root.join("data-a"), data_b: root.join("data-b"), _tmp: tmp }
}

pub fn engine(data: &Path, ide_safe: bool) -> Engine {
    Engine::open(EngineConfig {
        data_dir: data.to_path_buf(),
        askpass_exe: None,
        secrets: Arc::new(MemoryStore::default()),
        ide_probe: Arc::new(FixedProbe(ide_safe)),
    })
    .unwrap()
}

pub fn add(e: &Engine, path: &Path) -> String {
    e.add_project(&AddProjectOptions { path: path.to_str().unwrap().into(), ..Default::default() }).unwrap().id
}

pub fn open(path: &Path) -> Git {
    Git::open(path).unwrap()
}

pub fn file_hash(path: &Path) -> String {
    continuity_core::util::sha256_hex(&std::fs::read(path).unwrap())
}

pub fn remote_refs(remote: &Path) -> Vec<String> {
    git(remote, &["for-each-ref", "--format=%(refname)", "refs/continuity/"])
        .lines()
        .map(String::from)
        .filter(|l| !l.is_empty())
        .collect()
}
