//! The sound check: a meeting's microphone and loopback, opened exactly as a
//! meeting would open them (the same devices and the same echo path), with
//! their levels streamed and nothing recorded or transcribed.
//!
//! [`open_capture`] is also how a meeting opens its capture, so the two can
//! never disagree about which microphone or which echo canceller a meeting
//! will use.

use crate::audio::echo_cancel::{self, EchoCanceller, EchoPath, Liveness, LivenessCheck};
use crate::audio::{DualCapture, DualSamples};
use crate::config::{AudioConfig, Config};
use crate::error::AudioError;
use serde::Serialize;

/// The longest a sound check runs.
pub const MAX_SECONDS: u64 = 600;
/// How long it runs when not told.
pub const DEFAULT_SECONDS: u64 = 30;

/// Trigger file, in the runtime dir, that asks the daemon to start or stop
/// a sound check: `{"seconds": N}` or `{"stop": true}`.
pub const TRIGGER: &str = "meeting_soundcheck";

/// The meeting's loopback device, or `None` when it is off.
pub fn loopback_device(config: &Config) -> Option<String> {
    match config.meeting.audio.loopback_device.as_str() {
        "disabled" | "" => None,
        other => Some(other.to_string()),
    }
}

/// Dictation's audio config with the meeting's mic, when it has its own.
pub fn mic_config(config: &Config) -> AudioConfig {
    let mut mic = config.audio.clone();
    if !matches!(config.meeting.audio.mic_device.as_str(), "default" | "") {
        mic.device = config.meeting.audio.mic_device.clone();
    }
    mic
}

/// A meeting's audio capture, started.
pub struct MeetingCapture {
    pub capture: DualCapture,
    pub echo_path: EchoPath,
    /// The running echo canceller, when the mic is recorded through it.
    /// Dropping it stops it.
    pub canceller: Option<EchoCanceller>,
    /// What the microphone is recorded from, as the user would name it.
    pub mic: String,
    /// Watches the canceller's first seconds; see [`MeetingCapture::samples`].
    pub liveness: Option<LivenessCheck>,
}

impl MeetingCapture {
    /// The audio since the last call. While a new canceller is on trial, its
    /// mic also goes to the liveness guard: when the canceller delivers only
    /// digital silence for [`echo_cancel::LIVENESS_SECS`], it is not hearing
    /// the mic, and the mic moves to a direct capture with GTCRN
    /// ([`leave_silent_canceller`]). Returns whether that happened.
    pub async fn samples(&mut self, config: &Config) -> (DualSamples, bool) {
        let mut samples = self.capture.get_samples().await;
        let verdict = self.liveness.as_mut().map(|check| check.feed(&samples.mic));
        match verdict {
            Some(Liveness::Alive) => self.liveness = None,
            Some(Liveness::Silent) => {
                let rms = self.liveness.take().map(|c| c.rms()).unwrap_or_default();
                if let Some(rest) =
                    leave_silent_canceller(config, &mut self.capture, &mut self.canceller, rms)
                        .await
                {
                    samples.mic.extend(rest.mic);
                    samples.loopback.extend(rest.loopback);
                    self.echo_path = EchoPath::Gtcrn;
                    self.mic = mic_config(config).device;
                    return (samples, true);
                }
            }
            _ => {}
        }
        (samples, false)
    }
}

/// Open and start the meeting's capture the way a meeting does: the
/// meeting mic (or dictation's), the loopback device, and on speakers the
/// mic through PipeWire's echo canceller (fed what the loopback plays),
/// falling back to GTCRN. With the canceller the capture's liveness guard
/// is armed.
pub async fn open_capture(config: &Config) -> Result<MeetingCapture, AudioError> {
    let loopback_device = loopback_device(config);
    let mic_config = mic_config(config);
    if mic_config.device != config.audio.device {
        tracing::info!(
            "Meeting mic override: {} (dictation uses {})",
            mic_config.device,
            config.audio.device
        );
    }
    // On speakers the mic is recorded from PipeWire's echo canceller, so it
    // starts first; anything that goes wrong falls back to GTCRN.
    let mode = echo_cancel::Mode::parse(&config.meeting.audio.echo_cancel);
    let mic_device = config.meeting.audio.mic_device.clone();
    let loopback = config.meeting.audio.loopback_device.clone();
    let (mut echo_path, mut canceller) = tokio::task::spawn_blocking(move || {
        echo_cancel::begin(mode, &mic_device, &loopback, &Config::runtime_dir())
    })
    .await
    .unwrap_or((EchoPath::Gtcrn, None));

    if let Some(ref aec) = canceller {
        match DualCapture::with_mic_source(
            aec.source(),
            loopback_device.as_deref(),
            echo_cancel::MIC_DELAY_MS,
        ) {
            Ok(mut capture) => match capture.start().await {
                Ok(()) => {
                    let mic = aec.source().to_string();
                    return Ok(MeetingCapture {
                        capture,
                        echo_path,
                        canceller,
                        mic,
                        liveness: Some(LivenessCheck::new(16000)),
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
        liveness: None,
    })
}

/// The liveness guard's move, for a canceller that has delivered only
/// digital silence (at `rms`): record the mic directly instead, as a
/// meeting does when the canceller cannot start, and stop the canceller.
/// The new capture feeds the old one's level meters. Returns what the old
/// capture still held, or `None`, changing nothing, when the mic cannot be
/// opened directly.
pub async fn leave_silent_canceller(
    config: &Config,
    capture: &mut DualCapture,
    canceller: &mut Option<EchoCanceller>,
    rms: f32,
) -> Option<DualSamples> {
    tracing::warn!(
        "The echo canceller's mic has been silent for {}s ({:.1e} RMS): it is not hearing \
         the mic. Recording the mic directly with GTCRN instead",
        echo_cancel::LIVENESS_SECS,
        rms
    );
    let loopback_device = loopback_device(config);
    let mut direct = match DualCapture::new_metered(
        &mic_config(config),
        loopback_device.as_deref(),
        capture.meters(),
    ) {
        Ok(direct) => direct,
        Err(e) => {
            tracing::error!(
                "Cannot record the mic directly ({}); keeping the canceller",
                e
            );
            return None;
        }
    };
    if let Err(e) = direct.start().await {
        tracing::error!(
            "Cannot record the mic directly ({}); keeping the canceller",
            e
        );
        return None;
    }
    let mut old = std::mem::replace(capture, direct);
    let rest = old.stop().await.unwrap_or_default();
    if let Some(mut c) = canceller.take() {
        c.stop();
    }
    Some(rest)
}

/// A `soundcheck` event: a sound check started, or ended and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SoundcheckEvent {
    /// `running` or `stopped`
    pub state: String,
    /// While running: seconds left.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u64>,
    /// While running: `webrtc`, `gtcrn` or `none`, as a meeting would use;
    /// named like `echo_cancel` in `hello` and `meeting status --json`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo_cancel: Option<String>,
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
            echo_cancel: Some(
                opened
                    .echo_path
                    .effective(echo_cancel::gtcrn_available())
                    .as_str()
                    .into(),
            ),
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
            echo_cancel: None,
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
