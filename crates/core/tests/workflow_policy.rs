//! Supply-chain policy for the GitHub workflows (audit 3, finding 4): every
//! external action is pinned by full commit SHA, and write permission exists
//! only at job level, never as the workflow-wide default.

use std::path::PathBuf;

fn workflows() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows");
    let mut v: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("diretório de workflows")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    v.sort();
    assert!(!v.is_empty(), "nenhum workflow encontrado em {}", dir.display());
    v
}

fn pinned(spec: &str) -> bool {
    if spec.starts_with("./") || spec.starts_with("docker://") {
        return true;
    }
    match spec.rsplit_once('@') {
        Some((_, rev)) => rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        None => false,
    }
}

#[test]
fn audit3_finding4_actions_are_pinned_by_full_sha() {
    for (name, text) in workflows() {
        for (i, line) in text.lines().enumerate() {
            let t = line.trim_start().trim_start_matches("- ");
            if let Some(rest) = t.strip_prefix("uses:") {
                let spec = rest.split('#').next().unwrap().trim().trim_matches(|c| c == '"' || c == '\'');
                assert!(pinned(spec), "{name}:{}: action sem SHA completo: {spec}", i + 1);
            }
        }
    }
}

#[test]
fn audit3_finding4_write_permission_only_at_job_level() {
    for (name, text) in workflows() {
        let mut top_level = false;
        for (i, line) in text.lines().enumerate() {
            let indent = line.len() - line.trim_start().len();
            if indent == 0 && !line.trim().is_empty() {
                top_level = line.trim_end() == "permissions:";
                assert!(!line.contains("write-all"), "{name}:{}: permissões globais de escrita", i + 1);
                continue;
            }
            if line.contains(": write") || line.contains("write-all") {
                assert!(!top_level, "{name}:{}: escrita concedida a todo o workflow: {}", i + 1, line.trim());
                assert!(indent >= 6, "{name}:{}: escrita fora do nível de job: {}", i + 1, line.trim());
            }
        }
    }
}

#[test]
fn pin_check_rejects_tags_and_short_shas() {
    assert!(pinned("actions/checkout@11d5960a326750d5838078e36cf38b85af677262"));
    assert!(!pinned("actions/checkout@v4"));
    assert!(!pinned("dtolnay/rust-toolchain@stable"));
    assert!(!pinned("actions/checkout@11d5960"));
    assert!(!pinned("actions/checkout"));
}
