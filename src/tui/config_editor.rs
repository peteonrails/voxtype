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
    /// The file's text when this editor read it. A save compares the file
    /// against this to tell whether someone else has written it since.
    loaded_text: String,
    /// What this editor changed, in order, so a save can apply the same
    /// changes to a file that moved on underneath it.
    changes: Vec<Change>,
}

/// One edit made through a [`ConfigEditor`].
#[derive(Debug, Clone)]
enum Change {
    Set {
        table: String,
        key: String,
        value: Value,
        /// Whether the editor already saw this value in the file. A form
        /// that writes back every field it shows produces these; they say
        /// nothing about what the user wanted.
        unchanged: bool,
    },
    Unset {
        table: String,
        key: String,
    },
    Overlay {
        dotted: String,
        overlay: toml_edit::Table,
    },
}

impl Change {
    /// Pausing history or turning it off. These are applied to a file that
    /// changed underneath the editor even when the editor saw the same value
    /// already: a request to stop recording must not lose to a stale read.
    fn tightens_history_privacy(&self) -> bool {
        matches!(self, Change::Set { table, key, value, .. }
            if table == "history"
                && ((key == "paused" && value.as_bool() == Some(true))
                    || (key == "enabled" && value.as_bool() == Some(false))))
    }
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
            loaded_text: text,
            changes: Vec::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn candidate_config(&self) -> Result<config::Config, EditorError> {
        toml::from_str(&self.document.to_string())
            .map_err(|error| EditorError::Validate(error.to_string()))
    }

    /// The config the daemon would build from this file, for questions about
    /// what the daemon will do. Sections are parsed one by one, as
    /// `load_config` does, so one broken section does not hide the rest, and
    /// nothing is logged: the Hotkey section asks on every frame.
    ///
    /// The Soniox engine with no `[soniox]` table gets the table's defaults:
    /// the transcriber factory refuses Soniox without the table
    /// (`transcribe::create_transcriber`), and `SONIOX_API_KEY` or
    /// `--soniox-api-key` inserts one with defaults, so every Soniox daemon
    /// that starts has one. Other environment variables and CLI flags are not
    /// visible here.
    pub fn daemon_config(&self) -> Option<config::Config> {
        let (mut config, _rejected) =
            config::parse_config_salvaging(&self.document.to_string()).ok()?;
        if config.engine == config::TranscriptionEngine::Soniox && config.soniox.is_none() {
            config.soniox = Some(config::SonioxConfig::default());
        }
        Some(config)
    }

    pub fn set_string(&mut self, table: &str, key: &str, value: &str) {
        self.set_decorated_value(table, key, Value::from(value));
    }

    pub fn set_bool(&mut self, table: &str, key: &str, value: bool) {
        self.set_decorated_value(table, key, Value::from(value));
    }

    fn set_decorated_value(&mut self, table: &str, key: &str, value: Value) {
        self.record_set(table, key, value, true);
    }

    /// Apply a set to this editor's document and remember it for a rebase.
    fn record_set(&mut self, table: &str, key: &str, value: Value, keep_decor: bool) {
        let unchanged = self
            .value(table, key)
            .is_some_and(|previous| same_value(previous, &value));
        apply_set(&mut self.document, table, key, value.clone(), keep_decor);
        self.changes.push(Change::Set {
            table: table.to_string(),
            key: key.to_string(),
            value,
            unchanged,
        });
        self.dirty = true;
    }

    pub fn set_int(&mut self, table: &str, key: &str, value: i64) {
        self.record_set(table, key, Value::from(value), false);
    }

    /// Write a TOML float. Use this rather than `set_string` with a
    /// formatted number so the resulting key is parseable by every
    /// section's deserializer (TOML 0.x rejects "0.95" as a float in
    /// strict mode).
    pub fn set_float(&mut self, table: &str, key: &str, value: f64) {
        self.record_set(table, key, Value::from(value), false);
    }

    /// Write an `f32` as a TOML float. Convenience wrapper over `set_float`
    /// so callers working with `f32`-typed config fields don't need to
    /// repeat the widening cast at every call site (the missing `as f64`
    /// is exactly how the audio.feedback.volume and vad.threshold fields
    /// ended up serialized as quoted strings — see #451).
    ///
    /// The value is written with the fewest digits that read back as the
    /// same `f32`. Widening it with `as f64` keeps the binary value instead,
    /// and TOML then prints every digit of it: 0.7 came out as
    /// `0.699999988079071`.
    pub fn set_f32(&mut self, table: &str, key: &str, value: f32) {
        self.set_float(table, key, shortest_f64(value));
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
        self.remove_key(table, key);
    }

    /// Remove a key from a table; whether it was there.
    pub fn remove_key(&mut self, table: &str, key: &str) -> bool {
        let removed = apply_unset(&mut self.document, table, key);
        if removed {
            self.changes.push(Change::Unset {
                table: table.to_string(),
                key: key.to_string(),
            });
            self.dirty = true;
        }
        removed
    }

    /// Merge `overlay` into the table at `dotted`, creating it if missing.
    ///
    /// Tables merge key by key; every other item (scalars, arrays, arrays of
    /// tables) replaces what was there, so applying the same overlay twice
    /// never duplicates a key or an array entry. Keys the overlay doesn't
    /// mention are left alone, comments included. Returns the leaves whose
    /// value actually changed.
    pub fn overlay_table(
        &mut self,
        dotted: &str,
        overlay: &toml_edit::Table,
    ) -> Vec<OverlayChange> {
        let changes = apply_overlay(&mut self.document, dotted, overlay);
        if !changes.is_empty() {
            self.changes.push(Change::Overlay {
                dotted: dotted.to_string(),
                overlay: overlay.clone(),
            });
            self.dirty = true;
        }
        changes
    }

    fn table(&self, dotted: &str) -> Option<&dyn toml_edit::TableLike> {
        table_in(&self.document, dotted)
    }

    /// Public read-only access to a table, for callers that need to iterate
    /// arbitrary keys (e.g. the replacement-list editor walking
    /// `[text.replacements]`).
    ///
    /// Returned as a standard table whatever its form in the file, so an
    /// inline `replacements = { .. }` reads the same as `[text.replacements]`.
    pub fn raw_table(&self, dotted: &str) -> Option<toml_edit::Table> {
        let item = match dotted.rsplit_once('.') {
            Some((parent, last)) => self.table(parent)?.get(last)?,
            None if dotted.is_empty() => return Some(self.document.as_table().clone()),
            None => self.document.as_table().get(dotted)?,
        };
        item.clone().into_table().ok()
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
    #[cfg(test)]
    fn ensure_table(&mut self, dotted: &str) -> &mut Item {
        ensure_table(&mut self.document, dotted)
    }

    /// If the file changed since this editor read it, start again from what
    /// is on disk and apply this editor's own changes to that. Without this a
    /// save writes back the whole file as it was at load, undoing whatever
    /// another process wrote in between: a `voxtype history pause` could be
    /// reverted by a settings window that was already open.
    ///
    /// Call while holding the file's privacy lock, which every save takes
    /// when the data dir allows it, so the read and the write cannot be
    /// interleaved again.
    fn rebase_onto_disk(&mut self) -> Result<(), EditorError> {
        let on_disk = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(source) => {
                return Err(EditorError::Read {
                    path: self.path.clone(),
                    source,
                })
            }
        };
        if on_disk == self.loaded_text {
            return Ok(());
        }
        let mut fresh: DocumentMut = on_disk.parse().map_err(|e| EditorError::Parse {
            path: self.path.clone(),
            source: e,
        })?;
        if fresh.as_table().contains_key("") {
            fresh.as_table_mut().remove("");
        }
        for change in &self.changes {
            match change {
                // A field the editor wrote back as it found it is not a
                // request, so it does not override the newer file. A request
                // to pause or turn off history always is one.
                Change::Set { unchanged, .. }
                    if *unchanged && !change.tightens_history_privacy() => {}
                Change::Set {
                    table, key, value, ..
                } => apply_set(&mut fresh, table, key, value.clone(), true),
                Change::Unset { table, key } => {
                    apply_unset(&mut fresh, table, key);
                }
                Change::Overlay { dotted, overlay } => {
                    apply_overlay(&mut fresh, dotted, overlay);
                }
            }
        }
        self.document = fresh;
        self.loaded_text = on_disk;
        Ok(())
    }

    /// Atomically write the document and validate it parses through the
    /// regular `load_config` path. On validation failure the file is left
    /// untouched on disk (atomic rename hasn't happened yet).
    pub fn save(&mut self) -> Result<(), EditorError> {
        // A config that is a symlink (GNU Stow, a dotfiles repo) is saved
        // through to the file it names. The replacement is staged in that
        // file's directory so the rename stays on one filesystem, and the
        // link itself is never touched.
        let target =
            config::resolve_config_file(&self.path).map_err(|source| EditorError::Write {
                path: self.path.clone(),
                source,
            })?;
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let parent = parent.as_path();
        fs::create_dir_all(parent).map_err(|e| EditorError::Write {
            path: parent.to_path_buf(),
            source: e,
        })?;
        // Held from here to the rename. Reading the file, applying this
        // editor's changes to it and writing it back are one step as far as
        // any other save is concerned.
        //
        // The lock lives in the data dir, which can be unwritable (a
        // read-only home, a build sandbox) while the config is not. Saving
        // still works then. History stays safe because the recorder needs
        // this same lock to start or write a dictation, and records nothing
        // without it.
        let history_privacy = match crate::history::store::privacy_lock(&self.path) {
            Ok(lock) => Some(lock),
            Err(error) => {
                tracing::warn!(
                    "History privacy lock unavailable; saving {} without it: {}",
                    self.path.display(),
                    error
                );
                None
            }
        };
        self.rebase_onto_disk()?;
        let serialized = self.document.to_string();

        // Validate the file-layer selection, not CLI/environment overrides.
        // Inactive legacy tables remain editable/preserved in GGUF-only builds.
        let next: config::Config =
            toml::from_str(&serialized).map_err(|e| EditorError::Validate(e.to_string()))?;
        let previous = fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| toml::from_str::<config::Config>(&text).ok());
        let changed = previous
            .as_ref()
            .is_none_or(|old| old.engine != next.engine || old.model_name() != next.model_name());
        if changed
            && matches!(
                next.engine,
                config::TranscriptionEngine::Parakeet | config::TranscriptionEngine::Cohere
            )
        {
            crate::model_selection::validate_ready(next.engine.name(), next.model_name())
                .map_err(EditorError::Validate)?;
        }

        // Validate before touching the on-disk file: parse the serialized
        // text via the runtime config loader. We do this by writing to a temp
        // file, loading from there, and only renaming on success.
        let current_history = fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| toml::from_str::<config::Config>(&text).ok())
            .map(|config| config.history);
        let invalidates_history = current_history
            .as_ref()
            .map_or(next.history.paused || !next.history.enabled, |old| {
                (!old.paused && next.history.paused) || (old.enabled && !next.history.enabled)
            });
        if invalidates_history && history_privacy.is_some() {
            crate::history::store::advance_privacy_generation(&self.path).map_err(|source| {
                EditorError::Write {
                    path: self.path.clone(),
                    source,
                }
            })?;
        }

        let permissions = match fs::metadata(&target) {
            Ok(metadata) => Some(metadata.permissions()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(EditorError::Write {
                    path: target.clone(),
                    source,
                })
            }
        };

        // Back up the existing on-disk config the first time the TUI saves.
        // toml_edit reformats whitespace, comment placement, and key
        // ordering on round-trip, so even a no-op save can produce a
        // different-looking file. The backup gives users a fast undo if
        // they ran the TUI on a hand-rolled config they liked the shape
        // of. One-shot: existing .bak survives and is never overwritten,
        // so we capture the user's actual original, not the prior TUI run.
        if target.exists() {
            let mut bak = target.clone();
            let mut name = bak
                .file_name()
                .map(|n| n.to_os_string())
                .unwrap_or_default();
            name.push(".bak");
            bak.set_file_name(name);
            if !bak.exists() {
                // Keep the best-effort, first-backup behavior without following
                // a dangling .bak symlink or exposing a newly created backup.
                let _ = (|| -> std::io::Result<()> {
                    let mut backup = tempfile::NamedTempFile::new_in(parent)?;
                    std::io::copy(&mut fs::File::open(&target)?, backup.as_file_mut())?;
                    backup.as_file().sync_all()?;
                    backup
                        .persist_noclobber(&bak)
                        .map_err(|error| error.error)?;
                    Ok(())
                })();
            }
        }

        // NamedTempFile creates a unique 0600 replacement before any config
        // bytes are written, independent of umask and preexisting .tmp paths.
        let mut tmp =
            tempfile::NamedTempFile::new_in(parent).map_err(|source| EditorError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        tmp.write_all(serialized.as_bytes())
            .map_err(|source| EditorError::Write {
                path: tmp.path().to_path_buf(),
                source,
            })?;
        tmp.as_file()
            .sync_all()
            .map_err(|source| EditorError::Write {
                path: tmp.path().to_path_buf(),
                source,
            })?;

        // Validate by loading via the same code path the daemon uses.
        if let Err(e) = config::load_config(Some(tmp.path())) {
            return Err(EditorError::Validate(format!("{}", e)));
        }

        // Restore an existing file's intended permissions only after the
        // sensitive write and validation. A new config remains private 0600.
        if let Some(permissions) = permissions {
            tmp.as_file()
                .set_permissions(permissions)
                .map_err(|source| EditorError::Write {
                    path: tmp.path().to_path_buf(),
                    source,
                })?;
        }
        tmp.as_file()
            .sync_all()
            .map_err(|source| EditorError::Write {
                path: tmp.path().to_path_buf(),
                source,
            })?;
        tmp.persist(&target).map_err(|error| EditorError::Write {
            path: target,
            source: error.error,
        })?;

        self.dirty = false;
        self.loaded_text = serialized;
        self.changes.clear();
        Ok(())
    }
}

/// Whether two TOML values mean the same thing, whatever their formatting.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.value() == y.value(),
        (Value::Boolean(x), Value::Boolean(y)) => x.value() == y.value(),
        (Value::Integer(x), Value::Integer(y)) => x.value() == y.value(),
        (Value::Float(x), Value::Float(y)) => x.value() == y.value(),
        _ => false,
    }
}

fn apply_set(
    document: &mut DocumentMut,
    table: &str,
    key: &str,
    mut value: Value,
    keep_decor: bool,
) {
    if keep_decor {
        let previous = table_in(document, table)
            .and_then(|t| t.get(key))
            .and_then(|i| i.as_value());
        if let Some(previous) = previous {
            *value.decor_mut() = previous.decor().clone();
        }
    }
    match ensure_table_like(document, table) {
        Item::Value(Value::InlineTable(inline)) => set_inline_entry(inline, key, value),
        parent => {
            let parent = parent
                .as_table_like_mut()
                .expect("ensure_table_like returns a table");
            // Assign through the existing slot rather than re-inserting, so
            // the key keeps its place and the comments above it.
            match parent.get_mut(key) {
                Some(slot) => *slot = Item::Value(value),
                None => {
                    parent.insert(key, Item::Value(value));
                }
            }
        }
    }
}

/// Set `key` in an inline table, keeping the table inline.
///
/// The space before the closing `}` is stored as the last value's suffix, so a
/// new last entry takes it over: `{ a = 1, b = 2 }` gains `c` as
/// `{ a = 1, b = 2, c = 3 }`, not `{ a = 1, b = 2 , c = 3 }`.
fn set_inline_entry(inline: &mut toml_edit::InlineTable, key: &str, value: Value) {
    if let Some(slot) = inline.get_mut(key) {
        *slot = value;
        return;
    }
    let trailing = inline.iter_mut().last().and_then(|(_, last)| {
        let suffix = last.decor().suffix().cloned();
        last.decor_mut().set_suffix("");
        suffix
    });
    inline.insert(key, value);
    if let (Some(trailing), Some((_, last))) = (trailing, inline.iter_mut().last()) {
        last.decor_mut().set_suffix(trailing);
    }
}

/// Remove a key; whether it was there.
fn apply_unset(document: &mut DocumentMut, table: &str, key: &str) -> bool {
    match item_mut_in(document, table) {
        Some(Item::Value(Value::InlineTable(inline))) => {
            let was_last = inline.iter().last().is_some_and(|(k, _)| k == key);
            let Some(removed) = inline.remove(key) else {
                return false;
            };
            if was_last {
                let trailing = removed.decor().suffix().cloned();
                if let (Some(trailing), Some((_, last))) = (trailing, inline.iter_mut().last()) {
                    last.decor_mut().set_suffix(trailing);
                }
            }
            true
        }
        Some(item) => item
            .as_table_like_mut()
            .is_some_and(|t| t.remove(key).is_some()),
        None => false,
    }
}

fn apply_overlay(
    document: &mut DocumentMut,
    dotted: &str,
    overlay: &toml_edit::Table,
) -> Vec<OverlayChange> {
    let target = ensure_table(document, dotted)
        .as_table_mut()
        .expect("ensure_table returns a table");
    let mut changes = Vec::new();
    merge_table(target, overlay, dotted, &mut changes);
    changes
}

/// The table at `dotted`, whether written as a `[table]`, with dotted keys or
/// inline (`replacements = { .. }`). A path through anything that is not a
/// table is absent.
fn table_in<'a>(document: &'a DocumentMut, dotted: &str) -> Option<&'a dyn toml_edit::TableLike> {
    let mut current: &dyn toml_edit::TableLike = document.as_table();
    if dotted.is_empty() {
        return Some(current);
    }
    for segment in dotted.split('.') {
        current = current.get(segment)?.as_table_like()?;
    }
    Some(current)
}

/// The item holding the table at `dotted`: an `Item::Table`, or an
/// `Item::Value` holding an inline table.
fn item_mut_in<'a>(document: &'a mut DocumentMut, dotted: &str) -> Option<&'a mut Item> {
    let mut current = document.as_item_mut();
    if dotted.is_empty() {
        return Some(current);
    }
    for segment in dotted.split('.') {
        current = current.as_table_like_mut()?.get_mut(segment)?;
        if !current.is_table_like() {
            return None;
        }
    }
    Some(current)
}

/// Walk to the table at `dotted`, creating a `[table]` for each missing
/// segment, and return the item that holds it.
///
/// A table that already exists is used in whatever form it was written in.
/// Only a missing segment, or one holding a non-table value, is replaced: an
/// inline `replacements = { .. }` must never be swapped for an empty table,
/// which is how adding one replacement used to drop all the others.
fn ensure_table_like<'a>(document: &'a mut DocumentMut, dotted: &str) -> &'a mut Item {
    let mut current = document.as_item_mut();
    if dotted.is_empty() {
        return current;
    }
    for segment in dotted.split('.') {
        let parent = current
            .as_table_like_mut()
            .expect("only tables are walked into");
        if !parent.get(segment).is_some_and(Item::is_table_like) {
            // Inside an inline table this becomes an inline `{}`.
            parent.insert(segment, Item::Table(toml_edit::Table::new()));
        }
        current = parent.get_mut(segment).expect("just ensured");
    }
    current
}

/// Ensure a (possibly dotted) `[table]` path exists in `document` and return
/// it as a mutable Item holding a standard table, creating intermediate
/// tables as needed.
///
/// Inline tables on the path are converted to standard tables with every
/// entry kept, since a standard table can't live inside an inline one.
fn ensure_table<'a>(document: &'a mut DocumentMut, dotted: &str) -> &'a mut Item {
    assert!(!dotted.is_empty(), "ensure_table called with empty path");
    let mut current = document.as_item_mut();
    for segment in dotted.split('.') {
        let parent = current.as_table_mut().expect("only tables are walked into");
        match parent.get_mut(segment) {
            Some(item) if item.is_inline_table() => {
                let inline = std::mem::take(item);
                *item = Item::Table(
                    inline
                        .into_table()
                        .expect("an inline table converts to a table"),
                );
            }
            Some(item) if item.is_table() => {}
            _ => {
                parent.insert(segment, Item::Table(toml_edit::Table::new()));
            }
        }
        current = parent.get_mut(segment).expect("just ensured");
    }
    current
}

/// One leaf [`ConfigEditor::overlay_table`] changed: its full dotted key and
/// its value before (`None` when it was unset) and after, rendered as TOML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayChange {
    pub key: String,
    pub before: Option<String>,
    pub after: String,
}

fn merge_table(
    target: &mut toml_edit::Table,
    overlay: &dyn toml_edit::TableLike,
    prefix: &str,
    changes: &mut Vec<OverlayChange>,
) {
    for (key, item) in overlay.iter() {
        let path = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };

        if let Some(src) = item.as_table_like() {
            // Normalise the destination to a standard table so its existing
            // keys survive the merge (an inline `frame = { .. }` included).
            let existing = target.remove(key);
            let mut dest = match existing {
                Some(existing) => existing
                    .into_table()
                    .unwrap_or_else(|_| toml_edit::Table::new()),
                None => toml_edit::Table::new(),
            };
            if dest.is_empty() {
                // Headerless when the overlay only nests deeper tables, so
                // `[[osd.visual.layers]]` doesn't grow an empty `[osd.visual]`.
                dest.set_implicit(true);
            }
            merge_table(&mut dest, src, &path, changes);
            target.insert(key, Item::Table(dest));
            continue;
        }

        let after = detached(item);
        let before = target.get(key).filter(|i| !i.is_none());
        if before.is_some_and(|b| semantic(b) == semantic(&after)) {
            continue;
        }
        changes.push(OverlayChange {
            key: path,
            before: before.map(render),
            after: render(&after),
        });
        target.insert(key, after);
    }
}

/// Copy of `item` with fresh tables, so tables lifted from another document
/// don't carry that document's positions into this one's table ordering.
fn detached(item: &Item) -> Item {
    fn table(src: &toml_edit::Table) -> toml_edit::Table {
        let mut t = toml_edit::Table::new();
        t.set_implicit(src.is_implicit());
        for (k, v) in src.iter() {
            t.insert(k, detached(v));
        }
        t
    }
    match item {
        Item::Table(t) => Item::Table(table(t)),
        Item::ArrayOfTables(a) => {
            let mut out = toml_edit::ArrayOfTables::new();
            for t in a.iter() {
                out.push(table(t));
            }
            Item::ArrayOfTables(out)
        }
        other => other.clone(),
    }
}

/// The parsed value of `item`, ignoring formatting and comments, so an
/// overlay that restates an existing value isn't reported as a change.
fn semantic(item: &Item) -> Option<toml::Value> {
    let mut doc = DocumentMut::new();
    doc.insert("v", item.clone());
    toml::from_str::<toml::Table>(&doc.to_string())
        .ok()?
        .remove("v")
}

fn render(item: &Item) -> String {
    match item {
        Item::ArrayOfTables(a) => format!("{} table(s)", a.len()),
        Item::Value(v) => {
            let mut v = v.clone();
            v.decor_mut().clear();
            v.to_string()
        }
        other => other.to_string().trim().to_string(),
    }
}

/// The `f64` that prints as `value` does: `f32`'s `Display` gives the fewest
/// digits that read back as the same `f32`, and parsing those as an `f64`
/// gives the double a person would have typed for it.
fn shortest_f64(value: f32) -> f64 {
    value.to_string().parse().unwrap_or(f64::from(value))
}

/// Round an `f32` setting to two decimal places.
///
/// The forms step a value with `f32` arithmetic, and the sum is not always
/// the `f32` nearest the decimal it stands for: 0.6 + 0.1 is 0.70000005,
/// one step above the `f32` for 0.7, and that is what was saved. Dividing a
/// whole number by 100 is correctly rounded, so the result is the `f32`
/// nearest the two-decimal value and is written back as those two decimals.
pub fn round_to_hundredths(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
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

    /// A GNU Stow layout: `config.toml` in the config dir is a link into a
    /// dotfiles tree. Returns (tempdir, link, the file it names).
    fn stowed_config(contents: &str, link_text: &Path) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let dotfiles = dir.path().join("dotfiles/voxtype");
        let config_dir = dir.path().join("config/voxtype");
        fs::create_dir_all(&dotfiles).unwrap();
        fs::create_dir_all(&config_dir).unwrap();
        let target = dotfiles.join("config.toml");
        fs::write(&target, contents).unwrap();
        let link = config_dir.join("config.toml");
        std::os::unix::fs::symlink(link_text, &link).unwrap();
        (dir, link, target)
    }

    fn save_model(path: &Path, model: &str) -> Result<(), EditorError> {
        let mut editor = ConfigEditor::load_from_path(path.to_path_buf())?;
        editor.set_string("whisper", "model", model);
        editor.save()
    }

    /// #1102: saving through a relative link writes the file it names and
    /// keeps the link, so a Stow-managed dotfiles tree stays the source.
    #[test]
    fn save_preserves_relative_config_symlink() {
        let relative = Path::new("../../dotfiles/voxtype/config.toml");
        let (_dir, link, target) = stowed_config("engine = \"whisper\"\n", relative);

        save_model(&link, "large-v3").unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), relative);
        assert!(fs::read_to_string(&target).unwrap().contains("large-v3"));

        // The editor sees its own write, and a second save still goes through.
        save_model(&link, "small").unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), relative);
        assert!(fs::read_to_string(&target).unwrap().contains("small"));
    }

    #[test]
    fn save_preserves_absolute_config_symlink() {
        let (_dir, link, target) = stowed_config("engine = \"whisper\"\n", Path::new("unset"));
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        save_model(&link, "large-v3").unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), target);
        assert!(fs::read_to_string(&target).unwrap().contains("large-v3"));
    }

    /// Each hop of a chain is relative to the link that holds it, and only
    /// the last file is rewritten.
    #[test]
    fn save_follows_a_chain_of_links_to_the_last_file() {
        let relative = Path::new("../../dotfiles/voxtype/config.toml");
        let (dir, link, middle) = stowed_config("", relative);
        let store = dir.path().join("store");
        fs::create_dir(&store).unwrap();
        let file = store.join("real.toml");
        fs::write(&file, "engine = \"whisper\"\n").unwrap();
        fs::remove_file(&middle).unwrap();
        std::os::unix::fs::symlink("../../store/real.toml", &middle).unwrap();

        save_model(&link, "large-v3").unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), relative);
        assert_eq!(
            fs::read_link(&middle).unwrap(),
            Path::new("../../store/real.toml")
        );
        assert!(fs::read_to_string(&file).unwrap().contains("large-v3"));
    }

    /// A dangling link (the dotfiles file was removed) is saved by creating
    /// the file it names, as a missing config is created. The link then
    /// works again instead of being replaced by a stray copy.
    #[test]
    fn save_through_a_dangling_link_creates_its_target() {
        let relative = Path::new("../../dotfiles/voxtype/config.toml");
        let (dir, link, target) = stowed_config("", relative);
        fs::remove_dir_all(dir.path().join("dotfiles")).unwrap();

        save_model(&link, "large-v3").unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), relative);
        assert!(fs::read_to_string(&target).unwrap().contains("large-v3"));
        // There was nothing to back up, and a new config stays private.
        assert!(!target.with_file_name("config.toml.bak").exists());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn save_through_a_link_loop_fails_and_leaves_the_links() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.toml");
        let b = dir.path().join("b.toml");
        std::os::unix::fs::symlink("b.toml", &a).unwrap();
        std::os::unix::fs::symlink("a.toml", &b).unwrap();
        assert!(save_model(&a, "large-v3").is_err());
        assert_eq!(fs::read_link(&a).unwrap(), Path::new("b.toml"));
        assert_eq!(fs::read_link(&b).unwrap(), Path::new("a.toml"));
    }

    /// The target's mode is what a save keeps: the link's own mode is
    /// meaningless on Linux and must not leak onto the file.
    #[test]
    fn save_through_a_link_keeps_the_target_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let relative = Path::new("../../dotfiles/voxtype/config.toml");
        let (_dir, link, target) = stowed_config("engine = \"whisper\"\n", relative);
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();

        save_model(&link, "large-v3").unwrap();
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    /// The one-shot backup copies the file the link names and sits beside
    /// it, not beside the link.
    #[test]
    fn save_through_a_link_backs_up_the_target() {
        let relative = Path::new("../../dotfiles/voxtype/config.toml");
        let original = "# hand-rolled\nengine = \"whisper\"\n";
        let (_dir, link, target) = stowed_config(original, relative);

        save_model(&link, "large-v3").unwrap();
        let bak = target.with_file_name("config.toml.bak");
        assert_eq!(fs::read_to_string(&bak).unwrap(), original);
        assert!(!link.with_file_name("config.toml.bak").exists());
    }

    /// The daemon may know the config by its link and a CLI by the file it
    /// names. A pause saved through either must invalidate dictations the
    /// other started, so both key the privacy generation on the same file.
    #[test]
    fn a_pause_through_a_link_advances_the_target_generation() {
        let relative = Path::new("../../dotfiles/voxtype/config.toml");
        let (_dir, link, target) =
            stowed_config("[history]\nenabled = true\npaused = false\n", relative);
        assert_eq!(
            crate::history::store::privacy_generation(&target).unwrap(),
            0
        );
        let mut editor = ConfigEditor::load_from_path(link.clone()).unwrap();
        editor.set_bool("history", "paused", true);
        editor.save().unwrap();
        assert_eq!(
            crate::history::store::privacy_generation(&target).unwrap(),
            1
        );
        assert_eq!(crate::history::store::privacy_generation(&link).unwrap(), 1);
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

    const SHARED: &str = "# keep this comment\nengine = \"whisper\"\n\n[history]\nenabled = true\npaused = false\n\n[audio]\nmax_duration_secs = 60\n";

    fn saved(path: &Path) -> (config::Config, String) {
        let text = fs::read_to_string(path).unwrap();
        (toml::from_str(&text).unwrap(), text)
    }

    /// Another process writes `table.key` while an editor is open.
    fn written_meanwhile(path: &Path, table: &str, key: &str, value: bool) {
        let mut other = ConfigEditor::load_from_path(path.to_path_buf()).unwrap();
        other.set_bool(table, key, value);
        other.save().unwrap();
    }

    /// A data dir that cannot be created, as in the Nix build sandbox, where
    /// HOME is /homeless-shelter.
    fn unwritable_data_dir(path: &Path) {
        let blocker = path.parent().unwrap().join("not-a-directory");
        fs::write(&blocker, "").unwrap();
        crate::history::store::use_data_dir_for(path, &blocker.join("data"));
    }

    #[test]
    fn saves_when_the_data_dir_is_unwritable() {
        let (_dir, path) = temp_config(SHARED);
        unwritable_data_dir(&path);
        let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        ed.set_int("audio", "max_duration_secs", 90);
        ed.set_bool("history", "paused", true);
        ed.save().unwrap();

        let (config, text) = saved(&path);
        assert_eq!(config.audio.max_duration_secs, 90);
        assert!(config.history.paused, "the pause was not saved");
        assert!(text.contains("# keep this comment"));
    }

    #[test]
    fn a_stale_save_keeps_a_pause_made_in_between() {
        let (_dir, path) = temp_config(SHARED);
        let mut stale = ConfigEditor::load_from_path(path.clone()).unwrap();
        stale.set_int("audio", "max_duration_secs", 90);
        written_meanwhile(&path, "history", "paused", true);
        stale.save().unwrap();

        let (config, text) = saved(&path);
        assert!(config.history.paused, "the pause was written over");
        assert_eq!(config.audio.max_duration_secs, 90);
        assert!(text.contains("# keep this comment"));
    }

    #[test]
    fn a_stale_save_keeps_history_turned_off_in_between() {
        let (_dir, path) = temp_config(SHARED);
        let mut stale = ConfigEditor::load_from_path(path.clone()).unwrap();
        stale.set_int("audio", "max_duration_secs", 90);
        written_meanwhile(&path, "history", "enabled", false);
        stale.save().unwrap();

        let (config, _) = saved(&path);
        assert!(!config.history.enabled, "history was turned back on");
        assert_eq!(config.audio.max_duration_secs, 90);
    }

    #[test]
    fn a_form_writing_back_what_it_showed_does_not_undo_a_pause() {
        let (_dir, path) = temp_config(SHARED);
        let mut form = ConfigEditor::load_from_path(path.clone()).unwrap();
        // A settings page saves every field it shows, changed or not.
        form.set_bool("history", "enabled", true);
        form.set_bool("history", "paused", false);
        form.set_int("audio", "max_duration_secs", 90);
        written_meanwhile(&path, "history", "paused", true);
        form.save().unwrap();

        let (config, _) = saved(&path);
        assert!(config.history.paused, "an unchanged field undid the pause");
        assert_eq!(config.audio.max_duration_secs, 90);
    }

    #[test]
    fn a_request_to_pause_wins_over_a_resume_made_in_between() {
        let (_dir, path) = temp_config(&SHARED.replace("paused = false", "paused = true"));
        let mut pausing = ConfigEditor::load_from_path(path.clone()).unwrap();
        // Already paused as far as this editor can see, and asked for again.
        pausing.set_bool("history", "paused", true);
        written_meanwhile(&path, "history", "paused", false);
        pausing.save().unwrap();

        assert!(saved(&path).0.history.paused, "a pause request was lost");
    }

    #[test]
    fn a_later_resume_still_resumes() {
        let (_dir, path) = temp_config(&SHARED.replace("paused = false", "paused = true"));
        let mut resuming = ConfigEditor::load_from_path(path.clone()).unwrap();
        resuming.set_bool("history", "paused", false);
        // Something unrelated is written while the resume is in flight.
        let mut other = ConfigEditor::load_from_path(path.clone()).unwrap();
        other.set_int("audio", "max_duration_secs", 45);
        other.save().unwrap();
        resuming.save().unwrap();

        let (config, _) = saved(&path);
        assert!(!config.history.paused, "privacy state must not be one-way");
        assert_eq!(config.audio.max_duration_secs, 45);
    }

    #[test]
    fn a_stale_unset_removes_only_its_own_key() {
        let (_dir, path) = temp_config(SHARED);
        let mut stale = ConfigEditor::load_from_path(path.clone()).unwrap();
        stale.unset("audio", "max_duration_secs");
        written_meanwhile(&path, "history", "paused", true);
        stale.save().unwrap();

        let (config, text) = saved(&path);
        assert!(!text.contains("max_duration_secs"));
        assert!(config.history.paused);
    }

    #[test]
    fn the_later_save_wins_when_both_changed_the_same_key() {
        let (_dir, path) = temp_config(SHARED);
        let mut first = ConfigEditor::load_from_path(path.clone()).unwrap();
        first.set_int("audio", "max_duration_secs", 90);
        let mut second = ConfigEditor::load_from_path(path.clone()).unwrap();
        second.set_int("audio", "max_duration_secs", 30);
        second.save().unwrap();
        first.save().unwrap();

        assert_eq!(saved(&path).0.audio.max_duration_secs, 90);
    }

    #[test]
    fn a_second_save_from_the_same_editor_does_not_replay_old_changes() {
        let (_dir, path) = temp_config(SHARED);
        let mut editor = ConfigEditor::load_from_path(path.clone()).unwrap();
        editor.set_bool("history", "paused", true);
        editor.save().unwrap();
        written_meanwhile(&path, "history", "paused", false);
        editor.set_int("audio", "max_duration_secs", 90);
        editor.save().unwrap();

        let (config, _) = saved(&path);
        assert!(
            !config.history.paused,
            "an already-saved pause was replayed"
        );
        assert_eq!(config.audio.max_duration_secs, 90);
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
    fn inline_tables_read_and_write_in_place() {
        let (_dir, path) = temp_config(
            "[text]\nreplacements = { \"vox type\" = \"Voxtype\" }\nosd = { position = \"top\" }\n",
        );
        let mut ed = ConfigEditor::load_from(path).unwrap();
        assert_eq!(
            ed.get_string("text.replacements", "vox type").as_deref(),
            Some("Voxtype")
        );
        let table = ed
            .raw_table("text.replacements")
            .expect("inline table reads");
        assert_eq!(table.len(), 1);

        ed.set_string("text.replacements", "btw", "by the way");
        assert!(ed
            .document
            .to_string()
            .contains("replacements = { \"vox type\" = \"Voxtype\", btw = \"by the way\" }"));
        ed.unset("text.replacements", "vox type");
        assert!(ed
            .document
            .to_string()
            .contains("replacements = { btw = \"by the way\" }"));
    }

    #[test]
    fn overlay_onto_an_inline_table_keeps_its_entries() {
        let (_dir, path) = temp_config("osd = { position = \"top-right\", opacity = 0.5 }\n");
        let mut ed = ConfigEditor::load_from(path).unwrap();
        let mut overlay = toml_edit::Table::new();
        overlay.insert("opacity", toml_edit::value(0.8));
        ed.overlay_table("osd", &overlay);
        let doc: toml::Table = ed.document.to_string().parse().unwrap();
        assert_eq!(doc["osd"]["position"].as_str(), Some("top-right"));
        assert_eq!(doc["osd"]["opacity"].as_float(), Some(0.8));
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

    /// The configure TUI saved `volume = 0.7000000476837158` after a change
    /// to Audio > Volume: an `f32` widened to `f64` and printed in full.
    #[test]
    fn set_f32_writes_the_shortest_number_that_reads_back_the_same() {
        let (_dir, path) = temp_config("");
        let mut ed = ConfigEditor::load_from(path).unwrap();
        for (value, written) in [
            (0.7_f32, "0.7"),
            (0.1, "0.1"),
            (0.05, "0.05"),
            (0.35, "0.35"),
            (0.005, "0.005"),
            (1.0, "1.0"),
            (0.0, "0.0"),
            // Not the f32 nearest 0.7, so it keeps the digits that tell it
            // apart, and no more than that.
            (0.6_f32 + 0.1_f32, "0.70000005"),
        ] {
            ed.set_f32("audio.feedback", "volume", value);
            let serialized = ed.document.to_string();
            assert!(
                serialized.contains(&format!("volume = {written}\n")),
                "{value:?} should be written as {written}, got: {serialized}"
            );
            assert_eq!(ed.get_f32_or("audio.feedback", "volume", -1.0), value);
        }
    }

    #[test]
    fn round_to_hundredths_lands_on_the_f32_for_the_two_decimal_value() {
        // Every sum a form can produce by stepping across 0..=1, in either
        // direction and from either step size, written out and read back.
        let (_dir, path) = temp_config("");
        let mut ed = ConfigEditor::load_from(path).unwrap();
        for step in [0.1_f32, 0.05] {
            for direction in [1.0_f32, -1.0] {
                let mut value: f32 = if direction > 0.0 { 0.0 } else { 1.0 };
                for _ in 0..=20 {
                    value = round_to_hundredths(value + direction * step).clamp(0.0, 1.0);
                    ed.set_f32("vad", "threshold", value);
                    let serialized = ed.document.to_string();
                    let written = serialized
                        .trim()
                        .rsplit("threshold = ")
                        .next()
                        .unwrap()
                        .to_string();
                    let decimals = written.split('.').nth(1).map_or(0, str::len);
                    assert!(
                        decimals <= 2,
                        "stepping by {step} wrote {written} for {value:?}"
                    );
                }
            }
        }
        assert_eq!(round_to_hundredths(0.6 + 0.1), 0.7);
        assert_eq!(round_to_hundredths(0.433), 0.43);
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
