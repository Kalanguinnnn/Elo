//! Conservative check that no editor/IDE is likely using a project, used to
//! gate automatic application of incoming checkpoints. Any doubt → not safe.

use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::process::Command;

/// Process names (lowercase, without `.exe`) of editors and IDEs.
pub const KNOWN_EDITORS: &[&str] = &[
    "code", "code-insiders", "codium", "vscodium", "cursor", "windsurf", "zed", "zed-editor", "idea", "idea64", "pycharm",
    "pycharm64", "webstorm", "webstorm64", "phpstorm", "phpstorm64", "clion", "clion64", "goland", "goland64", "rider",
    "rider64", "rubymine", "rubymine64", "datagrip", "datagrip64", "rustrover", "rustrover64", "fleet", "studio",
    "studio64", "devenv", "sublime_text", "subl", "atom", "notepad++", "eclipse", "nvim", "vim", "gvim", "emacs",
    "kate", "gedit", "gnome-text-editor", "lapce", "helix", "hx", "nano", "micro", "android-studio", "kiro", "trae",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeCheck {
    pub safe: bool,
    pub running: Vec<String>,
    pub reason: String,
}

pub trait IdeProbe: Send + Sync {
    fn check(&self) -> IdeCheck;
}

pub struct SystemProbe;

impl IdeProbe for SystemProbe {
    fn check(&self) -> IdeCheck {
        match process_names() {
            Ok(names) => {
                let mut running: Vec<String> =
                    names.into_iter().filter(|n| KNOWN_EDITORS.contains(&n.as_str())).collect();
                running.sort();
                running.dedup();
                if running.is_empty() {
                    IdeCheck { safe: true, running, reason: "nenhum editor conhecido em execução".into() }
                } else {
                    IdeCheck { safe: false, reason: format!("editor em execução: {}", running.join(", ")), running }
                }
            }
            Err(e) => IdeCheck { safe: false, running: vec![], reason: format!("não foi possível listar processos: {e}") },
        }
    }
}

/// Test double.
pub struct FixedProbe(pub bool);
impl IdeProbe for FixedProbe {
    fn check(&self) -> IdeCheck {
        IdeCheck { safe: self.0, running: vec![], reason: if self.0 { "teste: seguro".into() } else { "teste: inseguro".into() } }
    }
}

fn process_names() -> std::io::Result<Vec<String>> {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("tasklist");
        cmd.args(["/fo", "csv", "/nh"]);
        crate::git::hide_console(&mut cmd);
        let out = cmd.output()?;
        if !out.status.success() {
            return Err(std::io::Error::other("tasklist falhou"));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(text
            .lines()
            .filter_map(|l| l.split(',').next())
            .map(|n| n.trim_matches('"').to_ascii_lowercase())
            .map(|n| n.strip_suffix(".exe").map(String::from).unwrap_or(n))
            .collect())
    }
    #[cfg(not(windows))]
    {
        let mut v = Vec::new();
        for e in std::fs::read_dir("/proc")?.flatten() {
            if !e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if let Ok(comm) = std::fs::read_to_string(e.path().join("comm")) {
                v.push(comm.trim().to_ascii_lowercase());
            }
            // Many IDEs run as `java`/`electron`; the cmdline reveals them.
            if let Ok(cmd) = std::fs::read(e.path().join("cmdline")) {
                if let Some(first) = cmd.split(|b| *b == 0).next() {
                    let s = String::from_utf8_lossy(first);
                    if let Some(base) = s.rsplit('/').next() {
                        v.push(base.to_ascii_lowercase());
                    }
                }
            }
        }
        Ok(v)
    }
}
