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
pub mod snapshot;
pub mod transport;
pub mod util;

pub use error::{Error, Result};

/// Per-user data directory (`%LOCALAPPDATA%\Continuity`, `~/.local/share/continuity`).
/// `CONTINUITY_DATA_DIR` overrides it (tests, portable use).
pub fn default_data_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("CONTINUITY_DATA_DIR") {
        return d.into();
    }
    let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    if cfg!(windows) { base.join("Continuity") } else { base.join("continuity") }
}
