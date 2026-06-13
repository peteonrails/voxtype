//! Top-level `Cli` parser struct.
//!
//! The struct lives here in isolation so the rest of the CLI tree
//! (subcommand enums) can be discovered as sibling modules under
//! `src/cli/`. The clap derive on `Cli` references `Commands`,
//! which is imported from `super`.

use clap::Parser;

use super::Commands;
use super::ENGINE_NAMES_CSV;

#[derive(Parser)]
#[command(name = "voxtype")]
#[command(author, version, about = "Push-to-talk voice-to-text for Linux")]
#[command(long_about = "\
Voxtype is a push-to-talk voice-to-text tool for Linux.\n\
Optimized for Wayland, works on X11 too.")]
#[command(after_help = "\
QUICK START:
  voxtype                     Start daemon with hotkey detection
  voxtype record toggle       Toggle recording (for compositor keybindings)
  voxtype setup model         Interactive model selection
  voxtype setup gpu           Manage GPU acceleration
  voxtype status --follow     Watch daemon status (Waybar integration)

See 'voxtype --help' for all options or 'man voxtype' for full docs.")]
#[command(after_long_help = "\
QUICK START:
  voxtype                     Start daemon with hotkey detection
  voxtype daemon              Same as above (explicit)
  voxtype record toggle       Toggle recording (for compositor keybindings)
  voxtype record start        Start recording
  voxtype record stop         Stop recording and transcribe
  voxtype record cancel       Cancel current recording
  voxtype status              Show daemon status
  voxtype setup               Check dependencies and download models
  voxtype config              Show current configuration

EXAMPLES:
  voxtype setup model         Interactive model selection (Whisper, Parakeet, or Moonshine)
  voxtype setup waybar        Show Waybar integration config
  voxtype setup gpu           Manage GPU acceleration (Vulkan/CUDA/MIGraphX)
  voxtype setup onnx          Switch between Whisper and ONNX engines
  voxtype status --follow --format json   Waybar integration

See 'voxtype <command> --help' for more info on a command.
See 'man voxtype' or docs/INSTALL.md for setup instructions.")]
pub struct Cli {
    /// Path to config file
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<std::path::PathBuf>,

    /// Increase verbosity (-v = debug, -vv = trace)
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Quiet mode (errors only)
    #[arg(short, long)]
    pub quiet: bool,

    // -- Transcription (engine-agnostic) --
    /// Override transcription model
    #[arg(
        long,
        value_name = "MODEL",
        help_heading = "Transcription",
        long_help = "Override model for transcription.\n\
        Whisper: tiny, base, small, medium, large-v3, large-v3-turbo (and .en variants).\n\
        Parakeet: parakeet-tdt-0.6b-v3, parakeet-tdt-0.6b-v3-int8"
    )]
    pub model: Option<String>,

    /// Override transcription engine
    #[arg(
        long,
        value_name = "ENGINE",
        help_heading = "Transcription",
        long_help = format!("Override transcription engine: {}", ENGINE_NAMES_CSV),
    )]
    pub engine: Option<String>,

    /// Language for transcription (e.g., en, fr, auto, or comma-separated: en,fr,de)
    #[arg(long, value_name = "LANG", help_heading = "Transcription")]
    pub language: Option<String>,

    /// Translate non-English speech to English
    #[arg(long, help_heading = "Transcription")]
    pub translate: bool,

    /// Number of CPU threads for inference
    #[arg(long, value_name = "N", help_heading = "Transcription")]
    pub threads: Option<usize>,

    /// Run transcription in a subprocess to release GPU memory after each recording
    #[arg(long, help_heading = "Transcription", hide_short_help = true)]
    pub gpu_isolation: bool,

    /// GPU device index for multi-GPU systems (e.g., 1 for discrete GPU)
    #[arg(
        long,
        value_name = "INDEX",
        help_heading = "Transcription",
        hide_short_help = true
    )]
    pub gpu_device: Option<i32>,

    /// Load model on-demand when recording starts instead of keeping it loaded
    #[arg(long, help_heading = "Transcription", hide_short_help = true)]
    pub on_demand_loading: bool,

    /// Secondary model for difficult audio (used with --model-modifier)
    #[arg(
        long,
        value_name = "MODEL",
        help_heading = "Transcription",
        hide_short_help = true
    )]
    pub secondary_model: Option<String>,

    /// Enable eager input processing (transcribe chunks while recording continues)
    #[arg(long, help_heading = "Transcription", hide_short_help = true)]
    pub eager_processing: bool,

    // -- Whisper-specific --
    /// Disable context window optimization for short recordings
    #[arg(long, help_heading = "Whisper", hide_short_help = true)]
    pub no_whisper_context_optimization: bool,

    /// Initial prompt to provide context for transcription
    #[arg(
        long,
        value_name = "PROMPT",
        help_heading = "Whisper",
        hide_short_help = true,
        long_help = "Initial prompt to provide context for transcription.\n\
        Hints at terminology, proper nouns, or formatting conventions."
    )]
    pub initial_prompt: Option<String>,

    /// Enable flash attention for reduced GPU memory usage and faster inference
    #[arg(long, help_heading = "Whisper", hide_short_help = true)]
    pub flash_attention: bool,

    /// Whisper execution mode: local, remote, or cli
    #[arg(
        long,
        value_name = "MODE",
        help_heading = "Whisper",
        hide_short_help = true
    )]
    pub whisper_mode: Option<String>,

    /// Remote server endpoint URL (for remote whisper mode)
    #[arg(
        long,
        value_name = "URL",
        help_heading = "Whisper",
        hide_short_help = true
    )]
    pub remote_endpoint: Option<String>,

    /// Model name to send to remote server
    #[arg(
        long,
        value_name = "MODEL",
        help_heading = "Whisper",
        hide_short_help = true
    )]
    pub remote_model: Option<String>,

    /// API key for remote server (or use VOXTYPE_WHISPER_API_KEY env var)
    #[arg(
        long,
        value_name = "KEY",
        help_heading = "Whisper",
        hide_short_help = true
    )]
    pub remote_api_key: Option<String>,

    // -- Soniox --
    /// API key for Soniox (or use SONIOX_API_KEY env var)
    #[arg(
        long,
        value_name = "KEY",
        help_heading = "Soniox",
        hide_short_help = true
    )]
    pub soniox_api_key: Option<String>,

    // -- Hotkey --
    /// Override hotkey (e.g., SCROLLLOCK, PAUSE, F13, MEDIA, WEV_234, EVTEST_226)
    #[arg(long, value_name = "KEY", help_heading = "Hotkey")]
    pub hotkey: Option<String>,

    /// Use toggle mode (press to start/stop) instead of push-to-talk (hold to record)
    #[arg(long, help_heading = "Hotkey")]
    pub toggle: bool,

    /// Disable built-in hotkey detection (use compositor keybindings instead)
    #[arg(long, help_heading = "Hotkey")]
    pub no_hotkey: bool,

    /// Cancel key for aborting recording or transcription (e.g., ESC, BACKSPACE, F12)
    #[arg(long, value_name = "KEY", help_heading = "Hotkey")]
    pub cancel_key: Option<String>,

    /// Modifier key for secondary model selection (e.g., LEFTSHIFT)
    #[arg(long, value_name = "KEY", help_heading = "Hotkey")]
    pub model_modifier: Option<String>,

    // -- Audio --
    /// Audio input device name (or "default" for system default)
    #[arg(long, value_name = "DEVICE", help_heading = "Audio")]
    pub audio_device: Option<String>,

    /// Maximum recording duration in seconds (safety limit)
    #[arg(
        long,
        value_name = "SECS",
        help_heading = "Audio",
        hide_short_help = true
    )]
    pub max_duration: Option<u32>,

    /// Enable audio feedback sounds (beeps when recording starts/stops)
    #[arg(long, help_heading = "Audio")]
    pub audio_feedback: bool,

    /// Disable audio feedback sounds
    #[arg(
        long,
        help_heading = "Audio",
        hide_short_help = true,
        conflicts_with = "audio_feedback"
    )]
    pub no_audio_feedback: bool,

    /// Pause MPRIS media players during recording (requires playerctl)
    #[arg(long, help_heading = "Audio", hide_short_help = true)]
    pub pause_media: bool,

    /// Wait for input device warm-up before signaling recording start (overrides a config that disables it; on by default)
    #[arg(long, help_heading = "Audio", hide_short_help = true)]
    pub wait_for_device: bool,

    /// Start immediately without waiting for input device warm-up
    #[arg(
        long,
        help_heading = "Audio",
        hide_short_help = true,
        conflicts_with = "wait_for_device",
        long_help = "Start immediately without waiting for input device warm-up.\n\
        By default, voxtype delays the recording-start cue (sound, OSD, notification)\n\
        until the input device delivers real audio, because devices resuming from\n\
        idle suspend produce ~0.5s of silence in which speech is lost."
    )]
    pub no_wait_for_device: bool,

    // -- Output (delivery, timing, file output, hooks) --
    /// Force clipboard mode (don't try to type)
    #[arg(long, help_heading = "Output")]
    pub clipboard: bool,

    /// Force paste mode (clipboard + Ctrl+V)
    #[arg(long, help_heading = "Output")]
    pub paste: bool,

    /// Restore clipboard after paste mode
    #[arg(
        long,
        help_heading = "Output",
        long_help = "Restore clipboard content after paste mode completes.\n\
        Saves clipboard before transcription and restores it after paste."
    )]
    pub restore_clipboard: bool,

    /// Delay in milliseconds after paste before restoring clipboard (default: 200)
    #[arg(
        long,
        value_name = "MS",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub restore_clipboard_delay_ms: Option<u32>,

    /// Output driver order (comma-separated)
    #[arg(
        long,
        value_name = "DRIVERS",
        help_heading = "Output",
        long_help = "Output driver order for type mode (comma-separated).\n\
        Available: wtype, dotool, ydotool, clipboard.\n\
        Example: --driver=ydotool,wtype,clipboard"
    )]
    pub driver: Option<String>,

    /// Auto-submit (press Enter) after outputting transcribed text
    #[arg(long, help_heading = "Output")]
    pub auto_submit: bool,

    /// Disable auto-submit (overrides config auto_submit = true)
    #[arg(
        long,
        conflicts_with = "auto_submit",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub no_auto_submit: bool,

    /// Fall back to clipboard if typing fails
    #[arg(long, help_heading = "Output")]
    pub fallback_to_clipboard: bool,

    /// Disable clipboard fallback
    #[arg(
        long,
        conflicts_with = "fallback_to_clipboard",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub no_fallback_to_clipboard: bool,

    /// Keystroke for paste mode (e.g., ctrl+v, shift+insert, ctrl+shift+v)
    #[arg(
        long,
        value_name = "KEYS",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub paste_keys: Option<String>,

    /// File path for file output mode
    #[arg(long, value_name = "PATH", help_heading = "Output")]
    pub file_path: Option<std::path::PathBuf>,

    /// File write mode: overwrite or append
    #[arg(
        long,
        value_name = "MODE",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub file_mode: Option<String>,

    /// Delay before typing starts (ms), helps prevent first character drop
    #[arg(
        long,
        value_name = "MS",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub pre_type_delay: Option<u32>,

    /// DEPRECATED: Use --pre-type-delay instead
    #[arg(long, value_name = "MS", hide = true)]
    pub wtype_delay: Option<u32>,

    /// Prefix wtype output with a Shift key press/release
    #[arg(
        long,
        help_heading = "Output",
        hide_short_help = true,
        long_help = "Prefix wtype output with a Shift key press/release.\n\
        Workaround for apps (e.g., Discord) that drop the first CJK character."
    )]
    pub wtype_shift_prefix: bool,

    /// Delay between typed characters in milliseconds (0 = fastest)
    #[arg(
        long,
        value_name = "MS",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub type_delay: Option<u32>,

    /// Keyboard layout for dotool (e.g., de, fr)
    #[arg(
        long,
        value_name = "LAYOUT",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub dotool_xkb_layout: Option<String>,

    /// Keyboard layout variant for dotool (e.g., nodeadkeys)
    #[arg(
        long,
        value_name = "VARIANT",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub dotool_xkb_variant: Option<String>,

    /// Keyboard layout for eitype (e.g., de, ru, us). Passed as `-l <LAYOUT>`.
    /// Overrides any layout derived from the transcribed language.
    #[arg(
        long,
        value_name = "LAYOUT",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub eitype_xkb_layout: Option<String>,

    /// Keyboard layout variant for eitype (e.g., dvorak, colemak)
    #[arg(
        long,
        value_name = "VARIANT",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub eitype_xkb_variant: Option<String>,

    /// Command to run before typing output (e.g., compositor submap switch)
    #[arg(
        long,
        value_name = "CMD",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub pre_output_command: Option<String>,

    /// Command to run after typing output (e.g., reset compositor submap)
    #[arg(
        long,
        value_name = "CMD",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub post_output_command: Option<String>,

    /// Command to run when recording starts (e.g., switch to compositor submap)
    #[arg(
        long,
        value_name = "CMD",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub pre_recording_command: Option<String>,

    /// Wait for modifier keys (Ctrl/Alt/Shift/Super) to be released before typing
    #[arg(
        long,
        help_heading = "Output",
        hide_short_help = true,
        long_help = "Wait for modifier keys (Ctrl/Alt/Shift/Super) to be released before typing.\n\
        Prevents transcribed text from triggering compositor or application keybindings\n\
        when the hotkey is still held. Requires user to be in the 'input' group;\n\
        silently disabled otherwise."
    )]
    pub wait_for_modifier_release: bool,

    /// Disable waiting for modifier release (overrides config)
    #[arg(
        long,
        conflicts_with = "wait_for_modifier_release",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub no_wait_for_modifier_release: bool,

    /// Maximum milliseconds to wait for modifier release before falling back to clipboard
    #[arg(
        long,
        value_name = "MS",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub modifier_release_timeout_ms: Option<u64>,

    // -- Text Processing --
    /// Enable spoken punctuation conversion (e.g., say "period" to get ".")
    #[arg(long, help_heading = "Text Processing")]
    pub spoken_punctuation: bool,

    /// Convert newlines to Shift+Enter instead of regular Enter
    #[arg(long, help_heading = "Text Processing")]
    pub shift_enter_newlines: bool,

    /// Disable Shift+Enter newlines (overrides config)
    #[arg(
        long,
        conflicts_with = "shift_enter_newlines",
        help_heading = "Text Processing",
        hide_short_help = true
    )]
    pub no_shift_enter_newlines: bool,

    /// Enable smart auto-submit (say "submit" to press Enter)
    #[arg(long, help_heading = "Text Processing")]
    pub smart_auto_submit: bool,

    /// Disable smart auto-submit (overrides config)
    #[arg(
        long,
        conflicts_with = "smart_auto_submit",
        help_heading = "Text Processing",
        hide_short_help = true
    )]
    pub no_smart_auto_submit: bool,

    /// Filter common filler words ("uh", "um", "er", ...) from transcribed text
    #[arg(long, help_heading = "Text Processing")]
    pub filter_fillers: bool,

    /// Disable filler-word filtering (overrides config)
    #[arg(
        long,
        conflicts_with = "filter_fillers",
        help_heading = "Text Processing",
        hide_short_help = true
    )]
    pub no_filter_fillers: bool,

    /// Text to append after each transcription (e.g., " " for trailing space)
    #[arg(
        long,
        value_name = "TEXT",
        help_heading = "Text Processing",
        hide_short_help = true,
        long_help = "Text to append after each transcription (e.g., \" \" for a trailing space).\n\
        Appended before auto_submit. Useful for separating sentences when dictating incrementally."
    )]
    pub append_text: Option<String>,

    // -- VAD --
    /// Enable Voice Activity Detection (filter silence before transcription)
    #[arg(long, help_heading = "VAD")]
    pub vad: bool,

    /// VAD speech detection threshold (0.0-1.0, default: 0.5).
    /// Lower = more sensitive, Higher = less sensitive
    #[arg(
        long,
        value_name = "THRESHOLD",
        help_heading = "VAD",
        hide_short_help = true
    )]
    pub vad_threshold: Option<f32>,

    /// VAD backend: auto, energy, whisper
    #[arg(
        long,
        value_name = "BACKEND",
        help_heading = "VAD",
        hide_short_help = true
    )]
    pub vad_backend: Option<String>,

    /// Minimum speech duration in milliseconds for VAD
    #[arg(long, value_name = "MS", help_heading = "VAD", hide_short_help = true)]
    pub vad_min_speech_ms: Option<u32>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::*;
    use clap::Parser;

    // =========================================================================
    // Engine flag tests
    // =========================================================================

    #[test]
    fn test_engine_flag_whisper() {
        let cli = Cli::parse_from(["voxtype", "--engine", "whisper"]);
        assert_eq!(cli.engine, Some("whisper".to_string()));
    }

    #[test]
    fn test_engine_flag_parakeet() {
        let cli = Cli::parse_from(["voxtype", "--engine", "parakeet"]);
        assert_eq!(cli.engine, Some("parakeet".to_string()));
    }

    #[test]
    fn test_engine_flag_not_set() {
        let cli = Cli::parse_from(["voxtype"]);
        assert!(cli.engine.is_none());
    }

    #[test]
    fn test_engine_flag_with_daemon_command() {
        let cli = Cli::parse_from(["voxtype", "--engine", "parakeet", "daemon"]);
        assert_eq!(cli.engine, Some("parakeet".to_string()));
        assert!(matches!(cli.command, Some(Commands::Daemon)));
    }

    #[test]
    fn test_engine_flag_with_model_flag() {
        let cli = Cli::parse_from(["voxtype", "--engine", "whisper", "--model", "large-v3"]);
        assert_eq!(cli.engine, Some("whisper".to_string()));
        assert_eq!(cli.model, Some("large-v3".to_string()));
    }

    #[test]
    fn test_engine_flag_case_preserved() {
        // The CLI should preserve case as-is; main.rs handles case-insensitive matching
        let cli = Cli::parse_from(["voxtype", "--engine", "PARAKEET"]);
        assert_eq!(cli.engine, Some("PARAKEET".to_string()));
    }

    // =========================================================================
    // Driver flag tests
    // =========================================================================

    #[test]
    fn test_driver_flag() {
        let cli = Cli::parse_from(["voxtype", "--driver=ydotool,wtype"]);
        assert_eq!(cli.driver, Some("ydotool,wtype".to_string()));
    }

    #[test]
    fn test_driver_flag_single() {
        let cli = Cli::parse_from(["voxtype", "--driver=ydotool"]);
        assert_eq!(cli.driver, Some("ydotool".to_string()));
    }

    #[test]
    fn test_driver_flag_not_set() {
        let cli = Cli::parse_from(["voxtype"]);
        assert!(cli.driver.is_none());
    }

    // =========================================================================
    // Transcribe engine flag tests
    // =========================================================================

    #[test]
    fn test_transcribe_engine_flag() {
        let cli = Cli::parse_from(["voxtype", "transcribe", "test.wav", "--engine", "moonshine"]);
        match cli.command {
            Some(Commands::Transcribe { file, engine }) => {
                assert_eq!(file, std::path::PathBuf::from("test.wav"));
                assert_eq!(engine, Some("moonshine".to_string()));
            }
            _ => panic!("Expected Transcribe command"),
        }
    }

    #[test]
    fn test_transcribe_engine_flag_not_set() {
        let cli = Cli::parse_from(["voxtype", "transcribe", "test.wav"]);
        match cli.command {
            Some(Commands::Transcribe { engine, .. }) => {
                assert!(engine.is_none());
            }
            _ => panic!("Expected Transcribe command"),
        }
    }

    #[test]
    fn test_transcribe_engine_whisper() {
        let cli = Cli::parse_from(["voxtype", "transcribe", "test.wav", "--engine", "whisper"]);
        match cli.command {
            Some(Commands::Transcribe { engine, .. }) => {
                assert_eq!(engine, Some("whisper".to_string()));
            }
            _ => panic!("Expected Transcribe command"),
        }
    }
}
