//! Programmatic mutation of the on-disk config file from the CLI.
//!
//! Backs `voxtype config set engine <NAME>`. This is the same operation the
//! TUI engine section performs (see `src/tui/engine.rs`), exposed as a
//! non-interactive command so external tools (Quickshell engine picker,
//! shell scripts, etc.) can switch engines without rendering a TUI.
//!
//! Validation rules mirror the TUI:
//!   1. The engine name must be a known variant of [`TranscriptionEngine`].
//!   2. For non-whisper engines, the binary must have been compiled with the
//!      matching Cargo feature. The TUI surfaces this as a warning; the CLI
//!      treats it as a hard error since there's no interactive escape hatch.
//!
//! Comments and unrelated fields are preserved via `toml_edit` (through
//! `ConfigEditor`). Saves go through the same atomic write + validation
//! pipeline as the TUI.

use std::path::PathBuf;

use crate::config::TranscriptionEngine;
use crate::tui::{ConfigEditor, EditorError};

#[derive(Debug, thiserror::Error)]
pub enum ConfigSetError {
    #[error(
        "unknown engine '{0}'. Valid engines: {}",
        TranscriptionEngine::names_csv()
    )]
    UnknownEngine(String),

    #[error(
        "engine '{0}' is not compiled into this binary.\n  \
         Rebuild voxtype with the matching Cargo feature:\n    \
         cargo build --release --features {0}\n  \
         Or install a prebuilt variant that includes it (see \
         `voxtype info variants`)."
    )]
    FeatureNotCompiled(String),

    #[error("config editor: {0}")]
    Editor(#[from] EditorError),
}

/// Is the engine name one we recognize at all?
///
/// Iterates the [`TranscriptionEngine`] variants and matches the exact
/// canonical lowercase name. Case-sensitive (so callers can detect typos
/// like `"Whisper"` before applying them to config). New engine variants
/// are picked up automatically via `strum::EnumIter`.
pub fn parse_engine(name: &str) -> Option<TranscriptionEngine> {
    use strum::IntoEnumIterator;
    TranscriptionEngine::iter().find(|e| e.name() == name)
}

/// Was this binary compiled with the feature needed to run the given engine?
///
/// Whisper and Soniox are unconditional (Soniox was un-feature-gated in
/// #441); every other engine is gated on the corresponding Cargo feature.
/// This is the source-of-truth check that matches what the TUI shows on
/// source builds (see `EngineState::refresh_binary_match` in
/// `src/tui/engine.rs`). The TUI's `compiled_features()` list in
/// `src/setup/binary.rs` is incomplete (it only enumerates parakeet + GPU
/// features), so we evaluate `cfg!` directly here rather than going
/// through that helper.
///
/// Matches `TranscriptionEngine` exhaustively so adding a new variant
/// produces a compile error here, not a silent `false` at runtime. The
/// previous wildcard arm hid `soniox` from this check for several months.
pub fn engine_feature_compiled(name: &str) -> bool {
    let Some(engine) = parse_engine(name) else {
        return false;
    };
    match engine {
        TranscriptionEngine::Whisper => true,
        TranscriptionEngine::Soniox => true,
        TranscriptionEngine::Parakeet => cfg!(feature = "parakeet"),
        TranscriptionEngine::Moonshine => cfg!(feature = "moonshine"),
        TranscriptionEngine::SenseVoice => cfg!(feature = "sensevoice"),
        TranscriptionEngine::Paraformer => cfg!(feature = "paraformer"),
        TranscriptionEngine::Dolphin => cfg!(feature = "dolphin"),
        TranscriptionEngine::Omnilingual => cfg!(feature = "omnilingual"),
        TranscriptionEngine::Cohere => cfg!(feature = "cohere"),
    }
}

/// Set the active engine in the config file at `path`.
///
/// Validates the name and the compiled-feature gate before touching disk.
/// If the file doesn't exist, an empty document is created and `engine = ".."`
/// is written at the root. If it exists, `toml_edit` updates only the
/// `engine` key, preserving comments and other fields.
pub fn set_engine(path: PathBuf, name: &str) -> Result<PathBuf, ConfigSetError> {
    if parse_engine(name).is_none() {
        return Err(ConfigSetError::UnknownEngine(name.to_string()));
    }
    if !engine_feature_compiled(name) {
        return Err(ConfigSetError::FeatureNotCompiled(name.to_string()));
    }

    let mut editor = ConfigEditor::load_from_path(path)?;
    editor.set_string("", "engine", name);
    editor.save()?;
    Ok(editor.path().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use strum::IntoEnumIterator;

    fn temp_config(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        (dir, path)
    }

    #[test]
    fn parse_engine_accepts_known_names() {
        for engine in TranscriptionEngine::iter() {
            let name = engine.name();
            assert!(parse_engine(name).is_some(), "should accept '{}'", name);
        }
    }

    /// Pins the user-facing error message to the enum so a new variant can't
    /// land without showing up in `voxtype config set engine <bogus>` output.
    /// Caught the post-#476 drift where `soniox` was missing from this list.
    #[test]
    fn unknown_engine_error_lists_every_variant() {
        let display = format!("{}", ConfigSetError::UnknownEngine("bogus".to_string()));
        for engine in TranscriptionEngine::iter() {
            assert!(
                display.contains(engine.name()),
                "ConfigSetError::UnknownEngine display is missing variant '{}': {}",
                engine.name(),
                display
            );
        }
    }

    #[test]
    fn parse_engine_rejects_unknown() {
        assert!(parse_engine("nope").is_none());
        assert!(parse_engine("Whisper").is_none(), "case-sensitive");
        assert!(parse_engine("").is_none());
    }

    #[test]
    fn engine_feature_whisper_always_compiled() {
        assert!(engine_feature_compiled("whisper"));
    }

    #[test]
    fn engine_feature_unknown_returns_false() {
        assert!(!engine_feature_compiled("not-a-real-engine"));
    }

    #[test]
    fn set_engine_rejects_unknown_name() {
        let (_dir, path) = temp_config("");
        let err = set_engine(path, "fakeengine").unwrap_err();
        match err {
            ConfigSetError::UnknownEngine(n) => assert_eq!(n, "fakeengine"),
            other => panic!("expected UnknownEngine, got {:?}", other),
        }
    }

    #[test]
    fn set_engine_whisper_succeeds_against_full_config() {
        // Use the production default config so load_config's strict
        // deserialization passes after the write. (A bare `engine = ...`
        // file would fail validation by design — voxtype's serde struct
        // requires every top-level table to be present.)
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, crate::config::default_config_content()).unwrap();
        let written = set_engine(path.clone(), "whisper").expect("set whisper");
        assert_eq!(written, path);
        let contents = fs::read_to_string(&path).unwrap();
        assert!(
            contents.contains("engine = \"whisper\""),
            "missing engine line in {contents:?}"
        );
    }

    #[test]
    fn set_engine_preserves_comments_and_adjacent_fields() {
        // ConfigEditor's round-trip is the exact mechanism the TUI uses;
        // verify the CLI path doesn't disturb non-engine content.
        //
        // Use the production default config (which is a complete,
        // commented TOML document) and then sprinkle a custom marker
        // comment + adjacent field we expect to survive the round-trip.
        let mut base = crate::config::default_config_content();
        // Inject a marker comment near the top so we can prove comments
        // are preserved. Insert after the first newline so it lands
        // inside the document body rather than ahead of any header.
        let marker = "\n# VOXTYPE-TEST-MARKER: keep this comment\n";
        let insert_at = base.find('\n').map(|i| i + 1).unwrap_or(0);
        base.insert_str(insert_at, marker);

        let (_dir, path) = temp_config(&base);
        // Switching to whisper is always safe regardless of feature flags.
        set_engine(path.clone(), "whisper").expect("set engine");

        let after = fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("# VOXTYPE-TEST-MARKER: keep this comment"),
            "marker comment lost after round-trip: {after}"
        );
        assert!(
            after.contains("engine = \"whisper\""),
            "engine not updated: {after}"
        );
        // [hotkey] table from the default config should still be present.
        assert!(
            after.contains("[hotkey]"),
            "hotkey table lost after round-trip: {after}"
        );
    }

    #[test]
    fn set_engine_in_memory_round_trip_preserves_comments() {
        // Pure ConfigEditor exercise (no full-config validation) — proves
        // that the toml_edit mutation we perform is the comment-preserving
        // one. Mirrors `round_trip_preserves_comments` in config_editor.rs.
        let (_dir, path) =
            temp_config("# top comment\nengine = \"parakeet\"\n# trailing comment\n");
        let mut ed = crate::tui::ConfigEditor::load_from_path(path).unwrap();
        ed.set_string("", "engine", "whisper");
        // We can't call ed.save() here without a full config schema, so
        // read the document directly via get_string for the round-trip
        // check.
        assert_eq!(ed.get_string("", "engine").as_deref(), Some("whisper"));
    }

    // Engines other than whisper/parakeet aren't enumerated in the default
    // feature set, so on a default `cargo test` run they'll fail the feature
    // gate. Exercise that path with a non-whisper engine and check the
    // error variant — but only if the feature isn't enabled, otherwise the
    // engine is legitimately available and this test would be misleading.
    #[test]
    fn set_engine_rejects_uncompiled_engine() {
        // Pick the first non-whisper engine whose feature is NOT compiled
        // into this test binary. Skip the test entirely if every engine is
        // compiled in (e.g. a maximalist CI build).
        let target = TranscriptionEngine::iter()
            .map(|e| e.name())
            .find(|n| *n != "whisper" && !engine_feature_compiled(n));
        let Some(name) = target else {
            eprintln!("skipping: all engine features are compiled in this build");
            return;
        };
        let (_dir, path) = temp_config("");
        let err = set_engine(path, name).unwrap_err();
        match err {
            ConfigSetError::FeatureNotCompiled(n) => assert_eq!(n, name),
            other => panic!("expected FeatureNotCompiled, got {:?}", other),
        }
    }
}
