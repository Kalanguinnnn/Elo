mod common;

use common::*;
use continuity_core::checkpoint::{self, CheckpointMeta, Trigger};
use continuity_core::snapshot;
use std::collections::HashMap;

fn meta(id: &str) -> CheckpointMeta {
    CheckpointMeta {
        id: id.into(),
        device_id: uuid::Uuid::new_v4().to_string(),
        device_name: "test".into(),
        branch: None,
        head: None,
        index_commit: String::new(),
        created_ms: 1,
        trigger: Trigger::Manual,
        parents: vec![],
        summary: String::new(),
    }
}

fn tree_files(g: &continuity_core::git::Git, tree: &str) -> HashMap<String, String> {
    g.ls_tree(tree)
        .unwrap()
        .into_iter()
        .map(|(p, e)| (p, String::from_utf8(g.blob_raw(&e.oid).unwrap()).unwrap_or_default()))
        .collect()
}

#[test]
fn capture_preserves_index_head_branch_and_covers_all_change_kinds() {
    let f = fixture();
    let a = &f.a;
    // staged change, unstaged change on another file, partial staging on a third
    write(a, "src/lib.rs", "pub fn a() { /* staged */ }\n");
    git(a, &["add", "src/lib.rs"]);
    write(a, "src/lib.rs", "pub fn a() { /* staged then edited */ }\n");
    write(a, "README.md", "# demo\nunstaged\n");
    write(a, "new/untracked.txt", "fresh\n");
    write(a, "debug.log", "ignored\n");
    write(a, "target/out.bin", "ignored dir\n");
    std::fs::remove_file(a.join("old.txt")).unwrap();
    git(a, &["mv", "src/lib.rs", "src/core.rs"]);

    let index_before = std::fs::read(a.join(".git/index")).unwrap();
    let status_before = git(a, &["status", "--porcelain=v2", "--untracked-files=all"]);
    let head_before = git(a, &["rev-parse", "HEAD"]);
    let branch_before = git(a, &["symbolic-ref", "HEAD"]);
    let cached_before = git(a, &["diff", "--cached", "--name-status"]);

    let g = open(a);
    let state = snapshot::capture(&g).unwrap();
    let mut m = meta(&checkpoint::new_id(1_700_000_000_000));
    snapshot::write_checkpoint(&g, &state, &mut m).unwrap();

    assert_eq!(std::fs::read(a.join(".git/index")).unwrap(), index_before, "index file must be byte-identical");
    assert_eq!(git(a, &["status", "--porcelain=v2", "--untracked-files=all"]), status_before);
    assert_eq!(git(a, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(a, &["symbolic-ref", "HEAD"]), branch_before);
    assert_eq!(git(a, &["diff", "--cached", "--name-status"]), cached_before);
    assert_eq!(git(a, &["branch", "--list"]).lines().count(), 1, "no branch created");
    assert_eq!(git(a, &["log", "--oneline", "--branches"]).lines().count(), 1, "no commit on branches");

    let files = tree_files(&g, &state.tree);
    assert_eq!(files["src/core.rs"], "pub fn a() { /* staged then edited */ }\n");
    assert!(!files.contains_key("src/lib.rs"), "rename source gone");
    assert_eq!(files["README.md"], "# demo\nunstaged\n");
    assert_eq!(files["new/untracked.txt"], "fresh\n");
    assert!(!files.contains_key("old.txt"), "deletion captured");
    assert!(!files.contains_key("debug.log"), "ignored file excluded");
    assert!(!files.keys().any(|k| k.starts_with("target/")), "ignored dir excluded");

    let staged = tree_files(&g, &state.index_tree);
    assert_eq!(staged["src/core.rs"], "pub fn a() { /* staged */ }\n", "staging captured separately");
    assert_eq!(staged["README.md"], "# demo\n");
    assert!(!staged.contains_key("new/untracked.txt"));

    let rc = snapshot::list_local(&g).unwrap();
    assert_eq!(rc.len(), 1);
    assert_eq!(rc[0].meta.head.as_deref(), Some(head_before.as_str()));
    assert_eq!(rc[0].meta.branch.as_deref(), Some("refs/heads/main"));
    // Checkpoint refs never show up as branches.
    assert!(!git(a, &["branch", "-a"]).contains("continuity"));
}

#[test]
fn unborn_repository_without_commits() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    git(r, &["init", "-b", "main"]);
    configure(r);
    write(r, "a.txt", "x\n");
    let g = open(r);
    let st = snapshot::capture(&g).unwrap();
    assert!(st.head.is_none());
    assert_eq!(st.branch.as_deref(), Some("refs/heads/main"));
    let mut m = meta(&checkpoint::new_id(5));
    snapshot::write_checkpoint(&g, &st, &mut m).unwrap();
    assert!(!git_ok(r, &["rev-parse", "--verify", "HEAD"]), "still unborn");
    assert_eq!(tree_files(&g, &st.tree)["a.txt"], "x\n");
}

#[test]
fn refuses_while_git_holds_index_lock() {
    let f = fixture();
    std::fs::write(f.a.join(".git/index.lock"), b"").unwrap();
    let err = snapshot::capture(&open(&f.a)).unwrap_err();
    assert_eq!(err.code(), "busy");
}

#[test]
fn engine_dedups_unchanged_state_and_records_lineage() {
    let f = fixture();
    let e = engine(&f.data_a, true);
    let id = add(&e, &f.a);
    write(&f.a, "x.txt", "1\n");
    let c1 = e.checkpoint_now(&id, Trigger::Manual).unwrap().expect("first");
    assert!(e.checkpoint_now(&id, Trigger::Timer).unwrap().is_none(), "no duplicate without changes");
    write(&f.a, "x.txt", "2\n");
    let c2 = e.checkpoint_now(&id, Trigger::Timer).unwrap().expect("second");
    assert_eq!(c2.parents, vec![c1.id.clone()]);
    assert!(c2.id > c1.id);
    // staging-only change is a new state too
    git(&f.a, &["add", "x.txt"]);
    let c3 = e.checkpoint_now(&id, Trigger::Timer).unwrap().expect("staging change");
    assert_eq!(c3.tree_oid, c2.tree_oid);
    assert_ne!(c3.index_commit, c2.index_commit);
}

#[test]
fn binary_files_roundtrip_through_restore_with_integrity() {
    let f = fixture();
    let e = engine(&f.data_a, true);
    let id = add(&e, &f.a);
    let mut blob = Vec::new();
    let mut x: u32 = 12345;
    for _ in 0..300_000 {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
        blob.push((x >> 16) as u8);
    }
    blob[10] = 0;
    write(&f.a, "assets/image.bin", &blob);
    write(&f.a, "docs/ünïcødé name.txt", "texto\n");
    let c = e.checkpoint_now(&id, Trigger::Manual).unwrap().unwrap();
    let target = f.data_a.join("restored");
    let rep = e.restore_to_dir(&id, &c.id, target.to_str().unwrap()).unwrap();
    assert!(rep.integrity_failures.is_empty(), "{:?}", rep.integrity_failures);
    assert_eq!(rep.verified, rep.files_written);
    assert_eq!(std::fs::read(target.join("assets/image.bin")).unwrap(), blob);
    assert_eq!(read(&target, "docs/ünïcødé name.txt"), "texto\n");
    // restoring into a non-empty directory or inside the project is refused
    assert!(e.restore_to_dir(&id, &c.id, target.to_str().unwrap()).is_err());
    assert!(e.restore_to_dir(&id, &c.id, f.a.join("inside").to_str().unwrap()).is_err());
}

#[test]
fn restore_individual_file_in_place_keeps_safety_checkpoint() {
    let f = fixture();
    let e = engine(&f.data_a, true);
    let id = add(&e, &f.a);
    write(&f.a, "notes.md", "version 1\n");
    let c1 = e.checkpoint_now(&id, Trigger::Manual).unwrap().unwrap();
    write(&f.a, "notes.md", "version 2 (bad)\n");
    write(&f.a, "other.md", "untouched\n");
    let index_before = std::fs::read(f.a.join(".git/index")).unwrap();
    let out = e.restore_files(&id, &c1.id, &["notes.md".into()]).unwrap();
    assert_eq!(read(&f.a, "notes.md"), "version 1\n");
    assert_eq!(read(&f.a, "other.md"), "untouched\n");
    assert_eq!(std::fs::read(f.a.join(".git/index")).unwrap(), index_before);
    let continuity_core::engine::ApplyOutcome::Applied { result, safety_checkpoint, .. } = out else { panic!("{out:?}") };
    assert!(result.integrity_failures.is_empty());
    // the overwritten version is recoverable from the safety checkpoint
    let v = e
        .file_view(&id, &continuity_core::engine::Side::Checkpoint { id: safety_checkpoint }, "notes.md")
        .unwrap();
    assert_eq!(v.text.as_deref(), Some("version 2 (bad)\n"));
}
