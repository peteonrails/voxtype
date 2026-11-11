//! Per-agent consent to send meeting transcripts.
//!
//! An agent CLI sends what it is given to whichever account it is signed in
//! to, usually a cloud service. voxtype asks once per agent and keeps the
//! answer in `$XDG_STATE_HOME/voxtype/assistant-consent.json`:
//!
//! ```json
//! {"version":1,"agents":{"claude":{"allowed_at":"2026-11-11T20:00:00Z","path":"/usr/bin/claude"}}}
//! ```
//!
//! `path` is where the CLI was found when consent was given. The daemon
//! falls back to it when its own PATH (a systemd user unit's, typically)
//! does not reach the agent.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// File name under voxtype's state directory.
pub const CONSENT_FILE: &str = "assistant-consent.json";

/// One agent's stored consent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsentEntry {
    pub allowed_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ConsentFile {
    #[serde(default = "version")]
    version: u32,
    #[serde(default)]
    agents: BTreeMap<String, ConsentEntry>,
}

fn version() -> u32 {
    1
}

/// The consent file, loaded. A missing or unreadable file is "no consent".
#[derive(Debug, Clone)]
pub struct ConsentStore {
    file: PathBuf,
    data: ConsentFile,
}

impl ConsentStore {
    pub fn load(file: &Path) -> Self {
        let data = match std::fs::read_to_string(file) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("Ignoring unreadable consent file {:?}: {}", file, e);
                ConsentFile::default()
            }),
            Err(_) => ConsentFile::default(),
        };
        Self {
            file: file.to_path_buf(),
            data,
        }
    }

    pub fn path(&self) -> &Path {
        &self.file
    }

    pub fn get(&self, agent: &str) -> Option<&ConsentEntry> {
        self.data.agents.get(agent)
    }

    pub fn is_allowed(&self, agent: &str) -> bool {
        self.get(agent).is_some()
    }

    /// Record consent for `agent`, found at `path`, and save.
    pub fn allow(&mut self, agent: &str, path: Option<PathBuf>) -> io::Result<()> {
        self.data.agents.insert(
            agent.to_string(),
            ConsentEntry {
                allowed_at: Utc::now(),
                path,
            },
        );
        self.save()
    }

    /// Withdraw consent for `agent` and save. Returns whether it was given.
    pub fn revoke(&mut self, agent: &str) -> io::Result<bool> {
        let had = self.data.agents.remove(agent).is_some();
        self.save()?;
        Ok(had)
    }

    fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut data = self.data.clone();
        data.version = version();
        let json = serde_json::to_string_pretty(&data).map_err(io::Error::other)?;
        let tmp = self
            .file
            .with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&tmp, json)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &self.file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_means_no_consent() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConsentStore::load(&dir.path().join(CONSENT_FILE));
        assert!(!store.is_allowed("claude"));
    }

    #[test]
    fn allow_and_revoke_persist() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state").join(CONSENT_FILE);
        let mut store = ConsentStore::load(&file);
        store
            .allow("claude", Some(PathBuf::from("/usr/bin/claude")))
            .unwrap();

        let reloaded = ConsentStore::load(&file);
        assert!(reloaded.is_allowed("claude"));
        assert!(!reloaded.is_allowed("codex"));
        assert_eq!(
            reloaded.get("claude").unwrap().path.as_deref(),
            Some(Path::new("/usr/bin/claude"))
        );

        let mut store = reloaded;
        assert!(store.revoke("claude").unwrap());
        assert!(!store.revoke("claude").unwrap());
        assert!(!ConsentStore::load(&file).is_allowed("claude"));
    }

    #[test]
    fn a_corrupt_file_is_no_consent() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(CONSENT_FILE);
        std::fs::write(&file, "not json").unwrap();
        assert!(!ConsentStore::load(&file).is_allowed("claude"));
    }
}
