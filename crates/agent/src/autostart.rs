//! Start the Agent at login. Only changed when the user toggles it.

use continuity_core::{Error, Result};
use std::path::Path;

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const VALUE: &str = "ContinuityAgent";

#[cfg(windows)]
pub fn set(enable: bool, exe: &Path) -> Result<()> {
    use std::process::Command;
    let mut cmd = Command::new("reg");
    if enable {
        cmd.args(["add", RUN_KEY, "/v", VALUE, "/t", "REG_SZ", "/f", "/d"]).arg(format!("\"{}\"", exe.display()));
    } else {
        cmd.args(["delete", RUN_KEY, "/v", VALUE, "/f"]);
    }
    continuity_core::git::hide_console(&mut cmd);
    let out = cmd.output()?;
    if !out.status.success() && enable {
        return Err(Error::Other(String::from_utf8_lossy(&out.stderr).into_owned()));
    }
    Ok(())
}

#[cfg(windows)]
pub fn is_enabled() -> bool {
    let mut cmd = std::process::Command::new("reg");
    cmd.args(["query", RUN_KEY, "/v", VALUE]);
    continuity_core::git::hide_console(&mut cmd);
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

#[cfg(not(windows))]
fn desktop_file() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart").join("continuity-agent.desktop"))
}

#[cfg(not(windows))]
pub fn set(enable: bool, exe: &Path) -> Result<()> {
    let f = desktop_file().ok_or_else(|| Error::Other("HOME não definido".into()))?;
    if enable {
        std::fs::create_dir_all(f.parent().unwrap())?;
        let exe = exe.display().to_string().replace('"', "\\\"");
        std::fs::write(
            &f,
            format!(
                "[Desktop Entry]\nType=Application\nName=Continuity Agent\nExec=\"{exe}\"\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n"
            ),
        )?;
    } else if f.exists() {
        std::fs::remove_file(f)?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    desktop_file().map(|f| f.exists()).unwrap_or(false)
}
