//! Persistent device identity.

use crate::db::Db;
use crate::error::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
}

pub fn hostname() -> String {
    let raw = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .unwrap_or_default();
    let name = crate::util::sanitize_line(&raw, 60);
    if name.is_empty() { "Computador".into() } else { name }
}

/// Load the device identity, creating it (UUID v4 + host name) on first run.
pub fn load_or_create(db: &Db) -> Result<DeviceInfo> {
    if let Some((id, name)) = db.local_device()? {
        return Ok(DeviceInfo { id, name });
    }
    let id = uuid::Uuid::new_v4().to_string();
    let name = hostname();
    db.create_local_device(&id, &name)?;
    db.touch_device(&id, &name, crate::util::now_ms())?;
    Ok(DeviceInfo { id, name })
}
