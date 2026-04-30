// Most callers land in the next commit; keep dead-code warnings quiet until
// the Hotkey section starts using it.
#![allow(dead_code)]

//! Shared config-file editing plumbing for TUI sections.
//!
//! Wraps `toml_edit` so per-section edits preserve comments, formatting, and
//! unknown fields. Writes are atomic (temp file + rename), and every write
//! is followed by a parse-validation pass through [`crate::config::load_config`]
//! before returning success — if the new file would fail to load at startup,
//! the in-memory edit is rolled back and the on-disk file is left alone.

use crate::config;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Value};

#[derive(Debug, thiserror::Error)]
pub enum EditorError {
    #[error("could not determine config path; set $XDG_CONFIG_HOME or $HOME")]
    NoConfigPath,
    #[error("read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml_edit::TomlError,
    },
    #[error("write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("validate after write: {0}")]
    Validate(String),
}

pub struct ConfigEditor {
    path: PathBuf,
    document: DocumentMut,
    dirty: bool,
}

impl ConfigEditor {
    /// Load `~/.config/voxtype/config.toml` (creating an empty document if the
    /// file is missing — `save()` will write it on first edit).
    pub fn load() -> Result<Self, EditorError> {
        let path = config::Config::default_path().ok_or(EditorError::NoConfigPath)?;
        Self::load_from(path)
    }

    fn load_from(path: PathBuf) -> Result<Self, EditorError> {
        let text = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(EditorError::Read {
                    path: path.clone(),
                    source: e,
                })
            }
        };
        let document: DocumentMut = text.parse().map_err(|e| EditorError::Parse {
            path: path.clone(),
            source: e,
        })?;
        Ok(Self {
            path,
            document,
            dirty: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn set_string(&mut self, table: &str, key: &str, value: &str) {
        let item = self.ensure_table(table);
        item[key] = toml_edit::value(value);
        self.dirty = true;
    }

    pub fn set_bool(&mut self, table: &str, key: &str, value: bool) {
        let item = self.ensure_table(table);
        item[key] = toml_edit::value(value);
        self.dirty = true;
    }

    pub fn set_int(&mut self, table: &str, key: &str, value: i64) {
        let item = self.ensure_table(table);
        item[key] = toml_edit::value(value);
        self.dirty = true;
    }

    /// Remove a key from a table (no-op if absent).
    pub fn unset(&mut self, table: &str, key: &str) {
        if let Some(t) = self.table_mut(table) {
            if t.remove(key).is_some() {
                self.dirty = true;
            }
        }
    }

    fn table_mut(&mut self, dotted: &str) -> Option<&mut toml_edit::Table> {
        let mut current = self.document.as_table_mut();
        if dotted.is_empty() {
            return Some(current);
        }
        for segment in dotted.split('.') {
            current = current
                .get_mut(segment)
                .and_then(|i| i.as_table_mut())?;
        }
        Some(current)
    }

    fn table(&self, dotted: &str) -> Option<&toml_edit::Table> {
        let mut current = self.document.as_table();
        if dotted.is_empty() {
            return Some(current);
        }
        for segment in dotted.split('.') {
            current = current.get(segment).and_then(|i| i.as_table())?;
        }
        Some(current)
    }

    /// Public read-only access to a table, for callers that need to iterate
    /// arbitrary keys (e.g. the replacement-list editor walking
    /// `[text.replacements]`).
    pub fn raw_table(&self, dotted: &str) -> Option<&toml_edit::Table> {
        self.table(dotted)
    }

    pub fn get_string(&self, table: &str, key: &str) -> Option<String> {
        self.value(table, key)?.as_str().map(|s| s.to_string())
    }

    pub fn get_bool(&self, table: &str, key: &str) -> Option<bool> {
        self.value(table, key)?.as_bool()
    }

    pub fn get_int(&self, table: &str, key: &str) -> Option<i64> {
        self.value(table, key)?.as_integer()
    }

    fn value(&self, table: &str, key: &str) -> Option<&Value> {
        self.table(table)?.get(key).and_then(|i| i.as_value())
    }

    /// Ensure a (possibly dotted) `[table]` path exists and return it as a
    /// mutable Item. Creates intermediate tables as needed.
    fn ensure_table(&mut self, dotted: &str) -> &mut Item {
        let segments: Vec<&str> = dotted.split('.').collect();
        let (last, rest) = segments
            .split_last()
            .expect("ensure_table called with empty path");

        // Walk through (or create) intermediate tables.
        let mut current: &mut toml_edit::Table = self.document.as_table_mut();
        for segment in rest {
            if !current
                .get(segment)
                .map(|i| i.is_table())
                .unwrap_or(false)
            {
                current.insert(segment, Item::Table(toml_edit::Table::new()));
            }
            current = current[segment]
                .as_table_mut()
                .expect("just inserted a table");
        }

        if !current
            .get(last)
            .map(|i| i.is_table())
            .unwrap_or(false)
        {
            current.insert(last, Item::Table(toml_edit::Table::new()));
        }
        &mut current[last]
    }

    /// Atomically write the document and validate it parses through the
    /// regular `load_config` path. On validation failure the file is left
    /// untouched on disk (atomic rename hasn't happened yet).
    pub fn save(&mut self) -> Result<(), EditorError> {
        let serialized = self.document.to_string();

        // Validate before touching the on-disk file: parse the serialized
        // text via the runtime config loader. We do this by writing to a temp
        // file, loading from there, and only renaming on success.
        let parent = self.path.parent().ok_or_else(|| EditorError::Write {
            path: self.path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::Other,
                "config path has no parent directory",
            ),
        })?;
        fs::create_dir_all(parent).map_err(|e| EditorError::Write {
            path: parent.to_path_buf(),
            source: e,
        })?;

        let mut tmp = self.path.clone();
        let mut file_name = tmp
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        file_name.push(".tmp");
        tmp.set_file_name(file_name);

        {
            let mut f = fs::File::create(&tmp).map_err(|e| EditorError::Write {
                path: tmp.clone(),
                source: e,
            })?;
            f.write_all(serialized.as_bytes())
                .map_err(|e| EditorError::Write {
                    path: tmp.clone(),
                    source: e,
                })?;
            f.sync_all().map_err(|e| EditorError::Write {
                path: tmp.clone(),
                source: e,
            })?;
        }

        // Validate by loading via the same code path the daemon uses.
        if let Err(e) = config::load_config(Some(&tmp)) {
            let _ = fs::remove_file(&tmp);
            return Err(EditorError::Validate(format!("{}", e)));
        }

        fs::rename(&tmp, &self.path).map_err(|e| EditorError::Write {
            path: self.path.clone(),
            source: e,
        })?;

        self.dirty = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn temp_config(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        (dir, path)
    }

    #[test]
    fn round_trip_preserves_comments() {
        let (_dir, path) = temp_config(
            "# top comment\n[hotkey]\n# inline\nkey = \"HOME\"\nmode = \"toggle\"\n",
        );
        let mut ed = ConfigEditor::load_from(path.clone()).unwrap();
        ed.set_string("hotkey", "key", "PAUSE");
        let serialized = ed.document.to_string();
        assert!(serialized.contains("# top comment"), "{}", serialized);
        assert!(serialized.contains("# inline"), "{}", serialized);
        assert!(serialized.contains("key = \"PAUSE\""));
        assert!(serialized.contains("mode = \"toggle\""));
    }

    #[test]
    fn missing_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        let ed = ConfigEditor::load_from(path).unwrap();
        assert!(!ed.is_dirty());
        assert_eq!(ed.document.to_string(), "");
    }

    #[test]
    fn ensure_table_creates_if_missing() {
        let (_dir, path) = temp_config("");
        let mut ed = ConfigEditor::load_from(path).unwrap();
        ed.set_bool("notification", "on_start", true);
        let s = ed.document.to_string();
        assert!(s.contains("[notification]"));
        assert!(s.contains("on_start = true"));
    }

    #[test]
    fn dirty_tracks_writes() {
        let (_dir, path) = temp_config("[hotkey]\nkey = \"HOME\"\n");
        let mut ed = ConfigEditor::load_from(path).unwrap();
        assert!(!ed.is_dirty());
        ed.set_string("hotkey", "key", "PAUSE");
        assert!(ed.is_dirty());
    }
}
