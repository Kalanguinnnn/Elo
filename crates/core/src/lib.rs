//! Continuity core: everything except process management and UI.

pub mod api;
pub mod apply;
pub mod autocommit;
pub mod b64;
pub mod checkpoint;
pub mod credentials;
pub mod db;
pub mod device;
pub mod engine;
pub mod error;
pub mod git;
pub mod github;
pub mod ide;
pub mod ipc;
pub mod paths;
pub mod restore;
pub mod retention;
pub mod secrets;
pub mod signing;
pub mod snapshot;
pub mod transport;
pub mod util;

pub use error::{Error, Result};

/// Per-user data directory (`%LOCALAPPDATA%\EloData`, `~/.local/share/elo`).
/// `CONTINUITY_DATA_DIR` overrides it (tests, portable use).
///
/// It must never be the installer's target: the per-user NSIS installer uses
/// `%LOCALAPPDATA%\<productName>`, and uninstalling would delete the database.
pub fn default_data_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("CONTINUITY_DATA_DIR") {
        return d.into();
    }
    let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    let dir = if cfg!(windows) { base.join("EloData") } else { base.join("elo") };
    let legacy = if cfg!(windows) { base.join("Continuity") } else { base.join("continuity") };
    migrate_legacy_data(&legacy, &dir);
    dir
}

/// Copy the database from the pre-Elo location once, so the device identity
/// and history survive the move. The old folder is left untouched.
fn migrate_legacy_data(old: &std::path::Path, new: &std::path::Path) {
    let db = "continuity.db";
    if new.join(db).exists() || !old.join(db).exists() {
        return;
    }
    if std::fs::create_dir_all(new).is_err() {
        return;
    }
    for f in [db, "continuity.db-wal", "continuity.db-shm"] {
        if old.join(f).exists() {
            let _ = std::fs::copy(old.join(f), new.join(f));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn legacy_database_is_copied_once() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("old"), t.path().join("new"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("continuity.db"), b"v1").unwrap();
        super::migrate_legacy_data(&old, &new);
        assert_eq!(std::fs::read(new.join("continuity.db")).unwrap(), b"v1");
        std::fs::write(old.join("continuity.db"), b"v2").unwrap();
        super::migrate_legacy_data(&old, &new);
        assert_eq!(std::fs::read(new.join("continuity.db")).unwrap(), b"v1", "never overwrites");
        assert!(old.join("continuity.db").exists(), "old data kept");
    }
}
