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
use std::sync::OnceLock;
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

/// Config file the TUI edits, when it is not the default.
///
/// The TUI has 24 `ConfigEditor::load()` call sites across 12 section modules.
/// Threading a path through all of them invites a missed one, and a missed one
/// silently writes the user's real config instead of the file they named —
/// which is the bug this exists to prevent (#595). Setting it once at TUI
/// startup keeps the resolution in a single place that cannot be partially
/// applied.
static TUI_CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Point `ConfigEditor::load()` at a specific file for the rest of this
/// process. Called once, from `tui::run`, with the `-c/--config` value.
///
/// Ignored when `path` is `None` so the default resolution stays untouched,
/// and a no-op if called twice: the TUI is a single-purpose process and a
/// second config file mid-run has no meaning.
pub fn set_tui_config_path(path: Option<PathBuf>) {
    if let Some(path) = path {
        let _ = TUI_CONFIG_PATH.set(path);
    }
}

/// The config file the TUI is editing: the `-c/--config` override when one was
/// given, otherwise the default path.
///
/// Anything in the TUI that needs to know which file is in play must ask here
/// rather than calling `Config::default_path()` directly, or it will disagree
/// with what the sections actually read and write.
pub fn tui_config_path() -> Option<PathBuf> {
    resolve_tui_config_path(TUI_CONFIG_PATH.get())
}

/// Pure form of `tui_config_path`, taking the override explicitly so the
/// resolution can be tested without touching the process-global `OnceLock`
/// (which only sets once, making order-dependent tests unavoidable otherwise).
fn resolve_tui_config_path(override_path: Option<&PathBuf>) -> Option<PathBuf> {
    match override_path {
        Some(path) => Some(path.clone()),
        None => config::Config::default_path(),
    }
}

impl ConfigEditor {
    /// Load the config the TUI is editing: the `-c/--config` file when one was
    /// given, otherwise `~/.config/voxtype/config.toml`. Creates an empty
    /// document if the file is missing — `save()` writes it on first edit.
    pub fn load() -> Result<Self, EditorError> {
        let path = tui_config_path().ok_or(EditorError::NoConfigPath)?;
        Self::load_from(path)
    }

    /// Load from an arbitrary path (creating an empty document if the file
    /// is missing). Used by the CLI `voxtype config set` command, which has
    /// to honor `--config <FILE>` and the resolution chain in main.rs.
    pub fn load_from_path(path: PathBuf) -> Result<Self, EditorError> {
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
        let mut document: DocumentMut = text.parse().map_err(|e| EditorError::Parse {
            path: path.clone(),
            source: e,
        })?;

        // Migration: earlier rc/0.7.0 builds had a bug where `set_string("",
        // "engine", ...)` created a literal `[""]` table at the document root
        // and stored `engine` inside it. The runtime config loader rejected
        // anything in there (so behavior wasn't broken), but the corrupt
        // section persisted across saves. Strip it on load — the corrected
        // set_string now writes to the root, so dropping the empty-name
        // table loses no settings.
        let mut dirty = false;
        if document.as_table().contains_key("") {
            document.as_table_mut().remove("");
            dirty = true;
        }

        Ok(Self {
            path,
            document,
            dirty,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn set_string(&mut self, table: &str, key: &str, value: &str) {
        if table.is_empty() {
            self.document.as_table_mut()[key] = toml_edit::value(value);
        } else {
            let item = self.ensure_table(table);
            item[key] = toml_edit::value(value);
        }
        self.dirty = true;
    }

    pub fn set_bool(&mut self, table: &str, key: &str, value: bool) {
        if table.is_empty() {
            self.document.as_table_mut()[key] = toml_edit::value(value);
        } else {
            let item = self.ensure_table(table);
            item[key] = toml_edit::value(value);
        }
        self.dirty = true;
    }

    pub fn set_int(&mut self, table: &str, key: &str, value: i64) {
        if table.is_empty() {
            self.document.as_table_mut()[key] = toml_edit::value(value);
        } else {
            let item = self.ensure_table(table);
            item[key] = toml_edit::value(value);
        }
        self.dirty = true;
    }

    /// Write a TOML float. Use this rather than `set_string` with a
    /// formatted number so the resulting key is parseable by every
    /// section's deserializer (TOML 0.x rejects "0.95" as a float in
    /// strict mode).
    pub fn set_float(&mut self, table: &str, key: &str, value: f64) {
        if table.is_empty() {
            self.document.as_table_mut()[key] = toml_edit::value(value);
        } else {
            let item = self.ensure_table(table);
            item[key] = toml_edit::value(value);
        }
        self.dirty = true;
    }

    /// Write an `f32` as a TOML float. Convenience wrapper over `set_float`
    /// so callers working with `f32`-typed config fields don't need to
    /// repeat the widening cast at every call site (the missing `as f64`
    /// is exactly how the audio.feedback.volume and vad.threshold fields
    /// ended up serialized as quoted strings — see #451).
    pub fn set_f32(&mut self, table: &str, key: &str, value: f32) {
        self.set_float(table, key, value as f64);
    }

    /// Read an `f32`, tolerating older TOML representations where the TUI
    /// wrote the value as a quoted string (`volume = "0.70"`) or as an
    /// int. Returns `default` if the key is absent or none of the fallback
    /// parses succeed. The string fallback exists for a transitional
    /// window after the v0.7.6 fix to #451 — every reload through this
    /// helper migrates the user's config to a proper TOML float on next
    /// save. Can be retired once we're confident no fielded config still
    /// carries the legacy form.
    pub fn get_f32_or(&self, table: &str, key: &str, default: f32) -> f32 {
        self.get_float(table, key)
            .map(|f| f as f32)
            .or_else(|| self.get_int(table, key).map(|n| n as f32))
            .or_else(|| self.get_string(table, key).and_then(|s| s.parse().ok()))
            .unwrap_or(default)
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
            current = current.get_mut(segment).and_then(|i| i.as_table_mut())?;
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

    /// Read a TOML float. Falls back to integer-as-float so a previously
    /// saved `opacity = 1` keeps loading after the user edits via the TUI
    /// (which writes back as `opacity = 1.0`).
    pub fn get_float(&self, table: &str, key: &str) -> Option<f64> {
        let v = self.value(table, key)?;
        v.as_float().or_else(|| v.as_integer().map(|n| n as f64))
    }

    fn value(&self, table: &str, key: &str) -> Option<&Value> {
        self.table(table)?.get(key).and_then(|i| i.as_value())
    }

    /// The raw TOML value at `table.key`, or `None` when the key is absent.
    ///
    /// Callers that need to report what is *literally in the file* (as
    /// opposed to a resolved value with defaults applied) use this to type
    /// the value themselves — `voxtype config schema --json` reports both.
    pub fn get_toml_value(&self, table: &str, key: &str) -> Option<&Value> {
        self.value(table, key)
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
            if !current.get(segment).map(|i| i.is_table()).unwrap_or(false) {
                current.insert(segment, Item::Table(toml_edit::Table::new()));
            }
            current = current[segment]
                .as_table_mut()
                .expect("just inserted a table");
        }

        if !current.get(last).map(|i| i.is_table()).unwrap_or(false) {
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
            source: std::io::Error::other("config path has no parent directory"),
        })?;
        fs::create_dir_all(parent).map_err(|e| EditorError::Write {
            path: parent.to_path_buf(),
            source: e,
        })?;

        // Back up the existing on-disk config the first time the TUI saves.
        // toml_edit reformats whitespace, comment placement, and key
        // ordering on round-trip, so even a no-op save can produce a
        // different-looking file. The backup gives users a fast undo if
        // they ran the TUI on a hand-rolled config they liked the shape
        // of. One-shot: existing .bak survives and is never overwritten,
        // so we capture the user's actual original, not the prior TUI run.
        if self.path.exists() {
            let mut bak = self.path.clone();
            let mut name = bak
                .file_name()
                .map(|n| n.to_os_string())
                .unwrap_or_default();
            name.push(".bak");
            bak.set_file_name(name);
            if !bak.exists() {
                let _ = fs::copy(&self.path, &bak);
            }
        }

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

    fn temp_config(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        (dir, path)
    }

    /// #595: an override is used verbatim, and its absence falls back to the
    /// default so the common case is unchanged.
    #[test]
    fn tui_config_path_prefers_the_override() {
        let scratch = PathBuf::from("/tmp/scratch.toml");
        assert_eq!(
            resolve_tui_config_path(Some(&scratch)).as_deref(),
            Some(scratch.as_path())
        );
        // Without an override the answer is whatever the default resolution
        // says — including None in an environment with no home directory.
        assert_eq!(
            resolve_tui_config_path(None),
            config::Config::default_path()
        );
    }

    /// The override is what `load()` consults, and it is what `save()` then
    /// writes back to — the two must not diverge, which is exactly how the
    /// bug wrote the user's real config while reporting the scratch file.
    #[test]
    fn editor_saves_to_the_path_it_loaded() {
        let dir = tempfile::TempDir::new().unwrap();
        let scratch = dir.path().join("scratch.toml");
        std::fs::write(&scratch, "engine = \"whisper\"\n").unwrap();

        let mut ed = ConfigEditor::load_from_path(scratch.clone()).unwrap();
        ed.set_string("whisper", "model", "large-v3");
        ed.save().unwrap();

        let written = std::fs::read_to_string(&scratch).unwrap();
        assert!(
            written.contains("large-v3"),
            "the file that was loaded must be the file that is written"
        );
    }

    #[test]
    fn round_trip_preserves_comments() {
        let (_dir, path) =
            temp_config("# top comment\n[hotkey]\n# inline\nkey = \"HOME\"\nmode = \"toggle\"\n");
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
    fn dotted_table_reads_and_writes_nested() {
        // The output section stores its post-process command under
        // [output.post_process], so dotted paths must resolve on both
        // the read and write sides.
        let (_dir, path) = temp_config("[output.post_process]\ncommand = \"my-cleanup\"\n");
        let mut ed = ConfigEditor::load_from(path).unwrap();
        assert_eq!(
            ed.get_string("output.post_process", "command").as_deref(),
            Some("my-cleanup")
        );
        ed.set_string("output.post_process", "command", "other-cleanup");
        assert!(ed.document.to_string().contains("command = \"other-cleanup\""));
        ed.unset("output.post_process", "command");
        assert_eq!(ed.get_string("output.post_process", "command"), None);
    }

    #[test]
    fn dirty_tracks_writes() {
        let (_dir, path) = temp_config("[hotkey]\nkey = \"HOME\"\n");
        let mut ed = ConfigEditor::load_from(path).unwrap();
        assert!(!ed.is_dirty());
        ed.set_string("hotkey", "key", "PAUSE");
        assert!(ed.is_dirty());
    }

    #[test]
    fn set_f32_writes_toml_number_not_string() {
        // Regression for #451: the TUI audio + vad sections used `set_string`
        // with `format!("{:.2}", f32)` to write floats, producing
        // `volume = "0.70"`. The daemon's serde config expects `f32` and
        // rejects the string with "invalid type: string \"0.70\", expected f32".
        // `set_f32` must emit a TOML number so deserialization works on next
        // load, and `get_f32_or` must round-trip it back.
        let (_dir, path) = temp_config("");
        let mut ed = ConfigEditor::load_from(path.clone()).unwrap();
        // 0.5 and 0.25 are exact in IEEE-754 so the f32 -> f64 widening
        // doesn't decorate the serialized form with a long mantissa.
        ed.set_f32("audio.feedback", "volume", 0.5);
        ed.set_f32("vad", "threshold", 0.25);
        let serialized = ed.document.to_string();

        // The critical invariant from #451: no quoted string form.
        assert!(
            !serialized.contains("\"0."),
            "no value should be quoted as a string, got: {}",
            serialized
        );
        // Loose sanity check on the bare-number form.
        assert!(
            serialized.contains("volume = 0.5"),
            "expected bare TOML float, got: {}",
            serialized
        );
        assert!(
            serialized.contains("threshold = 0.25"),
            "expected bare TOML float, got: {}",
            serialized
        );

        // Write to disk and reload — bypasses ConfigEditor::save()'s schema
        // validation (the partial doc here lacks required [audio].device etc.).
        // We just want to prove set_f32 -> file -> get_f32_or preserves the
        // value as a float.
        fs::write(&path, &serialized).unwrap();
        let reloaded = ConfigEditor::load_from(path).unwrap();
        assert_eq!(reloaded.get_f32_or("audio.feedback", "volume", -1.0), 0.5);
        assert_eq!(reloaded.get_f32_or("vad", "threshold", -1.0), 0.25);
    }

    #[test]
    fn get_f32_or_recovers_legacy_string_and_int_forms() {
        // Users running fielded v0.7.5 had their audio.feedback.volume and
        // vad.threshold written as quoted strings by the buggy TUI. After
        // the fix, the loader must still read those legacy values so the
        // user isn't reset to the default — and the next save will migrate
        // them to a proper TOML float.
        let (_dir, path) = temp_config(concat!(
            "[audio.feedback]\nvolume = \"0.70\"\n",
            "[vad]\nthreshold = 1\n", // legacy int form
            "[other]\n",              // missing key uses default
        ));
        let ed = ConfigEditor::load_from(path).unwrap();
        assert_eq!(ed.get_f32_or("audio.feedback", "volume", -1.0), 0.70);
        assert_eq!(ed.get_f32_or("vad", "threshold", -1.0), 1.0);
        assert_eq!(ed.get_f32_or("other", "missing", 0.42), 0.42);
    }
}
