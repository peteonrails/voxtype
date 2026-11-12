//! Meeting-mode configuration.

use serde::{Deserialize, Serialize};

use super::default_true;

/// Per-meeting choices given to `voxtype meeting start`. Unset fields keep
/// the configuration's values. Travels from the CLI to the daemon as JSON.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct MeetingStartOptions {
    /// Engine for this meeting's transcription (`whisper`, `parakeet`, ...)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// Model for this meeting; its engine is implied when `engine` is unset
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Language hint for engines that take one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The call's link, stored with the meeting
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The title is the call window's, which the calendar's replaces
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub title_from_window: bool,
    /// Play the recording announcement (`--announce`/`--no-announce`);
    /// unset keeps `[meeting] announce_recording`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub announce: Option<bool>,
    /// The announcement's audio file for this meeting; unset keeps
    /// `[meeting] announcement_file`, or the built-in clip
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub announcement_file: Option<String>,
}

/// Meeting transcription configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingConfig {
    /// Enable meeting mode
    #[serde(default)]
    pub enabled: bool,

    /// While meeting mode is off, offer to turn it on from the tray
    #[serde(default = "default_true")]
    pub offer_enable: bool,

    /// Meeting started/paused/resumed/ended notifications: "auto" skips them
    /// while a meeting view is on screen, "always", or "never". Errors always
    /// notify.
    #[serde(default = "default_status_notifications")]
    pub status_notifications: String,

    /// Duration of each audio chunk in seconds
    #[serde(default = "default_chunk_duration")]
    pub chunk_duration_secs: u32,

    /// Storage path for meetings ("auto" for default location)
    /// Default: ~/.local/share/voxtype/meetings/
    #[serde(default = "default_storage_path")]
    pub storage_path: String,

    /// Keep the meeting's audio in its folder: mic.opus, loopback.opus and
    /// mixed.opus (WAV when ffmpeg is missing)
    #[serde(default)]
    pub retain_audio: bool,

    /// Shell command run after a meeting stops and its folder is final
    /// (after the auto-summary, when one runs). Gets VOXTYPE_MEETING_ID,
    /// VOXTYPE_MEETING_DIR and VOXTYPE_MEETING_MANIFEST.
    #[serde(default)]
    pub post_meeting_command: Option<String>,

    /// Seconds before post_meeting_command is killed
    #[serde(default = "default_post_meeting_timeout")]
    pub post_meeting_timeout_secs: u64,

    /// Maximum meeting duration in minutes (0 = unlimited)
    #[serde(default = "default_max_duration")]
    pub max_duration_mins: u32,

    /// Open the live meeting view when a meeting starts. Read by the Omarchy
    /// plugin, which is the only frontend with such a view; elsewhere it does
    /// nothing.
    #[serde(default = "default_true")]
    pub show_view_on_start: bool,

    /// How opaque the meeting view's background is while the pointer is
    /// elsewhere (0.3 to 1.0), so it reads as an overlay. Text stays opaque.
    #[serde(default = "default_view_opacity")]
    pub view_opacity: f32,

    /// Name the speakers of a two-person meeting from its title: in
    /// "Pete <> Andrew standup" the side that is you names your microphone
    /// and the other names every remote voice. A name given by hand is
    /// never replaced.
    #[serde(default = "default_true")]
    pub auto_label_from_title: bool,

    /// Announce that the meeting is being recorded, once, when it starts:
    /// the clip is mixed into the call app's microphone stream so the other
    /// people on the call hear it, and played locally. Off by default. A
    /// courtesy, not consent: the laws on recording calls still apply.
    #[serde(default)]
    pub announce_recording: bool,

    /// Audio file played instead of the built-in announcement (WAV, FLAC or
    /// Ogg), for your own wording or language. Unset uses the built-in clip.
    #[serde(default)]
    pub announcement_file: Option<String>,

    /// Meeting audio configuration
    #[serde(default)]
    pub audio: MeetingAudioConfig,

    /// Diarization configuration
    #[serde(default)]
    pub diarization: MeetingDiarizationConfig,

    /// Summarization configuration
    #[serde(default)]
    pub summary: MeetingSummaryConfig,

    /// Call detection and the Record prompt
    #[serde(default)]
    pub detect: super::MeetingDetectConfig,

    /// Titles from the calendar for meetings that start without one
    #[serde(default)]
    pub calendar: MeetingCalendarConfig,
    /// Meeting assistant: `meeting ask` and agent summaries
    #[serde(default)]
    pub assistant: MeetingAssistantConfig,
}

/// Meeting audio configuration for dual capture
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingAudioConfig {
    /// Microphone device (uses main audio.device if not specified)
    #[serde(default = "default_mic_device")]
    pub mic_device: String,

    /// Loopback device for capturing remote participants
    /// Options: "auto" (detect), "disabled", or specific device name
    #[serde(default = "default_loopback")]
    pub loopback_device: String,

    /// Echo cancellation for the meeting mic:
    /// - "auto" (default): PipeWire's WebRTC echo canceller unless the output
    ///   is headphones, a headset or Bluetooth; GTCRN enhancement otherwise,
    ///   or when the canceller cannot run
    /// - "webrtc": the WebRTC canceller whatever the output, GTCRN if it
    ///   cannot run
    /// - "gtcrn": GTCRN enhancement only (the behaviour before 1.3.0)
    /// - "disabled": neither
    ///
    /// "aec" and "off", the names in early 1.3.0 test builds, still work as
    /// "webrtc" and "disabled".
    ///
    /// The canceller runs only during a meeting and never changes PipeWire's
    /// configuration. Transcript dedup runs in every case.
    #[serde(default = "default_echo_cancel")]
    pub echo_cancel: String,

    /// RMS threshold for meeting chunk voice activity detection.
    /// Lower values are more permissive; 0.0 disables the pre-transcription gate.
    #[serde(default = "default_meeting_vad_threshold")]
    pub vad_threshold: f32,

    /// The echo guard: keep the remote side, heard through your speakers,
    /// out of your lines. Microphone lines whose sound is all explained by
    /// the loopback go (or move to the remote speaker), and short lines that
    /// are the remote misheard go, unless the microphone's level shows you
    /// were the one speaking. `echo_fuzzy`, its name during development, is
    /// read too.
    #[serde(default = "default_true", alias = "echo_fuzzy")]
    pub echo_guard: bool,

    /// The longest microphone line, in words, the guard's misheard-words
    /// check looks at.
    #[serde(default = "default_echo_fuzzy_max_words")]
    pub echo_fuzzy_max_words: u32,
}

fn default_echo_fuzzy_max_words() -> u32 {
    4
}

fn default_view_opacity() -> f32 {
    0.8
}

fn default_mic_device() -> String {
    "default".to_string()
}

fn default_loopback() -> String {
    "auto".to_string()
}

fn default_echo_cancel() -> String {
    "auto".to_string()
}

fn default_meeting_vad_threshold() -> f32 {
    0.01
}

impl Default for MeetingAudioConfig {
    fn default() -> Self {
        Self {
            mic_device: default_mic_device(),
            loopback_device: default_loopback(),
            echo_cancel: default_echo_cancel(),
            vad_threshold: default_meeting_vad_threshold(),
            echo_guard: true,
            echo_fuzzy_max_words: default_echo_fuzzy_max_words(),
        }
    }
}

/// Meeting diarization configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingDiarizationConfig {
    /// Enable speaker diarization
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Diarization backend: "simple", "ml" or "hybrid".
    /// The match arms in `src/meeting/diarization/mod.rs` are the source of
    /// truth for what the daemon actually dispatches on; the CLI's
    /// `--diarization` value parser is pinned to the same set via
    /// `crate::cli::DIARIZATION_BACKENDS` and a test in this file.
    #[serde(default = "default_diarization_backend")]
    pub backend: String,

    /// Maximum number of speakers to detect
    #[serde(default = "default_max_speakers")]
    pub max_speakers: u32,

    /// Path to ONNX model for ML backend (uses default if not set)
    #[serde(default)]
    pub model_path: Option<String>,

    /// Minimum segment duration in milliseconds for ML embedding extraction
    #[serde(default = "default_min_segment_ms")]
    pub min_segment_ms: u64,

    // The four fields below apply to the clustering backends, "ml" and
    // "hybrid"; "simple" ignores them.
    /// Cosine similarity threshold for the ML backend (0.20-0.30 typical for ECAPA on 4s windows)
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f32,

    /// VAD sub-window length in seconds for ECAPA feeding
    #[serde(default = "default_vad_window_secs")]
    pub vad_window_secs: f32,

    /// VAD sub-window hop in seconds
    #[serde(default = "default_vad_hop_secs")]
    pub vad_hop_secs: f32,

    /// RMS floor for treating a sub-window as silence
    #[serde(default = "default_vad_rms_floor")]
    pub vad_rms_floor: f32,
}

fn default_diarization_backend() -> String {
    "simple".to_string()
}

fn default_max_speakers() -> u32 {
    10
}

fn default_min_segment_ms() -> u64 {
    500
}

// Empirically tuned against multi-speaker test clips (4-person roundtable,
// 3-person panel, 1h talk). The previous 0.75 anchor was far too strict for
// 4s ECAPA windows and produced overwhelmingly Unknown labels in practice.
fn default_similarity_threshold() -> f32 {
    0.25
}

fn default_vad_window_secs() -> f32 {
    4.0
}

fn default_vad_hop_secs() -> f32 {
    2.0
}

fn default_vad_rms_floor() -> f32 {
    0.005
}

fn default_status_notifications() -> String {
    "auto".to_string()
}

fn default_chunk_duration() -> u32 {
    30
}

fn default_storage_path() -> String {
    "auto".to_string()
}

fn default_max_duration() -> u32 {
    180
}

fn default_post_meeting_timeout() -> u64 {
    300
}

impl Default for MeetingDiarizationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: default_diarization_backend(),
            max_speakers: default_max_speakers(),
            model_path: None,
            min_segment_ms: default_min_segment_ms(),
            similarity_threshold: default_similarity_threshold(),
            vad_window_secs: default_vad_window_secs(),
            vad_hop_secs: default_vad_hop_secs(),
            vad_rms_floor: default_vad_rms_floor(),
        }
    }
}

/// Meeting summary configuration (Phase 5)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingSummaryConfig {
    /// Summarization backend: "local", "remote", or "disabled"
    #[serde(default = "default_summary_backend")]
    pub backend: String,

    /// Ollama URL for local backend
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,

    /// Ollama model name
    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,

    /// Remote API endpoint for remote backend
    #[serde(default)]
    pub remote_endpoint: Option<String>,

    /// Remote API key
    #[serde(default)]
    pub remote_api_key: Option<String>,

    /// Request timeout in seconds
    #[serde(default = "default_summary_timeout")]
    pub timeout_secs: u64,

    /// Summarize each meeting when it stops. Unset means on once an agent
    /// has been allowed (`voxtype meeting assistant allow <agent>`), off
    /// otherwise.
    #[serde(default)]
    pub auto: Option<bool>,
}

/// Meeting assistant configuration: which coding agent answers `meeting ask`
/// and writes summaries, run headless with no tools.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingAssistantConfig {
    /// Agent to use: "claude", "codex" or "pi". Unset uses `omarchy-default-agent`
    /// when that command exists, then a supported agent CLI found on PATH.
    #[serde(default)]
    pub agent: Option<String>,

    /// Never send transcripts to an agent; answer and summarize with the
    /// local model from `[meeting.summary]` (Ollama) instead.
    #[serde(default)]
    pub local_only: bool,

    /// Seconds to wait for an answer or a summary.
    #[serde(default = "default_assistant_timeout")]
    pub timeout_secs: u64,

    /// Transcript characters sent verbatim. A longer meeting sends a summary
    /// of its earlier part plus as many recent lines as fit.
    #[serde(default = "default_assistant_max_transcript_chars")]
    pub max_transcript_chars: u64,

    /// Command the "Open" action of an `ask --notify` notification runs,
    /// through `sh -c`. VOXTYPE_MEETING_ID and VOXTYPE_ASK_ID are set.
    /// Unset opens the meeting view on its Ask tab
    /// (`voxtype meeting view "$VOXTYPE_MEETING_ID" --tab ask`); an empty
    /// string shows no Open action.
    #[serde(default)]
    pub open_command: Option<String>,
}

fn default_assistant_timeout() -> u64 {
    180
}

fn default_assistant_max_transcript_chars() -> u64 {
    100_000
}

impl Default for MeetingAssistantConfig {
    fn default() -> Self {
        Self {
            agent: None,
            local_only: false,
            timeout_secs: default_assistant_timeout(),
            max_transcript_chars: default_assistant_max_transcript_chars(),
            open_command: None,
        }
    }
}

fn default_summary_backend() -> String {
    "disabled".to_string()
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_ollama_model() -> String {
    "llama3.2".to_string()
}

fn default_summary_timeout() -> u64 {
    120
}

impl Default for MeetingSummaryConfig {
    fn default() -> Self {
        Self {
            backend: default_summary_backend(),
            ollama_url: default_ollama_url(),
            ollama_model: default_ollama_model(),
            remote_endpoint: None,
            remote_api_key: None,
            timeout_secs: default_summary_timeout(),
            auto: None,
        }
    }
}

/// `[meeting.calendar]`: a title from the calendar for a meeting that
/// starts without one
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct MeetingCalendarConfig {
    /// Command printing the next meeting as the Omarchy Next Meeting
    /// plugin's JSON. Empty uses that plugin's `agendaCommand` when the
    /// plugin is on the Omarchy bar, and is otherwise off.
    #[serde(default)]
    pub command: String,

    /// An event counts when it is running, or starts within this many
    /// minutes either side of the meeting's start
    #[serde(default = "default_calendar_window")]
    pub window_mins: u32,

    /// Seconds to wait for the command
    #[serde(default = "default_calendar_timeout")]
    pub timeout_secs: u64,
}

fn default_calendar_window() -> u32 {
    10
}

fn default_calendar_timeout() -> u64 {
    3
}

impl Default for MeetingCalendarConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            window_mins: default_calendar_window(),
            timeout_secs: default_calendar_timeout(),
        }
    }
}

impl Default for MeetingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            offer_enable: true,
            status_notifications: default_status_notifications(),
            chunk_duration_secs: default_chunk_duration(),
            storage_path: default_storage_path(),
            retain_audio: false,
            post_meeting_command: None,
            post_meeting_timeout_secs: default_post_meeting_timeout(),
            max_duration_mins: default_max_duration(),
            show_view_on_start: true,
            view_opacity: default_view_opacity(),
            auto_label_from_title: true,
            announce_recording: false,
            announcement_file: None,
            audio: MeetingAudioConfig::default(),
            diarization: MeetingDiarizationConfig::default(),
            summary: MeetingSummaryConfig::default(),
            detect: super::MeetingDetectConfig::default(),
            calendar: MeetingCalendarConfig::default(),
            assistant: MeetingAssistantConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_meeting_config_default() {
        let config = MeetingConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.chunk_duration_secs, 30);
        assert_eq!(config.storage_path, "auto");
        assert!(!config.retain_audio);
        assert_eq!(config.max_duration_mins, 180);
        assert!(config.post_meeting_command.is_none());
        assert_eq!(config.post_meeting_timeout_secs, 300);
        assert!(config.show_view_on_start);
        assert!((config.view_opacity - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_show_view_on_start_defaults_on_and_can_be_turned_off() {
        // A config written before the key existed keeps the view opening.
        let config: Config = toml::from_str("[meeting]\nenabled = true\n").unwrap();
        assert!(config.meeting.show_view_on_start);

        let config: Config =
            toml::from_str("[meeting]\nenabled = true\nshow_view_on_start = false\n").unwrap();
        assert!(!config.meeting.show_view_on_start);
    }

    #[test]
    fn test_announce_recording_is_off_for_old_configs_and_can_be_set() {
        assert!(!MeetingConfig::default().announce_recording);
        assert!(MeetingConfig::default().announcement_file.is_none());

        // A config written before the keys existed never announces.
        let config: Config = toml::from_str("[meeting]\nenabled = true\n").unwrap();
        assert!(!config.meeting.announce_recording);
        assert!(config.meeting.announcement_file.is_none());

        let config: Config = toml::from_str(
            "[meeting]\nannounce_recording = true\nannouncement_file = \"~/announce-de.wav\"\n",
        )
        .unwrap();
        assert!(config.meeting.announce_recording);
        assert_eq!(
            config.meeting.announcement_file.as_deref(),
            Some("~/announce-de.wav")
        );
    }

    #[test]
    fn test_start_options_without_announce_fields_still_parse() {
        // A trigger written by an older CLI has neither field.
        let options: MeetingStartOptions = serde_json::from_str(r#"{"engine":"whisper"}"#).unwrap();
        assert_eq!(options.announce, None);
        assert_eq!(options.announcement_file, None);
        // Unset fields stay off the wire.
        let json = serde_json::to_string(&MeetingStartOptions::default()).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn test_meeting_audio_config_default() {
        let config = MeetingAudioConfig::default();
        assert_eq!(config.mic_device, "default");
        assert_eq!(config.loopback_device, "auto");
        assert_eq!(config.vad_threshold, 0.01);
        assert!(config.echo_guard);
        assert_eq!(config.echo_fuzzy_max_words, 4);
    }

    #[test]
    fn test_echo_guard_defaults_on_for_old_configs_and_can_be_set() {
        let config: Config = toml::from_str("[meeting.audio]\necho_cancel = \"auto\"\n").unwrap();
        let rules = crate::meeting::echo::EchoRules::from(&config.meeting.audio);
        assert_eq!(rules, crate::meeting::echo::EchoRules::default());

        let config: Config =
            toml::from_str("[meeting.audio]\necho_guard = false\necho_fuzzy_max_words = 40\n")
                .unwrap();
        let rules = crate::meeting::echo::EchoRules::from(&config.meeting.audio);
        assert!(!rules.guard);
        // The development name still reads.
        let config: Config = toml::from_str("[meeting.audio]\necho_fuzzy = false\n").unwrap();
        assert!(!config.meeting.audio.echo_guard);
        assert_eq!(rules.fuzzy_max_words, 12, "clamped to the schema's range");
    }

    #[test]
    fn test_meeting_diarization_config_default() {
        let config = MeetingDiarizationConfig::default();
        assert!(config.enabled);
        assert_eq!(config.backend, "simple");
        assert_eq!(config.max_speakers, 10);
    }

    #[test]
    fn test_meeting_summary_config_default() {
        let config = MeetingSummaryConfig::default();
        assert_eq!(config.backend, "disabled");
        assert_eq!(config.ollama_url, "http://localhost:11434");
        assert_eq!(config.ollama_model, "llama3.2");
        assert!(config.remote_endpoint.is_none());
        assert!(config.remote_api_key.is_none());
        assert_eq!(config.timeout_secs, 120);
    }

    #[test]
    fn test_meeting_config_in_default_config() {
        let config = Config::default();
        assert!(!config.meeting.enabled);
        assert_eq!(config.meeting.chunk_duration_secs, 30);
        assert_eq!(config.meeting.max_duration_mins, 180);
    }

    #[test]
    fn test_parse_meeting_config_from_toml() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [meeting]
            enabled = true
            chunk_duration_secs = 45
            storage_path = "/tmp/meetings"
            retain_audio = true
            max_duration_mins = 60
            post_meeting_command = "notify-send done"
            post_meeting_timeout_secs = 30
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.meeting.enabled);
        assert_eq!(config.meeting.chunk_duration_secs, 45);
        assert_eq!(config.meeting.storage_path, "/tmp/meetings");
        assert!(config.meeting.retain_audio);
        assert_eq!(config.meeting.max_duration_mins, 60);
        assert_eq!(
            config.meeting.post_meeting_command.as_deref(),
            Some("notify-send done")
        );
        assert_eq!(config.meeting.post_meeting_timeout_secs, 30);
    }

    #[test]
    fn test_parse_meeting_config_with_nested_sections() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [meeting]
            enabled = true

            [meeting.audio]
            mic_device = "hw:1"
            loopback_device = "disabled"
            vad_threshold = 0.001

            [meeting.diarization]
            enabled = false
            backend = "ml"
            max_speakers = 5

            [meeting.summary]
            backend = "local"
            ollama_model = "mistral"
            timeout_secs = 60
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.meeting.audio.mic_device, "hw:1");
        assert_eq!(config.meeting.audio.loopback_device, "disabled");
        assert_eq!(config.meeting.audio.vad_threshold, 0.001);
        assert!(!config.meeting.diarization.enabled);
        assert_eq!(config.meeting.diarization.backend, "ml");
        assert_eq!(config.meeting.diarization.max_speakers, 5);
        assert_eq!(config.meeting.summary.backend, "local");
        assert_eq!(config.meeting.summary.ollama_model, "mistral");
        assert_eq!(config.meeting.summary.timeout_secs, 60);
    }

    #[test]
    fn test_meeting_config_backward_compatible_omitted() {
        // Config without [meeting] section should parse fine with defaults
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.meeting.enabled);
        assert_eq!(config.meeting.chunk_duration_secs, 30);
        assert_eq!(config.meeting.storage_path, "auto");
        assert_eq!(config.meeting.diarization.backend, "simple");
        assert_eq!(config.meeting.summary.backend, "disabled");
        assert_eq!(config.meeting.summary.auto, None);
        assert_eq!(config.meeting.assistant.agent, None);
        assert!(!config.meeting.assistant.local_only);
        assert_eq!(config.meeting.assistant.timeout_secs, 180);
        assert_eq!(config.meeting.assistant.max_transcript_chars, 100_000);
    }

    #[test]
    fn test_parse_meeting_assistant_config() {
        let config: Config = toml::from_str(
            r#"
            [meeting.summary]
            auto = false

            [meeting.assistant]
            agent = "codex"
            local_only = true
            timeout_secs = 30
            max_transcript_chars = 5000
            open_command = "true"
        "#,
        )
        .unwrap();
        assert_eq!(config.meeting.summary.auto, Some(false));
        let a = &config.meeting.assistant;
        assert_eq!(a.agent.as_deref(), Some("codex"));
        assert!(a.local_only);
        assert_eq!(a.timeout_secs, 30);
        assert_eq!(a.max_transcript_chars, 5000);
        assert_eq!(a.open_command.as_deref(), Some("true"));
    }
}
