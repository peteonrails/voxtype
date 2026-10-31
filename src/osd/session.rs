//! Runtime files that tell OSD frontends about the session in flight.
//!
//! The state file carries one word (`recording`, `transcribing`, ...) and
//! that contract went stable in 1.0.0, so everything else an OSD wants to
//! show is published in sibling files in the runtime directory instead:
//!
//! - `recording.json` exists from the moment a recording or streaming
//!   session starts until the daemon returns to idle. It carries the wall
//!   clock start time and the effective `max_duration_secs`, so a frontend
//!   can show elapsed time and a countdown that survive an OSD restart, plus
//!   the engine and model serving this recording (#854).
//! - `outcome.json` is rewritten each time a session ends, saying how it
//!   ended (text delivered, no speech, error, cancelled, ...) so a frontend
//!   can confirm the result instead of silently disappearing.
//!
//! Both are written atomically (staged, then renamed), so a reader never
//! sees a half-written file. Write failures are logged and otherwise
//! ignored: these files are informational and must never affect dictation.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

pub const SESSION_FILE: &str = "recording.json";
pub const OUTCOME_FILE: &str = "outcome.json";

/// The session in flight, as published in `recording.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OsdSession {
    /// `recording` or `streaming`.
    pub mode: String,
    /// Wall-clock start, Unix milliseconds.
    pub started_at_ms: u64,
    /// Effective recording limit (config < env < CLI), in seconds.
    pub max_duration_secs: u64,
    /// Engine serving this recording, lowercase (e.g. `whisper`).
    pub engine: String,
    /// Model serving this recording, after any `--model` override.
    pub model: String,
}

/// How a session ended, as published in `outcome.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    /// Text was transcribed and delivered.
    Ok,
    /// Voice activity detection found no speech; nothing was transcribed.
    NoSpeech,
    /// The recording was too short to be deliberate (an accidental tap).
    TooShort,
    /// Transcription ran but produced no text.
    Empty,
    /// The user cancelled.
    Cancelled,
    /// Transcription or model loading failed.
    Error,
    /// Transcription succeeded but no output method could deliver it.
    OutputFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OsdOutcome {
    pub outcome: OutcomeKind,
    /// Characters delivered; zero unless `outcome` is `ok`.
    pub chars: usize,
    /// True when the session was ended by `max_duration_secs`, not the user.
    pub limit_reached: bool,
    /// When the session ended, Unix milliseconds. Frontends use it to tell a
    /// fresh outcome from one left over from an earlier session.
    pub at_ms: u64,
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn write_session(dir: &Path, session: &OsdSession) {
    write_json_atomic(&dir.join(SESSION_FILE), session);
}

pub fn clear_session(dir: &Path) {
    let path = dir.join(SESSION_FILE);
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::warn!("Failed to remove {:?}: {}", path, e);
        }
    }
}

pub fn write_outcome(dir: &Path, outcome: &OsdOutcome) {
    write_json_atomic(&dir.join(OUTCOME_FILE), outcome);
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) {
    let body = match serde_json::to_string(value) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!("Failed to encode {:?}: {}", path, e);
            return;
        }
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("Failed to create {:?}: {}", parent, e);
            return;
        }
    }
    let staged = staged_path(path);
    if let Err(e) = std::fs::write(&staged, format!("{}\n", body)) {
        tracing::warn!("Failed to stage {:?}: {}", staged, e);
        return;
    }
    if let Err(e) = std::fs::rename(&staged, path) {
        tracing::warn!("Failed to publish {:?}: {}", path, e);
        let _ = std::fs::remove_file(&staged);
    }
}

/// Hidden sibling in the same directory, so the rename never crosses a
/// filesystem. The pid keeps two daemons from colliding on it.
fn staged_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{}.{}.tmp", name, std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> OsdSession {
        OsdSession {
            mode: "recording".to_string(),
            started_at_ms: 1_700_000_000_000,
            max_duration_secs: 60,
            engine: "whisper".to_string(),
            model: "base.en".to_string(),
        }
    }

    #[test]
    fn session_round_trips_as_json_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        write_session(dir.path(), &session());

        let text = std::fs::read_to_string(dir.path().join(SESSION_FILE)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["mode"], "recording");
        assert_eq!(json["started_at_ms"], 1_700_000_000_000u64);
        assert_eq!(json["max_duration_secs"], 60);
        assert_eq!(json["engine"], "whisper");
        assert_eq!(json["model"], "base.en");

        clear_session(dir.path());
        assert!(!dir.path().join(SESSION_FILE).exists());
        // Clearing again (already idle) is a no-op, not an error.
        clear_session(dir.path());
    }

    #[test]
    fn outcome_kinds_serialize_snake_case() {
        let dir = tempfile::tempdir().unwrap();
        write_outcome(
            dir.path(),
            &OsdOutcome {
                outcome: OutcomeKind::NoSpeech,
                chars: 0,
                limit_reached: true,
                at_ms: 42,
            },
        );
        let text = std::fs::read_to_string(dir.path().join(OUTCOME_FILE)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["outcome"], "no_speech");
        assert_eq!(json["limit_reached"], true);
        assert_eq!(json["at_ms"], 42);
        assert_eq!(
            serde_json::to_value(OutcomeKind::OutputFailed).unwrap(),
            "output_failed"
        );
    }

    #[test]
    fn writes_leave_no_staging_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        write_session(dir.path(), &session());
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec![SESSION_FILE.to_string()]);
    }

    #[test]
    fn creates_a_missing_runtime_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("voxtype");
        write_session(&nested, &session());
        assert!(nested.join(SESSION_FILE).exists());
    }
}
