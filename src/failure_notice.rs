//! Desktop notices for failures the user has to fix before dictation works.
//!
//! When the microphone can't be opened or the model can't be loaded, the
//! daemon used to log the error and play the error beep. The beep needs the
//! same audio stack that may be broken, and nobody reads the journal after
//! pressing a hotkey, so what the user saw was a dead hotkey. These builders
//! turn those errors into a notification that names what failed and the
//! command that fixes it. They are pure so the wording is unit-tested; the
//! daemon owns sending them.

use std::time::{Duration, Instant};

use crate::config::{Config, TranscriptionEngine, WhisperMode};
use crate::error::{AudioError, TranscribeError};

/// A notification to post: what failed, and how to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// Longest error detail quoted in a notice. Notification servers clip long
/// bodies, and the fix comes after the detail, so the detail must not push
/// it off screen.
const MAX_DETAIL_CHARS: usize = 160;

/// First line of an error message, clipped. Several errors carry a remedy
/// on later lines that the notice replaces with its own.
fn detail(message: &str) -> String {
    let first = message.lines().next().unwrap_or("").trim();
    if first.chars().count() <= MAX_DETAIL_CHARS {
        first.to_string()
    } else {
        let clipped: String = first.chars().take(MAX_DETAIL_CHARS).collect();
        format!("{}...", clipped.trim_end())
    }
}

/// Notice for a microphone that could not be opened or started.
///
/// `device` is the configured `[audio] device`.
pub fn capture_failed(device: &str, err: &AudioError) -> Notice {
    const PICK: &str = "List microphones with `voxtype info devices`, then run \
                        `voxtype config set audio.device <name>` and restart voxtype.";
    match err {
        AudioError::DeviceNotFound(name) if name == "default" => Notice {
            title: "Voxtype: no microphone found".to_string(),
            body: format!(
                "The system has no default input source, so nothing can be recorded. \
                 Plug in or unmute a microphone and check `wpctl status` or \
                 `pactl list sources short`. {PICK}"
            ),
        },
        AudioError::DeviceNotFound(name)
        | AudioError::DeviceNotFoundWithList {
            requested: name, ..
        } => Notice {
            title: "Voxtype: microphone not found".to_string(),
            body: format!(
                "The microphone set in [audio] device = \"{name}\" is not connected. {PICK} \
                     To follow the system default instead: \
                     `voxtype config set audio.device default`."
            ),
        },
        other => Notice {
            title: "Voxtype: could not open the microphone".to_string(),
            body: format!(
                "Recording from \"{device}\" failed: {}. Check that PipeWire or PulseAudio is \
                 running (`systemctl --user status pipewire wireplumber`). {PICK}",
                detail(&other.to_string()),
            ),
        },
    }
}

/// Whether the engine runs without a local model, so downloading one can't
/// be the fix.
fn is_remote(config: &Config) -> bool {
    match config.engine {
        TranscriptionEngine::Soniox => true,
        TranscriptionEngine::Whisper => config.whisper.effective_mode() == WhisperMode::Remote,
        _ => false,
    }
}

/// Notice for a transcription model that could not be loaded.
///
/// `model` is the model that was being loaded (the configured one, or the
/// secondary model chosen for this recording). `at_startup` says the daemon
/// is exiting because of it, rather than skipping one recording.
pub fn model_load_failed(
    config: &Config,
    model: &str,
    err: &TranscribeError,
    at_startup: bool,
) -> Notice {
    // `engine` names the config section, `label` reads as a name in titles.
    let engine = config.engine.name();
    let label = format!("{:?}", config.engine);
    let consequence = if at_startup {
        "Voxtype could not start."
    } else {
        "Nothing was transcribed."
    };

    // The running binary lacks the engine. No model download fixes that;
    // the fix is the one `warn_on_variant_mismatch` gives at startup.
    if err.to_string().contains("not compiled with --features") {
        return Notice {
            title: format!("Voxtype: {label} engine unavailable"),
            body: format!(
                "{}. {consequence} Switch to a build that includes it with \
                 `sudo voxtype setup onnx --enable` (or press F2 in `voxtype configure`), \
                 or choose another engine with `voxtype setup model`.",
                detail(&err.to_string()),
            ),
        };
    }

    if is_remote(config) {
        return Notice {
            title: format!("Voxtype: {label} engine failed to start"),
            body: format!(
                "{}. {consequence} Check the [{engine}] settings in config.toml \
                 (endpoint, API key) and run `voxtype setup check`.",
                detail(&err.to_string()),
            ),
        };
    }

    let pick = "or pick an installed model with `voxtype setup model`";
    match err {
        // The loader's message lists the paths it searched; the log keeps it.
        TranscribeError::ModelNotFound(_) => {
            let fix = if std::path::Path::new(model).is_absolute() {
                format!("Fix the [{engine}] model path in config.toml, {pick}.")
            } else {
                format!("Download it with `voxtype setup --download --model {model}`, {pick}.")
            };
            Notice {
                title: format!("Voxtype: {label} model missing"),
                body: format!("Model \"{model}\" is not installed. {consequence} {fix}"),
            }
        }
        other => Notice {
            title: format!("Voxtype: {label} model failed to load"),
            body: format!(
                "Could not load \"{model}\": {}. {consequence} Check the download with \
                 `voxtype info models --verify`, re-download it with \
                 `voxtype setup --download --model {model}`, {pick}.",
                detail(&other.to_string()),
            ),
        },
    }
}

/// How long a failure that keeps recurring stays quiet after its notice.
/// Long enough that holding a dead hotkey repeatedly doesn't stack
/// notifications, short enough that someone who dismissed the first one and
/// came back later is told again.
pub const REPEAT_AFTER: Duration = Duration::from_secs(300);

/// Decides whether a failure deserves a notification: the first failure
/// always does, a repeat of the same failure does not until `REPEAT_AFTER`
/// has passed, and a different failure does at once.
#[derive(Debug, Default)]
pub struct NoticeGate {
    last: Option<(String, Instant)>,
}

impl NoticeGate {
    /// Record a failure identified by `key` and say whether to notify.
    pub fn should_notify(&mut self, key: &str, now: Instant) -> bool {
        if let Some((last_key, at)) = &self.last {
            if last_key == key && now.duration_since(*at) < REPEAT_AFTER {
                return false;
            }
        }
        self.last = Some((key.to_string(), now));
        true
    }

    /// Forget the last failure, after the operation succeeded, so the next
    /// failure notifies even if it is the same one.
    pub fn clear(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_default_source_names_the_source_tools() {
        let n = capture_failed("default", &AudioError::DeviceNotFound("default".into()));
        assert_eq!(n.title, "Voxtype: no microphone found");
        assert!(n.body.contains("wpctl status"));
        assert!(n.body.contains("voxtype info devices"));
    }

    #[test]
    fn missing_configured_device_names_it_and_offers_default() {
        let err = AudioError::DeviceNotFoundWithList {
            requested: "Blue Yeti".into(),
            available: "Available devices:\n  - default".into(),
        };
        let n = capture_failed("Blue Yeti", &err);
        assert_eq!(n.title, "Voxtype: microphone not found");
        assert!(n.body.contains("device = \"Blue Yeti\""));
        assert!(n.body.contains("voxtype config set audio.device default"));
        assert!(!n.body.contains("Available devices"));
    }

    #[test]
    fn connection_failure_quotes_error_and_points_at_pipewire() {
        let err = AudioError::Connection("Host is down\nmore".into());
        let n = capture_failed("default", &err);
        assert_eq!(n.title, "Voxtype: could not open the microphone");
        assert!(n.body.contains("Audio connection failed: Host is down."));
        assert!(!n.body.contains("more"));
        assert!(n.body.contains("pipewire"));
    }

    #[test]
    fn missing_named_model_offers_download_command() {
        let config = Config {
            engine: TranscriptionEngine::Parakeet,
            ..Default::default()
        };
        let err = TranscribeError::ModelNotFound(
            "Parakeet model 'parakeet-tdt-0.6b-v3' not found\n  Run 'voxtype setup'".into(),
        );
        let n = model_load_failed(&config, "parakeet-tdt-0.6b-v3", &err, false);
        assert_eq!(n.title, "Voxtype: Parakeet model missing");
        assert!(n
            .body
            .contains("voxtype setup --download --model parakeet-tdt-0.6b-v3"));
        assert!(n.body.contains("Nothing was transcribed."));
        assert!(!n.body.contains("Run 'voxtype setup'"));
    }

    #[test]
    fn missing_model_path_says_fix_the_path() {
        let config = Config::default();
        let err = TranscribeError::ModelNotFound("/models/x.bin".into());
        let n = model_load_failed(&config, "/models/x.bin", &err, true);
        assert!(n.body.contains("Fix the [whisper] model path"));
        assert!(!n.body.contains("--download"));
        assert!(n.body.contains("Voxtype could not start."));
    }

    #[test]
    fn load_failure_suggests_verify_and_redownload() {
        let config = Config::default();
        let err = TranscribeError::InitFailed("bad magic".into());
        let n = model_load_failed(&config, "base.en", &err, false);
        assert_eq!(n.title, "Voxtype: Whisper model failed to load");
        assert!(n.body.contains("bad magic"));
        assert!(n.body.contains("voxtype info models --verify"));
    }

    #[test]
    fn remote_engine_does_not_suggest_download() {
        let config = Config {
            engine: TranscriptionEngine::Soniox,
            ..Default::default()
        };
        let err = TranscribeError::ConfigError("missing API key".into());
        let n = model_load_failed(&config, "stt-rt", &err, false);
        assert!(n.body.contains("missing API key"));
        assert!(n.body.contains("[soniox]"));
        assert!(!n.body.contains("--download"));
    }

    #[test]
    fn engine_missing_from_build_points_at_variant_switch() {
        let config = Config {
            engine: TranscriptionEngine::Parakeet,
            ..Default::default()
        };
        let err = TranscribeError::InitFailed(
            "Parakeet engine requested but voxtype was not compiled with --features parakeet"
                .into(),
        );
        let n = model_load_failed(&config, "parakeet-tdt-0.6b-v3", &err, true);
        assert_eq!(n.title, "Voxtype: Parakeet engine unavailable");
        assert!(n.body.contains("voxtype setup onnx --enable"));
        assert!(!n.body.contains("--download"));
    }

    #[test]
    fn long_detail_is_clipped() {
        let long = "x".repeat(500);
        let n = capture_failed("default", &AudioError::StreamError(long));
        assert!(n.body.contains("..."));
        assert!(n.body.len() < 600);
    }

    #[test]
    fn gate_notifies_first_failure_then_suppresses_repeats() {
        let mut gate = NoticeGate::default();
        let t0 = Instant::now();
        assert!(gate.should_notify("a", t0));
        assert!(!gate.should_notify("a", t0 + Duration::from_secs(1)));
        assert!(gate.should_notify("b", t0 + Duration::from_secs(2)));
        assert!(gate.should_notify("a", t0 + Duration::from_secs(3)));
    }

    #[test]
    fn gate_renotifies_after_cooldown_or_success() {
        let mut gate = NoticeGate::default();
        let t0 = Instant::now();
        assert!(gate.should_notify("a", t0));
        assert!(gate.should_notify("a", t0 + REPEAT_AFTER));
        gate.clear();
        assert!(gate.should_notify("a", t0 + REPEAT_AFTER + Duration::from_secs(1)));
    }
}
