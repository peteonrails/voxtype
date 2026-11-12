//! The sound check: a meeting's microphone and loopback, opened exactly as a
//! meeting would open them (the same devices and the same echo path), with
//! their levels streamed and nothing recorded or transcribed.
//!
//! [`open_capture`] is also how a meeting opens its capture, so the two can
//! never disagree about which microphone or which echo canceller a meeting
//! will use.

use crate::audio::echo_cancel::{self, EchoCanceller, EchoPath};
use crate::audio::DualCapture;
use crate::config::Config;
use crate::error::AudioError;
use serde::Serialize;

/// The longest a sound check runs.
pub const MAX_SECONDS: u64 = 600;
/// How long it runs when not told.
pub const DEFAULT_SECONDS: u64 = 30;

/// Trigger file, in the runtime dir, that asks the daemon to start or stop
/// a sound check: `{"seconds": N}` or `{"stop": true}`.
pub const TRIGGER: &str = "meeting_soundcheck";

/// A meeting's audio capture, started.
pub struct MeetingCapture {
    pub capture: DualCapture,
    pub echo_path: EchoPath,
    /// The running echo canceller, when the mic is recorded through it.
    /// Dropping it stops it.
    pub canceller: Option<EchoCanceller>,
    /// What the microphone is recorded from, as the user would name it.
    pub mic: String,
}

/// Open and start the meeting's capture the way a meeting does: the
/// meeting mic (or dictation's), the loopback device, and on speakers the
/// mic through PipeWire's echo canceller, falling back to GTCRN.
pub async fn open_capture(config: &Config) -> Result<MeetingCapture, AudioError> {
    let loopback_device = match config.meeting.audio.loopback_device.as_str() {
        "disabled" | "" => None,
        other => Some(other.to_string()),
    };
    let mut mic_config = config.audio.clone();
    let meeting_mic = config.meeting.audio.mic_device.as_str();
    if !matches!(meeting_mic, "default" | "") {
        tracing::info!(
            "Meeting mic override: {} (dictation uses {})",
            meeting_mic,
            config.audio.device
        );
        mic_config.device = meeting_mic.to_string();
    }
    // On speakers the mic is recorded from PipeWire's echo canceller, so it
    // starts first; anything that goes wrong falls back to GTCRN.
    let mode = echo_cancel::Mode::parse(&config.meeting.audio.echo_cancel);
    let mic_device = config.meeting.audio.mic_device.clone();
    let (mut echo_path, mut canceller) = tokio::task::spawn_blocking(move || {
        echo_cancel::begin(mode, &mic_device, &Config::runtime_dir())
    })
    .await
    .unwrap_or((EchoPath::Gtcrn, None));

    if let Some(ref aec) = canceller {
        match DualCapture::with_mic_source(aec.source(), loopback_device.as_deref()) {
            Ok(mut capture) => match capture.start().await {
                Ok(()) => {
                    let mic = aec.source().to_string();
                    return Ok(MeetingCapture {
                        capture,
                        echo_path,
                        canceller,
                        mic,
                    });
                }
                Err(e) => {
                    tracing::warn!("Cannot record the echo-cancelled mic ({}); using GTCRN", e)
                }
            },
            Err(e) => tracing::warn!("Cannot record the echo-cancelled mic ({}); using GTCRN", e),
        }
        canceller = None;
        echo_path = EchoPath::Gtcrn;
    }

    let mut capture = DualCapture::new(&mic_config, loopback_device.as_deref()).map_err(|e| {
        tracing::error!("Failed to create meeting audio capture: {}", e);
        e
    })?;
    capture.start().await.map_err(|e| {
        tracing::error!("Failed to start meeting audio: {}", e);
        e
    })?;
    Ok(MeetingCapture {
        capture,
        echo_path,
        canceller,
        mic: mic_config.device,
    })
}

/// A `soundcheck` event: a sound check started, or ended and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SoundcheckEvent {
    /// `running` or `stopped`
    pub state: String,
    /// While running: seconds left.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u64>,
    /// While running: `aec`, `gtcrn` or `off`, as a meeting would use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo_path: Option<String>,
    /// While running: what the microphone is recorded from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic: Option<String>,
    /// While running: whether the loopback (the other side) is captured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loopback: Option<bool>,
    /// When stopped: `done` (its time ran out), `stopped` (asked to),
    /// `meeting` (a meeting needs the microphone) or `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When stopped with an error: what went wrong.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl SoundcheckEvent {
    pub fn running(opened: &MeetingCapture, seconds: u64) -> Self {
        Self {
            state: "running".into(),
            seconds: Some(seconds),
            echo_path: Some(opened.echo_path.as_str().into()),
            mic: Some(opened.mic.clone()),
            loopback: Some(opened.capture.has_loopback()),
            reason: None,
            message: None,
        }
    }

    pub fn stopped(reason: &str, message: Option<String>) -> Self {
        Self {
            state: "stopped".into(),
            seconds: None,
            echo_path: None,
            mic: None,
            loopback: None,
            reason: Some(reason.into()),
            message,
        }
    }
}

/// What the trigger file asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Start { seconds: u64 },
    Stop,
}

impl Request {
    /// Parse the trigger file's contents. Anything unreadable is a start
    /// with the default length, the safe reading of "something asked".
    pub fn parse(text: &str) -> Self {
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        if v["stop"].as_bool() == Some(true) {
            return Request::Stop;
        }
        let seconds = v["seconds"]
            .as_u64()
            .unwrap_or(DEFAULT_SECONDS)
            .clamp(1, MAX_SECONDS);
        Request::Start { seconds }
    }

    pub fn to_json(self) -> String {
        match self {
            Request::Start { seconds } => serde_json::json!({ "seconds": seconds }).to_string(),
            Request::Stop => serde_json::json!({ "stop": true }).to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_and_are_clamped() {
        for r in [Request::Start { seconds: 45 }, Request::Stop] {
            assert_eq!(Request::parse(&r.to_json()), r);
        }
        assert_eq!(
            Request::parse(r#"{"seconds": 99999}"#),
            Request::Start {
                seconds: MAX_SECONDS
            }
        );
        assert_eq!(
            Request::parse(""),
            Request::Start {
                seconds: DEFAULT_SECONDS
            }
        );
    }

    #[test]
    fn events_serialize_only_what_applies() {
        let stopped = serde_json::to_value(SoundcheckEvent::stopped("done", None)).unwrap();
        assert_eq!(
            stopped,
            serde_json::json!({"state": "stopped", "reason": "done"})
        );
    }
}
