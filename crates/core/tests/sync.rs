//! End-to-end: computer A → checkpoint → Git remote → computer B → protected
//! apply → continue working, plus conflicts, branches, offline and secrets.

mod common;

use common::*;
use continuity_core::apply::{ApplyOptions, Resolution};
use continuity_core::checkpoint::Trigger;
use continuity_core::db::ProjectPatch;
use continuity_core::engine::{ApplyOutcome, Engine};
use std::collections::HashMap;

fn push(e: &Engine, id: &str) {
    let r = e.push_pending(id).unwrap();
    assert!(r.quarantined.is_empty() && r.needs_review.is_empty(), "{r:?}");
}

fn receive(e: &Engine, id: &str) -> String {
    e.fetch(id).unwrap();
    let inc = e.incoming(id).unwrap();
    assert_eq!(inc.len(), 1, "exactly one incoming update expected: {inc:?}");
    inc[0].checkpoint.id.clone()
}

fn applied(o: ApplyOutcome) -> continuity_core::apply::ApplyResult {
    match o {
        ApplyOutcome::Applied { result, .. } => {
            assert!(result.integrity_failures.is_empty(), "{:?}", result.integrity_failures);
            result
        }
        other => panic!("expected Applied, got {other:?}"),
    }
}

#[test]
fn full_flow_a_to_b_and_back_without_manual_commits() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    assert_ne!(ea.device().id, eb.device().id);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);

    // A works: tracked edit, new untracked file, deletion, partial staging.
    write(&f.a, "src/lib.rs", "pub fn a() { 1 }\n");
    write(&f.a, "src/new.rs", "pub fn new() {}\n");
    std::fs::remove_file(f.a.join("old.txt")).unwrap();
    write(&f.a, "README.md", "# demo\nstaged line\n");
    git(&f.a, &["add", "README.md"]);
    write(&f.a, "local.log", "ignored, never synced\n");
    let a_cached = git(&f.a, &["diff", "--cached", "--name-only"]);
    let c = ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    assert!(remote_refs(&f.remote).iter().any(|r| r.ends_with(&c.id)));
    // the remote branch was not touched
    assert_eq!(git(&f.remote, &["rev-parse", "main"]), git(&f.a, &["rev-parse", "HEAD"]));

    // B receives and applies.
    let b_head_before = git(&f.b, &["rev-parse", "HEAD"]);
    let inc = receive(&eb, &pb);
    assert_eq!(inc, c.id);
    let preview = eb.plan_apply(&pb, &inc, &ApplyOptions::default()).unwrap();
    assert!(preview.plan.auto_eligible(), "{:?}", preview.plan);
    applied(eb.apply(&pb, &inc, &ApplyOptions::default(), false).unwrap());
    assert_eq!(read(&f.b, "src/lib.rs"), "pub fn a() { 1 }\n");
    assert_eq!(read(&f.b, "src/new.rs"), "pub fn new() {}\n");
    assert!(!exists(&f.b, "stale.txt"));
    assert!(!exists(&f.b, "local.log"));
    assert_eq!(git(&f.b, &["rev-parse", "HEAD"]), b_head_before);
    assert_eq!(git(&f.b, &["symbolic-ref", "HEAD"]), "refs/heads/main");
    assert_eq!(git(&f.b, &["diff", "--cached", "--name-only"]), a_cached, "staging transferred");
    assert!(eb.incoming(&pb).unwrap().is_empty());

    // B continues the work; A receives it as a fast-forward.
    write(&f.b, "src/new.rs", "pub fn new() { 2 }\n");
    eb.checkpoint_now(&pb, Trigger::Timer).unwrap().unwrap();
    push(&eb, &pb);
    let inc = receive(&ea, &pa);
    let preview = ea.plan_apply(&pa, &inc, &ApplyOptions::default()).unwrap();
    assert!(preview.plan.auto_eligible(), "A had no changes since its checkpoint: {:?}", preview.plan);
    applied(ea.apply(&pa, &inc, &ApplyOptions::default(), false).unwrap());
    assert_eq!(read(&f.a, "src/new.rs"), "pub fn new() { 2 }\n");
    assert_eq!(read(&f.a, "local.log"), "ignored, never synced\n", "ignored local file untouched");

    // And once more A → B: lineage keeps it a fast-forward.
    write(&f.a, "src/lib.rs", "pub fn a() { 3 }\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    let inc = receive(&eb, &pb);
    assert!(eb.plan_apply(&pb, &inc, &ApplyOptions::default()).unwrap().plan.auto_eligible());
    applied(eb.apply(&pb, &inc, &ApplyOptions::default(), false).unwrap());
    assert_eq!(read(&f.b, "src/lib.rs"), "pub fn a() { 3 }\n");
}

#[test]
fn commits_made_on_a_fast_forward_b_branch() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);
    write(&f.a, "feature.rs", "committed\n");
    git(&f.a, &["add", "feature.rs"]);
    git(&f.a, &["commit", "-m", "local commit, not pushed"]);
    write(&f.a, "wip.rs", "uncommitted\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    let inc = receive(&eb, &pb);
    let out = applied(eb.apply(&pb, &inc, &ApplyOptions::default(), false).unwrap());
    assert!(out.head_updated);
    assert_eq!(git(&f.b, &["rev-parse", "HEAD"]), git(&f.a, &["rev-parse", "HEAD"]));
    assert_eq!(read(&f.b, "wip.rs"), "uncommitted\n");
    assert_eq!(git(&f.b, &["status", "--porcelain"]), "?? wip.rs");
}

#[test]
fn divergent_edits_become_conflicts_and_nothing_is_overwritten() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);

    write(&f.a, "src/lib.rs", "A version\n");
    write(&f.a, "only_a.txt", "from A\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);

    write(&f.b, "src/lib.rs", "B version\n");
    write(&f.b, "only_b.txt", "from B\n");
    eb.checkpoint_now(&pb, Trigger::Timer).unwrap().unwrap();
    push(&eb, &pb);

    let inc = receive(&eb, &pb);
    let preview = eb.plan_apply(&pb, &inc, &ApplyOptions::default()).unwrap();
    assert!(!preview.auto_eligible);
    assert_eq!(preview.plan.conflicts.len(), 1);
    // Automatic mode refuses.
    assert!(matches!(eb.apply(&pb, &inc, &ApplyOptions::default(), true).unwrap(), ApplyOutcome::NeedsManual { .. }));
    let ApplyOutcome::Conflicts { conflict_id, count } = eb.apply(&pb, &inc, &ApplyOptions::default(), false).unwrap() else {
        panic!("expected conflicts")
    };
    assert_eq!(count, 1);
    // Nothing written yet.
    assert_eq!(read(&f.b, "src/lib.rs"), "B version\n");
    assert!(!exists(&f.b, "only_a.txt"));

    let detail = eb.conflict_detail(&conflict_id).unwrap();
    assert_eq!(detail.files[0].ours.text.as_deref(), Some("B version\n"));
    assert_eq!(detail.files[0].theirs.text.as_deref(), Some("A version\n"));

    let mut res = HashMap::new();
    res.insert("src/lib.rs".to_string(), Resolution::Theirs);
    let out = applied(eb.resolve_conflict(&conflict_id, &res).unwrap());
    assert_eq!(read(&f.b, "src/lib.rs"), "A version\n");
    assert_eq!(read(&f.b, "only_a.txt"), "from A\n");
    assert_eq!(read(&f.b, "only_b.txt"), "from B\n", "non-conflicting local work kept");
    // The version not chosen is preserved.
    let dir = std::path::PathBuf::from(out.preserved_dir.unwrap());
    assert_eq!(read(&dir, "ours/src/lib.rs"), "B version\n");
    assert!(eb.conflicts(Some(&pb)).unwrap().is_empty());

    // The merged state flows back to A as a fast-forward of A's lineage? A has
    // not changed since its checkpoint, and B's merge descends from it.
    eb.push_pending(&pb).unwrap();
    let inc = receive(&ea, &pa);
    let preview = ea.plan_apply(&pa, &inc, &ApplyOptions::default()).unwrap();
    assert!(preview.plan.auto_eligible(), "{:?}", preview.plan);
    applied(ea.apply(&pa, &inc, &ApplyOptions::default(), false).unwrap());
    assert_eq!(read(&f.a, "only_b.txt"), "from B\n");
}

#[test]
fn keep_both_writes_sidecar_and_non_overlapping_edits_merge() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);
    eb.checkpoint_now(&pb, Trigger::Manual).unwrap(); // establish lineage on B

    write(&f.a, "README.md", "A readme\n");
    write(&f.a, "src/lib.rs", "A lib\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    write(&f.b, "README.md", "B readme\n");
    let inc = receive(&eb, &pb);
    let ApplyOutcome::Conflicts { conflict_id, .. } = eb.apply(&pb, &inc, &ApplyOptions::default(), false).unwrap() else {
        panic!()
    };
    let name = ea.device().name;
    let mut res = HashMap::new();
    res.insert("README.md".to_string(), Resolution::Both);
    applied(eb.resolve_conflict(&conflict_id, &res).unwrap());
    assert_eq!(read(&f.b, "README.md"), "B readme\n");
    assert_eq!(read(&f.b, &continuity_core::apply::sidecar_path("README.md", &name)), "A readme\n");
    assert_eq!(read(&f.b, "src/lib.rs"), "A lib\n", "non-overlapping change merged");
}

#[test]
fn branch_switch_requires_explicit_confirmation() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);
    git(&f.a, &["checkout", "-b", "feature/x"]);
    write(&f.a, "feat.txt", "feature work\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    let inc = receive(&eb, &pb);
    let ApplyOutcome::Blocked { plan } = eb.apply(&pb, &inc, &ApplyOptions::default(), false).unwrap() else {
        panic!("must not switch branches silently")
    };
    assert!(plan.needs_branch_confirmation);
    assert_eq!(git(&f.b, &["symbolic-ref", "HEAD"]), "refs/heads/main");
    assert!(!exists(&f.b, "feat.txt"));
    let opts = ApplyOptions { allow_branch_switch: true, files_only: false };
    applied(eb.apply(&pb, &inc, &opts, false).unwrap());
    assert_eq!(git(&f.b, &["symbolic-ref", "HEAD"]), "refs/heads/feature/x");
    assert_eq!(read(&f.b, "feat.txt"), "feature work\n");
}

#[test]
fn diverged_branch_is_never_reset_files_only_is_offered() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);
    write(&f.a, "a.txt", "a\n");
    git(&f.a, &["add", "a.txt"]);
    git(&f.a, &["commit", "-m", "A commit"]);
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    write(&f.b, "b.txt", "b\n");
    git(&f.b, &["add", "b.txt"]);
    git(&f.b, &["commit", "-m", "B commit"]);
    let b_head = git(&f.b, &["rev-parse", "HEAD"]);
    let inc = receive(&eb, &pb);
    assert!(matches!(eb.apply(&pb, &inc, &ApplyOptions::default(), false).unwrap(), ApplyOutcome::Blocked { .. }));
    assert_eq!(git(&f.b, &["rev-parse", "HEAD"]), b_head);
    let opts = ApplyOptions { allow_branch_switch: false, files_only: true };
    applied(eb.apply(&pb, &inc, &opts, false).unwrap());
    assert_eq!(git(&f.b, &["rev-parse", "HEAD"]), b_head, "HEAD untouched in files-only mode");
    assert_eq!(read(&f.b, "a.txt"), "a\n");
    assert_eq!(read(&f.b, "b.txt"), "b\n");
}

#[test]
fn local_ignored_file_is_not_overwritten() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);
    write(&f.a, "config.local", "from A\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    std::fs::write(f.b.join(".git/info/exclude"), "config.local\n").unwrap();
    write(&f.b, "config.local", "B private settings\n");
    let inc = receive(&eb, &pb);
    let preview = eb.plan_apply(&pb, &inc, &ApplyOptions::default()).unwrap();
    assert_eq!(preview.plan.conflicts.len(), 1);
    assert_eq!(preview.plan.conflicts[0].reason, "local_untracked");
    let ApplyOutcome::Conflicts { conflict_id, .. } = eb.apply(&pb, &inc, &ApplyOptions::default(), false).unwrap() else {
        panic!()
    };
    assert_eq!(read(&f.b, "config.local"), "B private settings\n");
    let mut res = HashMap::new();
    res.insert("config.local".to_string(), Resolution::Theirs);
    let out = applied(eb.resolve_conflict(&conflict_id, &res).unwrap());
    assert_eq!(read(&f.b, "config.local"), "from A\n");
    let dir = std::path::PathBuf::from(out.preserved_dir.unwrap());
    assert_eq!(read(&dir, "local/config.local"), "B private settings\n", "replaced local file backed up");
}

#[test]
fn concurrent_edit_during_apply_is_detected() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);
    write(&f.a, "src/lib.rs", "A\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    let inc = receive(&eb, &pb);
    // Simulate an editor writing between planning and execution.
    let g = open(&f.b);
    let state = continuity_core::snapshot::capture(&g).unwrap();
    let theirs = continuity_core::snapshot::read_checkpoint(&g, &eb.db().checkpoint(&pb, &inc).unwrap().refname).unwrap();
    let base = state.tree.clone();
    let plan = continuity_core::apply::plan(&g, &state, &theirs, &base, &ApplyOptions::default()).unwrap();
    write(&f.b, "src/lib.rs", "edited by IDE meanwhile\n");
    let err = continuity_core::apply::execute(&g, &plan, &plan.changes, &[], &f.data_b.join("p")).unwrap_err();
    assert_eq!(err.code(), "conflict");
    assert_eq!(read(&f.b, "src/lib.rs"), "edited by IDE meanwhile\n");
}

#[test]
fn auto_apply_only_when_trusted_safe_and_clean() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let pa = add(&ea, &f.a);
    {
        // IDE running → manual.
        let eb = engine(&f.data_b, false);
        let pb = add(&eb, &f.b);
        eb.update_project(&pb, &ProjectPatch { trusted: Some(true), ..Default::default() }).unwrap();
        write(&f.a, "x.txt", "1\n");
        ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
        push(&ea, &pa);
        eb.sync_project(&pb, true).unwrap();
        assert!(!exists(&f.b, "x.txt"), "must not apply while an IDE may be using the project");
    }
    let eb = engine(&f.data_b, true);
    let pb = eb.projects().unwrap()[0].id.clone();
    // Not trusted → no auto apply.
    eb.update_project(&pb, &ProjectPatch { trusted: Some(false), ..Default::default() }).unwrap();
    eb.sync_project(&pb, true).unwrap();
    assert!(!exists(&f.b, "x.txt"));
    // Trusted + safe + clean, but device A not approved yet → manual.
    eb.update_project(&pb, &ProjectPatch { trusted: Some(true), ..Default::default() }).unwrap();
    eb.sync_project(&pb, true).unwrap();
    assert!(!exists(&f.b, "x.txt"));
    // Device approved → applied automatically.
    eb.approve_device(&ea.device().id, true).unwrap();
    eb.sync_project(&pb, true).unwrap();
    assert_eq!(read(&f.b, "x.txt"), "1\n");
    // Local edits on B → next update needs manual action.
    write(&f.b, "mine.txt", "local\n");
    write(&f.a, "x.txt", "2\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    eb.sync_project(&pb, true).unwrap();
    assert_eq!(read(&f.b, "x.txt"), "1\n");
    assert!(eb.db().activity(Some(&pb), 50).unwrap().iter().any(|a| a.kind == "update_needs_manual"));
}

#[test]
fn offline_queue_retries_with_backoff_and_survives_restart() {
    let f = fixture();
    let pa;
    {
        let ea = engine(&f.data_a, true);
        pa = add(&ea, &f.a);
        let good = f.remote.to_str().unwrap().to_string();
        let gone = f.remote.with_file_name("missing.git");
        git(&f.a, &["remote", "set-url", "origin", gone.to_str().unwrap()]);
        write(&f.a, "offline.txt", "made offline\n");
        ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
        assert_eq!(ea.process_queue().unwrap(), 0);
        let q = ea.queue().unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].attempts, 1);
        assert_eq!(q[0].failure_kind.as_deref(), Some("offline"));
        assert!(q[0].next_attempt_ms > continuity_core::util::now_ms(), "backoff scheduled");
        assert_eq!(ea.process_queue().unwrap(), 0, "not due yet");
        git(&f.a, &["remote", "set-url", "origin", &good]);
    }
    // Agent restarts: the queue is persisted.
    let ea = engine(&f.data_a, true);
    assert_eq!(ea.queue().unwrap().len(), 1);
    ea.db().queue_retry_now(None).unwrap();
    assert_eq!(ea.process_queue().unwrap(), 1);
    assert!(ea.queue().unwrap().is_empty());
    assert_eq!(remote_refs(&f.remote).len(), 1);
    assert_eq!(ea.projects().unwrap()[0].id, pa);
}

#[test]
fn secrets_block_or_require_confirmation_before_push() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let pa = add(&ea, &f.a);
    let token = format!("ghp_{}", "Zz09".repeat(9));
    write(&f.a, "deploy.yml", format!("token: {token}\n"));
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    let r = ea.push_pending(&pa).unwrap();
    assert_eq!(r.quarantined.len(), 1);
    assert!(remote_refs(&f.remote).is_empty(), "nothing with a credential reaches the remote");
    let findings = ea.findings(&pa).unwrap();
    assert!(findings.iter().all(|x| !x.excerpt.contains(&token[8..])), "secret not stored");
    assert!(ea.approve_findings(&pa, &[findings[0].fingerprint.clone()]).is_err(), "blocking cannot be approved");

    // Remove the token; a later checkpoint is clean and pushes.
    write(&f.a, "deploy.yml", "token: ${{ secrets.TOKEN }}\n");
    write(&f.a, "settings.py", "PASSWORD = \"correct-horse-battery\"\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    let r = ea.push_pending(&pa).unwrap();
    assert_eq!(r.needs_review.len(), 1, "warning-level finding waits for confirmation");
    assert!(remote_refs(&f.remote).is_empty());
    let fps: Vec<String> = ea.findings(&pa).unwrap().into_iter().filter(|x| x.severity == "warning").map(|x| x.fingerprint).collect();
    ea.approve_findings(&pa, &fps).unwrap();
    let r = ea.push_pending(&pa).unwrap();
    assert_eq!(r.pushed.len(), 1);
    assert_eq!(remote_refs(&f.remote).len(), 1);
}

fn fake_token() -> String {
    format!("ghp_{}", "Qw81".repeat(9))
}

#[test]
fn audit2_secret_only_in_staging_is_not_pushed() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let pa = add(&ea, &f.a);
    write(&f.a, "config.yml", format!("token: {}\n", fake_token()));
    git(&f.a, &["add", "config.yml"]);
    // Removed from the working tree, still staged.
    write(&f.a, "config.yml", "token: ${TOKEN}\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    let r = ea.push_pending(&pa).unwrap();
    assert_eq!(r.quarantined.len(), 1, "{r:?}");
    assert!(remote_refs(&f.remote).is_empty(), "the staging commit must not reach the remote");
}

#[test]
fn audit4_secret_in_large_file_or_long_line_is_not_pushed() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let pa = add(&ea, &f.a);
    // 2 MiB text file with the token at the very end.
    let mut big = "a".repeat(2 * 1024 * 1024);
    big.push_str(&format!("\nkey={}\n", fake_token()));
    write(&f.a, "data/dump.txt", big);
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    assert_eq!(ea.push_pending(&pa).unwrap().quarantined.len(), 1);
    std::fs::remove_file(f.a.join("data/dump.txt")).unwrap();
    // Minified one-line bundle with the token after byte 4000.
    write(&f.a, "dist/app.min.js", format!("{};var t=\"{}\";", "x".repeat(9000), fake_token()));
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    assert_eq!(ea.push_pending(&pa).unwrap().quarantined.len(), 1);
    assert!(remote_refs(&f.remote).is_empty());
}

#[test]
fn audit1_autocommit_branch_push_is_scanned_including_history() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let pa = add(&ea, &f.a);
    let remote_main = git(&f.remote, &["rev-parse", "main"]);
    // A secret committed by hand and then removed: the final tree is clean,
    // but pushing the branch would publish the intermediate commit.
    write(&f.a, "leak.txt", format!("{}\n", fake_token()));
    git(&f.a, &["add", "leak.txt"]);
    git(&f.a, &["commit", "-m", "oops"]);
    std::fs::remove_file(f.a.join("leak.txt")).unwrap();
    ea.update_project(&pa, &ProjectPatch { autocommit: Some(true), ..Default::default() }).unwrap();
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap();
    assert!(git(&f.a, &["log", "-1", "--format=%s"]).starts_with("chore(continuity):"), "auto-commit made");
    ea.db().queue_retry_now(None).unwrap();
    ea.process_queue().unwrap();
    assert_eq!(git(&f.remote, &["rev-parse", "main"]), remote_main, "branch with secret history not pushed");
    assert!(remote_refs(&f.remote).is_empty(), "checkpoint carrying the same history held too");
    assert!(ea.findings(&pa).unwrap().iter().any(|x| x.checkpoint_id.starts_with("branch:")));
}

/// Push a hand-made checkpoint ref, as an attacker with write access to the
/// remote would. Returns the forged checkpoint id.
fn forge_checkpoint(
    repo: &std::path::Path,
    device_id: &str,
    parents: Vec<String>,
    key: Option<&continuity_core::signing::DeviceKey>,
) -> String {
    use continuity_core::checkpoint::{self, CheckpointMeta};
    use continuity_core::snapshot;
    let g = open(repo);
    write(repo, "evil.txt", "malicious change\n");
    let state = snapshot::capture(&g).unwrap();
    std::fs::remove_file(repo.join("evil.txt")).unwrap();
    let head: Vec<&str> = state.head.as_deref().into_iter().collect();
    let idx = snapshot::commit_tree(&g, &state.index_tree, &head, "index").unwrap();
    let id = checkpoint::new_id(continuity_core::util::now_ms() + 60_000);
    let mut meta = CheckpointMeta {
        id: id.clone(),
        device_id: device_id.into(),
        device_name: "Laptop".into(),
        branch: state.branch.clone(),
        head: state.head.clone(),
        index_commit: idx.clone(),
        created_ms: continuity_core::util::now_ms(),
        trigger: Trigger::Timer,
        parents,
        summary: String::new(),
        public_key: None,
        signature: None,
    };
    if let Some(k) = key {
        meta.public_key = Some(k.public_key());
        meta.signature = Some(k.sign(&continuity_core::signing::payload(&meta, &state.tree)));
    }
    let mut parents_c = head.clone();
    parents_c.push(&idx);
    let commit = snapshot::commit_tree(&g, &state.tree, &parents_c, &meta.to_message()).unwrap();
    let r = checkpoint::ref_name(device_id, &id);
    git(repo, &["update-ref", &r, &commit]);
    git(repo, &["push", "-q", "origin", &format!("{r}:{r}")]);
    id
}

#[test]
fn audit3_forged_checkpoints_are_never_auto_applied() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);
    eb.update_project(&pb, &ProjectPatch { trusted: Some(true), ..Default::default() }).unwrap();

    // A genuine, signed checkpoint from A is not auto-applied until A is approved.
    write(&f.a, "real.txt", "from A\n");
    let c = ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    assert!(c.verified);
    push(&ea, &pa);
    eb.sync_project(&pb, true).unwrap();
    assert!(!exists(&f.b, "real.txt"), "unapproved device");
    let a_id = ea.device().id;
    let dev = eb.devices().unwrap().into_iter().find(|d| d.id == a_id).unwrap();
    assert_eq!(dev.public_key, Some(ea.devices().unwrap().into_iter().find(|d| d.is_local).unwrap().public_key.unwrap()));
    eb.approve_device(&a_id, true).unwrap();
    eb.sync_project(&pb, true).unwrap();
    assert_eq!(read(&f.b, "real.txt"), "from A\n", "approved + verified → automatic");
    let b_head = eb.projects().unwrap()[0].lineage_head.clone().unwrap();

    // 1) Unsigned checkpoint from an unknown device claiming B's head as parent.
    let fake_dev = uuid::Uuid::new_v4().to_string();
    let forged = forge_checkpoint(&f.a, &fake_dev, vec![b_head.clone()], None);
    eb.sync_project(&pb, true).unwrap();
    assert!(!exists(&f.b, "evil.txt"), "unsigned checkpoint must not be written");
    assert!(!eb.db().checkpoint(&pb, &forged).unwrap().verified);

    // 2) Checkpoint impersonating A, signed with an attacker key.
    let attacker = continuity_core::signing::DeviceKey::load_or_create(&continuity_core::credentials::MemoryStore::default()).unwrap();
    let forged = forge_checkpoint(&f.a, &a_id, vec![b_head], Some(&attacker));
    eb.sync_project(&pb, true).unwrap();
    assert!(!exists(&f.b, "evil.txt"), "impersonation must not be written");
    assert!(!eb.db().checkpoint(&pb, &forged).unwrap().verified);
    assert!(eb.db().activity(None, 50).unwrap().iter().any(|a| a.kind == "device_key_mismatch"));
    // The manual path still shows it, flagged as unverified.
    let preview = eb.plan_apply(&pb, &forged, &ApplyOptions::default()).unwrap();
    assert!(!preview.origin_verified && !preview.auto_eligible);

    // Resetting a device key revokes approval and re-pins on the next signed checkpoint.
    eb.reset_device_key(&a_id).unwrap();
    let dev = eb.devices().unwrap().into_iter().find(|d| d.id == a_id).unwrap();
    assert!(dev.public_key.is_none() && !dev.approved);
    write(&f.a, "real2.txt", "again\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    eb.fetch(&pb).unwrap();
    let dev = eb.devices().unwrap().into_iter().find(|d| d.id == a_id).unwrap();
    assert!(dev.public_key.is_some() && !dev.approved, "re-pinned, waiting for a new approval");
}

#[test]
fn audit2_finding2_key_reset_invalidates_checkpoints_verified_with_the_old_key() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);
    eb.update_project(&pb, &ProjectPatch { trusted: Some(true), ..Default::default() }).unwrap();
    let a_id = ea.device().id;

    // A checkpoint signed with A's current key arrives and stays pending.
    write(&f.a, "old.txt", "signed with the old key
");
    let old = ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    eb.fetch(&pb).unwrap();
    assert!(eb.db().checkpoint(&pb, &old.id).unwrap().verified);

    // The user resets A's key (say it leaked); a new key gets pinned and approved.
    eb.reset_device_key(&a_id).unwrap();
    assert!(!eb.db().checkpoint(&pb, &old.id).unwrap().verified, "reset clears verifications made with the old key");
    let new_key = continuity_core::signing::DeviceKey::load_or_create(&continuity_core::credentials::MemoryStore::default()).unwrap();
    forge_checkpoint(&f.a, &a_id, vec![old.id.clone()], Some(&new_key));
    eb.fetch(&pb).unwrap();
    let dev = eb.devices().unwrap().into_iter().find(|d| d.id == a_id).unwrap();
    assert_eq!(dev.public_key, Some(new_key.public_key()));
    eb.approve_device(&a_id, true).unwrap();

    // The old-key checkpoint must not ride on the new approval, even if the
    // stored flag were stale.
    rusqlite::Connection::open(f.data_b.join("continuity.db"))
        .unwrap()
        .execute("UPDATE checkpoints SET verified = 1 WHERE id = ?1", [&old.id])
        .unwrap();
    let preview = eb.plan_apply(&pb, &old.id, &ApplyOptions::default()).unwrap();
    assert!(!preview.origin_verified && !preview.auto_eligible);
    let out = eb.apply(&pb, &old.id, &ApplyOptions::default(), true).unwrap();
    assert!(matches!(out, ApplyOutcome::NeedsManual { .. }), "{out:?}");
    assert!(!exists(&f.b, "stale.txt"));
}

#[test]
fn concurrent_checkpoints_are_serialised() {
    let f = fixture();
    let ea = std::sync::Arc::new(engine(&f.data_a, true));
    let pa = add(&ea, &f.a);
    write(&f.a, "c.txt", "concurrent\n");
    let mut hs = Vec::new();
    for _ in 0..6 {
        let e = ea.clone();
        let id = pa.clone();
        hs.push(std::thread::spawn(move || e.checkpoint_now(&id, Trigger::Watcher).unwrap().is_some()));
    }
    let created = hs.into_iter().map(|h| h.join().unwrap()).filter(|c| *c).count();
    assert_eq!(created, 1);
    assert_eq!(git(&f.a, &["for-each-ref", "refs/continuity/"]).lines().count(), 1);
    assert!(!git(&f.a, &["status", "--porcelain"]).contains("index"));
}

#[test]
fn interrupted_operation_is_reported_after_restart() {
    let f = fixture();
    {
        let ea = engine(&f.data_a, true);
        let pa = add(&ea, &f.a);
        ea.db().op_start("op-x", &pa, "apply", Some("0000000000001-abcdef"), "").unwrap();
    } // crash
    let ea = engine(&f.data_a, true);
    let pa = ea.projects().unwrap()[0].id.clone();
    assert_eq!(ea.project_status(&pa).unwrap().interrupted_ops, 1);
    assert!(ea.db().activity(Some(&pa), 10).unwrap().iter().any(|a| a.kind == "operation_interrupted"));
}

#[test]
fn retention_prunes_old_checkpoints_locally_and_remotely_but_keeps_needed_ones() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let pa = add(&ea, &f.a);
    for i in 0..5 {
        write(&f.a, "r.txt", format!("{i}\n"));
        ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    }
    push(&ea, &pa);
    assert_eq!(remote_refs(&f.remote).len(), 5);
    let mut s = ea.settings().unwrap();
    s.retention = continuity_core::retention::RetentionPolicy { keep_all_hours: 0, daily_days: 0, weekly_weeks: 0, min_keep: 1 };
    ea.set_settings(&s).unwrap();
    let rep = ea.run_retention(&pa).unwrap();
    assert!(rep.queued_remote >= 3, "{rep:?}");
    ea.process_queue().unwrap();
    let left = remote_refs(&f.remote);
    assert!(left.len() <= 2, "{left:?}");
    let head = ea.projects().unwrap()[0].lineage_head.clone().unwrap();
    assert!(left.iter().any(|r| r.ends_with(&head)), "lineage head is never pruned");
    assert_eq!(git(&f.a, &["for-each-ref", "refs/continuity/"]).lines().count(), left.len());
}

#[test]
fn unsafe_paths_in_remote_trees_are_never_written() {
    use continuity_core::snapshot::RefCheckpoint;
    let f = fixture();
    let g = open(&f.b);
    // Craft a hostile tree with entries git itself would refuse to check out.
    let blob = git(&f.b, &["hash-object", "-w", "--stdin"]);
    let blob = if blob.is_empty() {
        let p = f.b.join("payload.tmp");
        std::fs::write(&p, "pwned\n").unwrap();
        let o = git(&f.b, &["hash-object", "-w", p.to_str().unwrap()]);
        std::fs::remove_file(&p).unwrap();
        o
    } else {
        blob
    };
    let raw = |entries: &[(&str, &str, &str)]| -> String {
        let mut data = Vec::new();
        for (mode, name, oid) in entries {
            data.extend_from_slice(format!("{mode} {name}\0").as_bytes());
            data.extend_from_slice(&hex::decode(oid).unwrap());
        }
        let p = f.b.join(".git/tree.raw");
        std::fs::write(&p, data).unwrap();
        git(&f.b, &["hash-object", "-t", "tree", "-w", "--literally", p.to_str().unwrap()])
    };
    let inner = raw(&[("100644", "config", &blob)]);
    let tree = raw(&[("100644", "..", &blob), ("40000", ".git", &inner), ("40000", ".GIT", &inner), ("100644", "ok.txt", &blob)]);
    let state = continuity_core::snapshot::capture(&g).unwrap();
    let empty = g.empty_tree().unwrap();
    let idx = continuity_core::snapshot::commit_tree(&g, &empty, &[], "i").unwrap();
    let theirs = RefCheckpoint {
        refname: "x".into(),
        commit: idx.clone(),
        tree: tree.clone(),
        meta: continuity_core::checkpoint::CheckpointMeta {
            id: "0000000000001-abcdef".into(),
            device_id: uuid::Uuid::new_v4().to_string(),
            device_name: "evil".into(),
            branch: None,
            head: None,
            index_commit: idx,
            created_ms: 1,
            trigger: Trigger::Manual,
            parents: vec![],
            summary: String::new(),
            public_key: None,
            signature: None,
        },
    };
    let plan = continuity_core::apply::plan(&g, &state, &theirs, &state.tree, &ApplyOptions { files_only: true, ..Default::default() }).unwrap();
    let written: Vec<&str> = plan.changes.iter().map(|c| c.path.as_str()).collect();
    assert!(written.iter().all(|p| !p.contains("..") && !p.to_ascii_lowercase().starts_with(".git/")), "{written:?}");
    assert!(plan.skipped.len() >= 2, "{:?}", plan.skipped);
    let hooks_before = std::fs::read_dir(f.b.join(".git")).unwrap().count();
    let changes: Vec<_> = plan.changes.iter().filter(|c| matches!(c.action, continuity_core::apply::FileAction::Write { .. })).cloned().collect();
    continuity_core::apply::execute(&g, &plan, &changes, &[], &f.data_b.join("p")).unwrap();
    assert_eq!(std::fs::read_dir(f.b.join(".git")).unwrap().count(), hooks_before);
    assert!(!f.b.parent().unwrap().join("..").join("pwned").exists());
    assert!(exists(&f.b, "ok.txt"), "the safe entry is still applied");
}

#[test]
fn sync_and_shutdown_pushes_everything_and_clears_queue() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let pa = add(&ea, &f.a);
    write(&f.a, "last.txt", "before quitting\n");
    let rep = ea.sync_and_shutdown().unwrap();
    assert_eq!(rep.len(), 1);
    assert!(rep[0].error.is_none(), "{rep:?}");
    assert!(rep[0].checkpoint.is_some());
    assert_eq!(rep[0].pushed, 1);
    assert!(ea.queue().unwrap().is_empty());
    assert_eq!(remote_refs(&f.remote).len(), 1);
    // queue items of a removed project are dropped, not retried forever
    write(&f.a, "again.txt", "x\n");
    ea.checkpoint_now(&pa, Trigger::Manual).unwrap().unwrap();
    ea.remove_project(&pa).unwrap();
    ea.db().enqueue(&pa, "push", "").unwrap();
    ea.process_queue().unwrap();
    assert!(ea.queue().unwrap().is_empty());
}

#[test]
fn autocommit_mode_commits_and_pushes_branch_but_respects_partial_staging() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let pa = add(&ea, &f.a);
    ea.update_project(&pa, &ProjectPatch { autocommit: Some(true), ..Default::default() }).unwrap();
    let head0 = git(&f.a, &["rev-parse", "HEAD"]);

    // Partial staging → commit postponed, branch untouched, checkpoint still made.
    write(&f.a, "staged.txt", "s\n");
    git(&f.a, &["add", "staged.txt"]);
    write(&f.a, "README.md", "unstaged\n");
    assert!(ea.checkpoint_now(&pa, Trigger::Timer).unwrap().is_some());
    assert_eq!(git(&f.a, &["rev-parse", "HEAD"]), head0);
    assert_eq!(git(&f.a, &["diff", "--cached", "--name-only"]), "staged.txt");

    // Clean staging → a conventional commit on the branch, pushed without force.
    git(&f.a, &["reset", "-q"]);
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap();
    let head1 = git(&f.a, &["rev-parse", "HEAD"]);
    assert_ne!(head1, head0);
    assert!(git(&f.a, &["log", "-1", "--format=%s"]).starts_with("chore(continuity):"));
    assert_eq!(git(&f.a, &["status", "--porcelain"]), "", "worktree and index match the new commit");
    ea.db().queue_retry_now(None).unwrap();
    ea.process_queue().unwrap();
    assert_eq!(git(&f.remote, &["rev-parse", "main"]), head1);

    // Remote branch moved elsewhere → rejected, never forced.
    write(&f.b, "other.txt", "b\n");
    git(&f.b, &["add", "other.txt"]);
    git(&f.b, &["commit", "-m", "from B"]);
    git(&f.b, &["pull", "-q", "--rebase", "origin", "main"]);
    git(&f.b, &["push", "-q", "origin", "main"]);
    let remote_head = git(&f.remote, &["rev-parse", "main"]);
    write(&f.a, "more.txt", "a\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap();
    ea.db().queue_retry_now(None).unwrap();
    ea.process_queue().unwrap();
    assert_eq!(git(&f.remote, &["rev-parse", "main"]), remote_head, "remote branch not overwritten");
    assert!(ea.db().activity(Some(&pa), 50).unwrap().iter().any(|a| a.kind == "push_branch_rejected"));
}

#[test]
fn eol_conversion_keeps_integrity_and_clean_status() {
    let f = fixture();
    for d in [&f.a, &f.b] {
        git(d, &["config", "core.autocrlf", "true"]);
        // Re-checkout so the worktree matches the new EOL settings.
        git(d, &["rm", "-q", "--cached", "-r", "."]);
        git(d, &["reset", "-q", "--hard"]);
    }
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);
    write(&f.a, "src/lib.rs", "line1\r\nline2\r\n");
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);
    let inc = receive(&eb, &pb);
    applied(eb.apply(&pb, &inc, &ApplyOptions::default(), false).unwrap());
    let st = git(&f.b, &["status", "--porcelain"]);
    assert_eq!(st, " M src/lib.rs", "only the synced change shows: {st}");
    assert_eq!(git(&f.b, &["diff", "--stat"]), git(&f.a, &["diff", "--stat"]));
}

/// A path `sh` understands on every platform (Git for Windows' sh takes C:/x).
fn sh_path(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[test]
fn audit3_finding1_project_git_config_cannot_run_programs() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let pa = add(&ea, &f.a);
    let tmp = f.remote.parent().unwrap();
    let marker = |n: &str| tmp.join(format!("ran-{n}"));

    // (a) core.fsmonitor and (b) a reference-transaction hook: overridden on
    // every call, so Elo keeps working and neither runs.
    git(&f.a, &["config", "core.fsmonitor", &format!("echo x > {}", sh_path(&marker("fsmonitor")))]);
    write(&f.a, ".git/hooks/reference-transaction", format!("#!/bin/sh\necho x > {}\n", sh_path(&marker("hook"))));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(f.a.join(".git/hooks/reference-transaction"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    write(&f.a, "src/lib.rs", "pub fn a() { 1 }\n");
    let st = ea.project_status(&pa).unwrap();
    assert_eq!(st.uncommitted_changes, Some(1));
    assert!(st.unsafe_config.is_none(), "{:?}", st.unsafe_config);
    ea.checkpoint_now(&pa, Trigger::Timer).unwrap().expect("checkpoint");
    assert!(!marker("fsmonitor").exists(), "core.fsmonitor do projeto foi executado");
    assert!(!marker("hook").exists(), "hook do projeto foi executado");
    // Control: plain Git does run both, so the payloads are real.
    git(&f.a, &["status", "--porcelain"]);
    git(&f.a, &["update-ref", "refs/control", "HEAD"]);
    assert!(marker("fsmonitor").exists() && marker("hook").exists(), "controle: git comum executaria os comandos");
    git(&f.a, &["config", "--unset", "core.fsmonitor"]);
    std::fs::remove_file(f.a.join(".git/hooks/reference-transaction")).unwrap();

    // (c) a clean filter defined in the project's config: Elo refuses to
    // capture and says why, instead of running it.
    git(&f.a, &["config", "filter.evil.clean", &format!("echo x > {}; cat", sh_path(&marker("filter")))]);
    write(&f.a, ".git/info/attributes", "*.rs filter=evil\n");
    write(&f.a, "src/lib.rs", "pub fn a() { 2 }\n");
    let err = ea.checkpoint_now(&pa, Trigger::Timer).unwrap_err();
    assert_eq!(err.code(), "unsafe_repo_config", "{err}");
    assert!(err.to_string().contains("filter.evil.clean"), "{err}");
    let st = ea.project_status(&pa).unwrap();
    assert!(st.unsafe_config.as_deref().is_some_and(|m| m.contains("filter.evil.clean")), "{:?}", st.unsafe_config);
    assert!(!marker("filter").exists(), "filtro do projeto foi executado");
    // ... and refuses to add a repository configured like that.
    let eb = engine(&f.data_b, true);
    git(&f.b, &["config", "credential.helper", "!echo evil"]);
    let err = eb
        .add_project(&continuity_core::engine::AddProjectOptions { path: f.b.to_str().unwrap().into(), ..Default::default() })
        .unwrap_err();
    assert_eq!(err.code(), "unsafe_repo_config", "{err}");
    // Git LFS installed only in the project gets a concrete way out.
    git(&f.b, &["config", "--unset", "credential.helper"]);
    git(&f.b, &["config", "filter.lfs.process", "git-lfs filter-process"]);
    let err = eb
        .add_project(&continuity_core::engine::AddProjectOptions { path: f.b.to_str().unwrap().into(), ..Default::default() })
        .unwrap_err()
        .to_string();
    assert!(err.contains("git lfs install") && err.contains("--remove-section filter.lfs"), "{err}");
    git(&f.a, &["add", "-A"]);
    assert!(marker("filter").exists(), "controle: git comum executaria o filtro");
    git(&f.a, &["reset", "-q"]);
    git(&f.a, &["config", "--remove-section", "filter.evil"]);

    // (d) a URL rewrite to the ext:: transport: the effective URL is checked
    // before fetch/push, and the transport is not allowed anyway.
    let remote = f.remote.to_str().unwrap();
    git(&f.a, &["config", "protocol.ext.allow", "always"]);
    git(&f.a, &["config", &format!("url.ext::sh -c echo% x>{}.insteadOf", sh_path(&marker("ext"))), remote]);
    let err = ea.fetch(&pa).unwrap_err();
    assert!(err.to_string().contains("transporte"), "{err}");
    let err = ea.push_pending(&pa).unwrap_err();
    assert!(err.to_string().contains("transporte"), "{err}");
    assert!(!marker("ext").exists(), "transporte ext:: foi executado");
    let _ = git_ok(&f.a, &["fetch", "origin"]);
    assert!(marker("ext").exists(), "controle: git comum executaria o ext::");
}

#[test]
fn audit3_finding2_credentials_in_urls_are_refused_and_never_shown() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let pa = add(&ea, &f.a);
    // Built at runtime: a literal token in the source would itself be flagged
    // by the secret scanner when Elo syncs this repository.
    let secret = &format!("ghp_{}", "0".repeat(36));
    let with_token = format!("https://user:{secret}@127.0.0.1:9/x.git");

    let err = ea.set_project_remote(&pa, &with_token).unwrap_err();
    assert!(!err.to_string().contains(secret), "{err}");
    let err = ea.clone_project(&with_token, f.remote.parent().unwrap().join("c").to_str().unwrap()).unwrap_err();
    assert!(!err.to_string().contains(secret), "{err}");
    let eb = engine(&f.data_b, true);
    let err = eb
        .add_project(&continuity_core::engine::AddProjectOptions {
            path: f.b.to_str().unwrap().into(),
            remote_url: Some(format!("https://{secret}@github.com/a/b.git")),
            ..Default::default()
        })
        .unwrap_err();
    assert!(err.to_string().contains("token"), "{err}");
    assert_eq!(git(&f.a, &["remote", "get-url", "origin"]), f.remote.to_str().unwrap(), "nada foi gravado");

    // A URL configured before (or outside) Elo: never shown or logged in full.
    git(&f.a, &["remote", "set-url", "origin", &with_token]);
    let st = ea.project_status(&pa).unwrap();
    assert_eq!(st.remote_url.as_deref(), Some("https://***@127.0.0.1:9/x.git"));
    let err = ea.fetch(&pa).unwrap_err().to_string();
    assert!(!err.contains(secret), "{err}");
    let _ = ea.sync_project(&pa, false);
    let log = ea.db().activity(Some(&pa), 200).unwrap();
    assert!(log.iter().all(|a| !a.message.contains(secret)), "{log:?}");
}

#[cfg(unix)]
#[test]
fn audit3_finding3_engine_creates_a_private_profile_on_every_entry_point() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("new/profile");
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    // As `continuity --direct` does: Engine::open without the Agent.
    let _e = engine(&data, true);
    assert_eq!(mode(&data), 0o700);
    assert_eq!(mode(&data.join("continuity.db")), 0o600);
    // An existing profile with loose permissions is tightened on open.
    drop(_e);
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(data.join("continuity.db"), std::fs::Permissions::from_mode(0o644)).unwrap();
    let _e = engine(&data, true);
    assert_eq!(mode(&data), 0o700);
    assert_eq!(mode(&data.join("continuity.db")), 0o600);
}

#[test]
fn many_files_binary_and_empty_are_applied_and_restored_intact() {
    let f = fixture();
    let ea = engine(&f.data_a, true);
    let eb = engine(&f.data_b, true);
    let pa = add(&ea, &f.a);
    let pb = add(&eb, &f.b);
    let n = 300;
    for i in 0..n {
        write(&f.a, &format!("gen/arquivo {i:03}.txt"), format!("linha {i}\n"));
    }
    let bin: Vec<u8> = (0..=255u8).chain([b'\n', 0, b'\n']).collect();
    write(&f.a, "gen/bin.dat", &bin);
    write(&f.a, "gen/empty.txt", "");
    let c = ea.checkpoint_now(&pa, Trigger::Timer).unwrap().unwrap();
    push(&ea, &pa);

    let inc = receive(&eb, &pb);
    let r = applied(eb.apply(&pb, &inc, &ApplyOptions::default(), false).unwrap());
    assert_eq!(r.written.len(), n + 2);
    for i in [0, 127, 128, 255, 256, n - 1] {
        assert_eq!(read(&f.b, &format!("gen/arquivo {i:03}.txt")), format!("linha {i}\n"));
    }
    assert_eq!(std::fs::read(f.b.join("gen/bin.dat")).unwrap(), bin);
    assert_eq!(read(&f.b, "gen/empty.txt"), "");

    let target = f.data_a.join("restaurado");
    let rep = ea.restore_to_dir(&pa, &c.id, target.to_str().unwrap()).unwrap();
    assert!(rep.integrity_failures.is_empty() && rep.skipped.is_empty(), "{rep:?}");
    assert_eq!(rep.verified, rep.files_written);
    assert_eq!(std::fs::read(target.join("gen/bin.dat")).unwrap(), bin);
    assert_eq!(read(&target, &format!("gen/arquivo {:03}.txt", n - 1)), format!("linha {}\n", n - 1));
}
