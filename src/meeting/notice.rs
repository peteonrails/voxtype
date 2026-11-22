//! The recording-consent notice.
//!
//! Recording or transcribing a conversation can need the other people's
//! agreement. Unless the recording announcement is on (`[meeting]
//! announce_recording`), voxtype tells them nothing on the user's behalf,
//! and even then it asks nobody for consent. So the user is told, until they say they have read it: `voxtype meeting
//! start` prints the notice on stderr, `meeting.sock` carries
//! `recording_notice: true` for the meeting views, and `voxtype meeting
//! notice --ack` records the acknowledgement in
//! `$XDG_STATE_HOME/voxtype/recording-notice.json`, like the assistant's
//! consent. Nothing ever blocks on it.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The notice, word for word, wherever it is shown, while the meeting does
/// not announce itself.
pub const NOTICE: &str = "Laws on recording and transcribing conversations differ by place, and \
some require everyone on the call to agree. Voxtype doesn't tell the other people for you, so let \
them know you're using a meeting assistant.";

/// The notice while the recording announcement is on: [`NOTICE`] would
/// then be untrue.
pub const NOTICE_ANNOUNCED: &str = "Voxtype will announce the recording to the call. You're \
still responsible for getting consent where the law requires it.";

/// The notice to show, given whether the meeting announces the recording.
pub fn text(announce: bool) -> &'static str {
    if announce {
        NOTICE_ANNOUNCED
    } else {
        NOTICE
    }
}

/// How to stop seeing it, for the terminal.
pub const ACK_HINT: &str = "Run `voxtype meeting notice --ack` to stop showing this.";

pub const NOTICE_FILE: &str = "recording-notice.json";

/// Dropped in the runtime directory by `meeting notice --ack`, so a running
/// daemon tells the meeting views at once.
pub const ACK_TRIGGER: &str = "meeting_notice_ack";

/// What has been recorded about the notice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeState {
    /// When the user acknowledged it; never, until they do.
    #[serde(default)]
    pub acknowledged_at: Option<DateTime<Utc>>,
}

impl NoticeState {
    pub fn acknowledged(&self) -> bool {
        self.acknowledged_at.is_some()
    }
}

pub fn path() -> PathBuf {
    crate::config::Config::state_dir().join(NOTICE_FILE)
}

/// The saved state. A missing or unreadable file reads as not acknowledged,
/// so an old install, or a damaged file, shows the notice again.
pub fn load(path: &Path) -> NoticeState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Record the acknowledgement. Acknowledging again keeps the first time.
pub fn acknowledge(path: &Path) -> std::io::Result<NoticeState> {
    let mut state = load(path);
    if state.acknowledged_at.is_none() {
        state.acknowledged_at = Some(Utc::now());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&state)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(state)
}

/// Should the notice still be shown?
pub fn pending() -> bool {
    !load(&path()).acknowledged()
}

/// `voxtype meeting notice --json`, worded for whether meetings announce.
pub fn to_json(state: &NoticeState, announce: bool) -> serde_json::Value {
    serde_json::json!({
        "notice": text(announce),
        "announce_recording": announce,
        "acknowledged": state.acknowledged(),
        "acknowledged_at": state.acknowledged_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unacknowledged_until_acknowledged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("voxtype").join(NOTICE_FILE);
        assert!(
            !load(&path).acknowledged(),
            "an old install starts unacknowledged"
        );
        let first = acknowledge(&path).unwrap();
        assert!(first.acknowledged());
        assert_eq!(load(&path), first);
        let again = acknowledge(&path).unwrap();
        assert_eq!(
            again.acknowledged_at, first.acknowledged_at,
            "the first time is kept"
        );
    }

    #[test]
    fn a_damaged_file_shows_the_notice_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NOTICE_FILE);
        std::fs::write(&path, "{not json").unwrap();
        assert!(!load(&path).acknowledged());
        std::fs::write(&path, r#"{"acknowledged_at": null}"#).unwrap();
        assert!(!load(&path).acknowledged());
    }

    #[test]
    fn json_carries_the_wording_and_the_state() {
        let v = to_json(&NoticeState::default(), false);
        assert_eq!(v["notice"], NOTICE);
        assert_eq!(v["announce_recording"], false);
        assert_eq!(v["acknowledged"], false);
        assert!(v["acknowledged_at"].is_null());
    }

    #[test]
    fn the_wording_is_the_agreed_text() {
        assert_eq!(
            NOTICE,
            "Laws on recording and transcribing conversations differ by place, and some require \
             everyone on the call to agree. Voxtype doesn't tell the other people for you, so let \
             them know you're using a meeting assistant."
        );
        assert_eq!(
            NOTICE_ANNOUNCED,
            "Voxtype will announce the recording to the call. You're still responsible for \
             getting consent where the law requires it."
        );
    }

    #[test]
    fn announcing_never_claims_voxtype_stays_quiet() {
        assert_eq!(text(false), NOTICE);
        assert_eq!(text(true), NOTICE_ANNOUNCED);
        assert!(!text(true).contains("doesn't tell"));
        let v = to_json(&NoticeState::default(), true);
        assert_eq!(v["notice"], NOTICE_ANNOUNCED);
        assert_eq!(v["announce_recording"], true);
    }
}
