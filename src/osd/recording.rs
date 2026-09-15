//! Recording identity shared by the daemon and session-aware OSD clients.
//!
//! A small sidecar keeps the existing plain-text state file compatible with
//! scripts. Session identifiers also travel on `audio.sock.v2`, so queued
//! frames from a previous recording cannot end the next startup indicator.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Current daemon state and its recording identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingState {
    /// Plain state name, matching the original state file.
    pub state: String,
    /// Unique across recordings and daemon restarts.
    pub session: u64,
}

/// Sidecar beside the configured state file (including custom paths).
pub fn sidecar_path(state_path: &Path) -> PathBuf {
    let mut name = state_path.as_os_str().to_os_string();
    name.push(".osd.json");
    PathBuf::from(name)
}

impl RecordingState {
    /// Atomically publish state and identity so observers never mix sessions.
    pub fn write(&self, state_path: &Path) -> std::io::Result<()> {
        let path = sidecar_path(state_path);
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut staged, self)?;
        staged.persist(path).map_err(|e| e.error)?;
        Ok(())
    }

    /// Read an optional sidecar; older daemons provide only the plain state file.
    pub fn read(state_path: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(sidecar_path(state_path)).ok()?).ok()
    }
}
