//! Top-level `Cli` parser struct.
//!
//! The struct lives here in isolation so the rest of the CLI tree
//! (subcommand enums) can be discovered as sibling modules under
//! `src/cli/`. The clap derive on `Cli` references `Commands`,
//! which is imported from `super`.

use clap::Parser;

use super::Commands;
use super::ENGINE_NAMES_CSV;

fn parse_positive_f32(value: &str) -> Result<f32, String> {
    let parsed = value
        .parse::<f32>()
        .map_err(|_| format!("'{value}' is not a number"))?;
    if parsed.is_finite() && parsed > 0.0 {
        Ok(parsed)
    } else {
        Err("value must be a positive finite number".to_string())
    }
}

#[derive(Parser)]
#[command(name = "voxtype")]
#[command(author, version = super::VERSION, about = "Push-to-talk voice-to-text for Linux")]
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
  voxtype doctor              Check the setup and show how to fix problems

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
  voxtype doctor              Check the setup and show how to fix problems
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

    /// Increase verbosity (-v = debug, -vv = trace). Dictated text is only
    /// logged from debug up
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Native Linux tray (enabled by default). Use --tray=false to disable.
    #[arg(long, num_args = 0..=1, default_missing_value = "true", require_equals = true)]
    pub tray: Option<bool>,

    /// On Omarchy, link the packaged Settings panel on daemon start (enabled
    /// by default). Use --omarchy-link-settings-plugin=false to opt out.
    #[arg(
        long,
        value_name = "BOOL",
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true
    )]
    pub omarchy_link_settings_plugin: Option<bool>,

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
        Parakeet: parakeet-tdt-0.6b-v3, parakeet-tdt-0.6b-v3-int8.\n\
        Other engines take the directory-form names from `voxtype info models`\n\
        (e.g. moonshine-base, cohere-transcribe-q4f16). A model from another\n\
        engine selects that engine; a Whisper name only sets [whisper] model.\n\
        With `record start`, applies to that one recording on any engine."
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

    /// Recover lost streaming dictation with the configured batch backend
    /// (enabled by default). Use --streaming-recovery=false to disable.
    #[arg(
        long,
        value_name = "BOOL",
        help_heading = "Streaming",
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true
    )]
    pub streaming_recovery: Option<bool>,

    /// Maximum seconds (1-120, built-in default 30) to wait for a streaming
    /// backend to finish after stop before recovering the dictation.
    #[arg(
        long,
        value_name = "SECS",
        help_heading = "Streaming",
        hide_short_help = true,
        value_parser = clap::value_parser!(u64).range(1..=120)
    )]
    pub streaming_stop_timeout_secs: Option<u64>,

    /// GGUF checkpoint path for the shared native engine
    #[arg(
        long,
        value_name = "FILE",
        help_heading = "GGUF",
        hide_short_help = true
    )]
    pub gguf_model: Option<String>,

    /// Expected GGUF general.architecture value
    #[arg(
        long,
        value_name = "ARCH",
        help_heading = "GGUF",
        hide_short_help = true
    )]
    pub gguf_architecture: Option<String>,

    /// Language hint for the selected GGUF checkpoint
    #[arg(
        long,
        value_name = "LANG",
        help_heading = "GGUF",
        hide_short_help = true
    )]
    pub gguf_language: Option<String>,

    /// transcribe.cpp backend: auto, cpu, cpu_accel, vulkan, metal, cuda, or rocm
    #[arg(
        long,
        value_name = "BACKEND",
        help_heading = "GGUF",
        hide_short_help = true
    )]
    pub gguf_backend: Option<String>,

    /// Request native GGUF streaming (currently rejected explicitly)
    #[arg(long, help_heading = "GGUF", hide_short_help = true)]
    pub gguf_streaming: bool,

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

    /// Soniox data-residency region of your API key's project: us, eu, jp, in
    /// (or use VOXTYPE_SONIOX_REGION env var)
    #[arg(
        long,
        value_name = "REGION",
        value_parser = ["us", "eu", "jp", "in"],
        help_heading = "Soniox",
        hide_short_help = true
    )]
    pub soniox_region: Option<String>,

    // -- ElevenLabs --
    /// API key for ElevenLabs (or use ELEVENLABS_API_KEY env var)
    #[arg(
        long,
        value_name = "KEY",
        help_heading = "ElevenLabs",
        hide_short_help = true
    )]
    pub elevenlabs_api_key: Option<String>,

    /// ElevenLabs API region
    #[arg(
        long,
        value_name = "REGION",
        value_parser = ["global", "us", "eu", "india", "singapore"],
        help_heading = "ElevenLabs",
        hide_short_help = true
    )]
    pub elevenlabs_region: Option<String>,

    /// Language hint for ElevenLabs (ISO 639-1 or ISO 639-3)
    #[arg(
        long,
        value_name = "LANG",
        help_heading = "ElevenLabs",
        hide_short_help = true
    )]
    pub elevenlabs_language: Option<String>,

    /// ElevenLabs dictation mode: batch, realtime, or partials
    #[arg(
        long,
        value_name = "MODE",
        value_parser = ["batch", "realtime", "partials"],
        help_heading = "ElevenLabs",
        hide_short_help = true
    )]
    pub elevenlabs_mode: Option<String>,

    /// ElevenLabs model for batch transcription (e.g., scribe_v3)
    #[arg(
        long,
        value_name = "MODEL",
        help_heading = "ElevenLabs",
        hide_short_help = true
    )]
    pub elevenlabs_model: Option<String>,

    /// ElevenLabs model for realtime and partials modes (e.g., scribe_v3_live)
    #[arg(
        long,
        value_name = "MODEL",
        help_heading = "ElevenLabs",
        hide_short_help = true
    )]
    pub elevenlabs_realtime_model: Option<String>,

    /// When ElevenLabs commits realtime segments: vad or manual
    #[arg(
        long,
        value_name = "STRATEGY",
        value_parser = ["vad", "manual"],
        help_heading = "ElevenLabs",
        hide_short_help = true
    )]
    pub elevenlabs_commit_strategy: Option<String>,

    /// Remove provider-detected filler words, false starts, and disfluencies
    #[arg(
        long,
        help_heading = "ElevenLabs",
        hide_short_help = true,
        conflicts_with = "elevenlabs_verbatim"
    )]
    pub elevenlabs_no_verbatim: bool,

    /// Preserve filler words, false starts, and disfluencies
    #[arg(
        long,
        help_heading = "ElevenLabs",
        hide_short_help = true,
        conflicts_with = "elevenlabs_no_verbatim"
    )]
    pub elevenlabs_verbatim: bool,

    /// Seconds of silence before ElevenLabs commits a realtime segment
    #[arg(
        long,
        value_name = "SECS",
        value_parser = parse_positive_f32,
        help_heading = "ElevenLabs",
        hide_short_help = true
    )]
    pub elevenlabs_vad_silence_threshold_secs: Option<f32>,

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

    /// Key that pastes the last dictation again (e.g., CTRL+ALT+V, F9); needs history on
    #[arg(long, value_name = "KEY", help_heading = "Hotkey")]
    pub paste_last_key: Option<String>,

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

    /// Keep the audio of a recording whose transcription failed (default)
    #[arg(
        long,
        help_heading = "Audio",
        hide_short_help = true,
        long_help = "Keep the audio of a recording whose transcription failed, so it can be \
        retried with 'voxtype transcribe <file>'. This is the default. The 20 most recent \
        are kept in ~/.local/state/voxtype/failed-audio/, readable only by you."
    )]
    pub keep_failed_recordings: bool,

    /// Don't keep the audio of a recording whose transcription failed
    #[arg(
        long,
        help_heading = "Audio",
        hide_short_help = true,
        conflicts_with = "keep_failed_recordings",
        long_help = "Don't write the audio of a failed transcription to disk (overrides \
        config and environment). A dictation whose transcription fails is then lost."
    )]
    pub no_keep_failed_recordings: bool,

    /// Pause MPRIS media players during recording
    #[arg(long, help_heading = "Audio", hide_short_help = true)]
    pub pause_media: bool,

    /// Lower active media volume during recording instead of pausing it
    #[arg(long, help_heading = "Audio", hide_short_help = true)]
    pub duck_media: bool,

    /// Percent of its current amplitude ducked media keeps (50 = half, -6 dB)
    #[arg(
        long,
        value_name = "PERCENT",
        help_heading = "Audio",
        hide_short_help = true
    )]
    pub duck_media_volume: Option<u8>,

    /// Milliseconds to fade ducked media down and back up (0 = instant)
    #[arg(
        long,
        value_name = "MS",
        help_heading = "Audio",
        hide_short_help = true
    )]
    pub duck_media_fade_ms: Option<u32>,

    /// Stop a recording when the microphone delivers only exact digital silence
    #[arg(long, help_heading = "Audio", hide_short_help = true)]
    pub dead_input_check: bool,

    /// Keep recording even when the microphone delivers only digital silence
    #[arg(
        long,
        help_heading = "Audio",
        hide_short_help = true,
        conflicts_with = "dead_input_check"
    )]
    pub no_dead_input_check: bool,

    /// Milliseconds of all-zero microphone input before the recording is stopped
    #[arg(
        long,
        value_name = "MS",
        help_heading = "Audio",
        hide_short_help = true
    )]
    pub dead_input_window: Option<u32>,

    /// Microphone level, in dBFS, that ends the silent-input check early
    #[arg(
        long,
        value_name = "DBFS",
        help_heading = "Audio",
        hide_short_help = true,
        allow_hyphen_values = true
    )]
    pub dead_input_threshold: Option<f32>,

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
        long_help = "Output driver order (comma-separated).\n\
        Available: wtype, eitype, dotool, xdotool, ydotool, clipboard, xclip, hyprland.\n\
        Paste mode uses the keystroke drivers in this order (wtype, eitype,\n\
        ydotool, hyprland). hyprland sends the paste keystroke through\n\
        Hyprland's own dispatcher; it is paste-only and opt-in.\n\
        Env: VOXTYPE_DRIVER_ORDER\n\
        Example: --driver=ydotool,wtype,clipboard\n\
        Example: --paste --driver=hyprland,wtype"
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

    /// Keep the text of a transcription that could not be delivered (default)
    #[arg(
        long,
        help_heading = "Output",
        hide_short_help = true,
        long_help = "Keep the text of a transcription no output method could deliver, so it \
        can be recovered. This is the default. The 200 most recent are kept in \
        ~/.local/state/voxtype/undelivered/, readable only by you, and the failure \
        notification names the file."
    )]
    pub keep_undelivered_text: bool,

    /// Don't keep the text of a transcription that could not be delivered
    #[arg(
        long,
        help_heading = "Output",
        hide_short_help = true,
        conflicts_with = "keep_undelivered_text",
        long_help = "Don't save the text of an undelivered transcription to a file, and \
        don't quote it in the failure notification (overrides config and environment). \
        A dictation that can't be typed or copied is then lost."
    )]
    pub no_keep_undelivered_text: bool,

    /// Keystroke for paste mode (e.g., ctrl+v, shift+insert, ctrl+shift+v), or
    /// "auto" (the default) for ctrl+v in GUI apps and --paste-keys-terminal in
    /// terminals
    #[arg(
        long,
        value_name = "KEYS",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub paste_keys: Option<String>,

    /// Keystroke for paste mode when the focused window is a terminal
    /// (default: ctrl+shift+v; detected on Hyprland, Sway and niri)
    #[arg(
        long,
        value_name = "KEYS",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub paste_keys_terminal: Option<String>,

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
        when the hotkey is still held. Needs to read /dev/input (the 'input' group);\n\
        without it the guard is skipped. Compositor submaps (voxtype setup compositor)\n\
        avoid the problem without that access."
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
    /// Enable English-only trained disfluency cleanup (local model required)
    #[arg(long, help_heading = "Text Processing")]
    pub lard_disfluency: bool,

    /// Disable trained disfluency cleanup
    #[arg(
        long,
        conflicts_with = "lard_disfluency",
        help_heading = "Text Processing"
    )]
    pub no_lard_disfluency: bool,

    /// Local LARD VXDF model file (never downloaded at startup)
    #[arg(long, value_name = "FILE", help_heading = "Text Processing")]
    pub lard_model_path: Option<std::path::PathBuf>,

    /// Language you dictate in, for text cleanup (e.g. en, en-US, pt-BR, auto)
    #[arg(
        long,
        value_name = "LANG",
        help_heading = "Text Processing",
        long_help = "The language you dictate in, for text cleanup that depends on it (LARD\n\
        disfluency cleanup is English-only). Set it when a multilingual model detects\n\
        the language automatically: an engine language that is set, or a model that\n\
        only writes one language, already answers the question and wins over this.\n\
        It is not sent to the transcription engine; --language is the engine's hint.\n\
        Use a code such as en, en-US or pt-BR, or auto to declare nothing."
    )]
    pub text_language: Option<String>,

    /// Restore punctuation and casing for plain CTC transcription engines
    #[arg(
        long,
        help_heading = "Text Processing",
        long_help = "Restore punctuation, sentence boundaries, and letter casing for engines that emit plain CTC text. Has no effect on engines such as Whisper, Parakeet TDT, and Cohere that already supply punctuation and casing. Requires 'voxtype setup punctuation'."
    )]
    pub punctuation_casing: bool,

    /// Disable punctuation and casing restoration (overrides config)
    #[arg(
        long,
        conflicts_with = "punctuation_casing",
        help_heading = "Text Processing",
        hide_short_help = true
    )]
    pub no_punctuation_casing: bool,

    /// Enable spoken punctuation conversion (e.g., say "period" to get ".")
    #[arg(long, help_heading = "Text Processing")]
    pub spoken_punctuation: bool,

    /// Disable spoken punctuation conversion (overrides config and environment)
    #[arg(
        long,
        conflicts_with = "spoken_punctuation",
        help_heading = "Text Processing",
        hide_short_help = true
    )]
    pub no_spoken_punctuation: bool,

    /// Collapse repeated starts, stutters, and explicit retractions (off by default)
    #[arg(long, help_heading = "Text Processing")]
    pub collapse_restarts: bool,

    /// Disable restart collapsing (overrides config and environment)
    #[arg(
        long,
        conflicts_with = "collapse_restarts",
        help_heading = "Text Processing",
        hide_short_help = true
    )]
    pub no_collapse_restarts: bool,

    /// Repair premature sentence breaks using deterministic rules (off by default)
    #[arg(long, help_heading = "Text Processing")]
    pub repair_sentence_breaks: bool,

    /// Disable sentence-break repair (overrides config and environment)
    #[arg(
        long,
        conflicts_with = "repair_sentence_breaks",
        help_heading = "Text Processing",
        hide_short_help = true
    )]
    pub no_repair_sentence_breaks: bool,

    /// Format dictated quantities, money, and times (off by default; requires itn)
    #[arg(long, help_heading = "Text Processing")]
    pub format_numbers: bool,

    /// Disable number formatting (overrides config and environment)
    #[arg(
        long,
        conflicts_with = "format_numbers",
        help_heading = "Text Processing",
        hide_short_help = true
    )]
    pub no_format_numbers: bool,

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

    // -- Local LLM --
    /// Dictation cleanup backend: command, omarchy or openai (models add ~2-6 s per dictation)
    #[arg(
        long,
        value_name = "BACKEND",
        env = "VOXTYPE_POST_PROCESS_BACKEND",
        value_parser = ["command", "omarchy", "openai"],
        help_heading = "Output",
        hide_short_help = true,
        long_help = "Who cleans up each dictation before it is typed ([output.post_process] backend).\n\
        command (default): the post-process command, as before.\n\
        omarchy: the text model Omarchy runs on this machine (`omarchy models`). Voxtype\n\
        never starts one; with none running, the original text is typed.\n\
        openai: any OpenAI-compatible server (--post-process-url, --post-process-model).\n\
        WARNING: the model backends add roughly 2 to 6 seconds to every dictation.\n\
        On a timeout or any failure the original text is typed."
    )]
    pub post_process_backend: Option<String>,

    /// Model for the post-process model backend (Omarchy recipe or model id)
    #[arg(
        long,
        value_name = "MODEL",
        env = "VOXTYPE_POST_PROCESS_MODEL",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub post_process_model: Option<String>,

    /// Server URL for the openai post-process backend, including /v1
    #[arg(
        long,
        value_name = "URL",
        env = "VOXTYPE_POST_PROCESS_URL",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub post_process_url: Option<String>,

    /// File holding the API key for the openai post-process backend
    #[arg(
        long,
        value_name = "FILE",
        env = "VOXTYPE_POST_PROCESS_API_KEY_FILE",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub post_process_api_key_file: Option<String>,

    /// Instructions for the post-process model backend (replaces the built-in prompt)
    #[arg(
        long,
        value_name = "TEXT",
        env = "VOXTYPE_POST_PROCESS_PROMPT",
        help_heading = "Output",
        hide_short_help = true
    )]
    pub post_process_prompt: Option<String>,

    /// Meeting summary backend: disabled, local, remote, omarchy, openai
    #[arg(
        long,
        value_name = "BACKEND",
        env = "VOXTYPE_SUMMARY_BACKEND",
        value_parser = ["disabled", "agent", "local", "remote", "omarchy", "openai"],
        help_heading = "Meeting",
        hide_short_help = true,
        long_help = "Backend for meeting summaries ([meeting.summary] backend).\n\
        omarchy: the text model Omarchy runs on this machine. It also answers\n\
        `meeting ask` with no agent consent, since nothing leaves the machine.\n\
        Voxtype never starts a model; with none running, summaries and asks fall\n\
        back to your agent or Ollama as before.\n\
        openai: any OpenAI-compatible server (--summary-openai-url, --summary-openai-model).\n\
        It answers asks only when the server is on this machine."
    )]
    pub summary_backend: Option<String>,

    /// Omarchy recipe for the omarchy summary backend (default: last started)
    #[arg(
        long,
        value_name = "RECIPE",
        env = "VOXTYPE_SUMMARY_OMARCHY_MODEL",
        help_heading = "Meeting",
        hide_short_help = true
    )]
    pub summary_omarchy_model: Option<String>,

    /// Server URL for the openai summary backend, including /v1
    #[arg(
        long,
        value_name = "URL",
        env = "VOXTYPE_SUMMARY_OPENAI_URL",
        help_heading = "Meeting",
        hide_short_help = true
    )]
    pub summary_openai_url: Option<String>,

    /// Model id for the openai summary backend
    #[arg(
        long,
        value_name = "MODEL",
        env = "VOXTYPE_SUMMARY_OPENAI_MODEL",
        help_heading = "Meeting",
        hide_short_help = true
    )]
    pub summary_openai_model: Option<String>,

    /// File holding the API key for the openai summary backend
    #[arg(
        long,
        value_name = "FILE",
        env = "VOXTYPE_SUMMARY_OPENAI_API_KEY_FILE",
        help_heading = "Meeting",
        hide_short_help = true
    )]
    pub summary_openai_api_key_file: Option<String>,

    // -- History --
    /// Record dictations to history. Use --history=false to turn it off for
    /// this run.
    #[arg(
        long,
        value_name = "BOOL",
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true,
        help_heading = "History",
        long_help = "Record each dictation, including failed and cancelled ones, to\n\
        ~/.local/share/voxtype/history/ ([history] enabled). --history=false\n\
        turns it off for this run. Browse it with `voxtype history`."
    )]
    pub history: Option<bool>,

    /// Pause history: nothing dictated is written until resumed
    #[arg(
        long,
        value_name = "BOOL",
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true,
        help_heading = "History",
        hide_short_help = true
    )]
    pub history_paused: Option<bool>,

    /// Days of history to keep (0 keeps it forever; default 30)
    #[arg(
        long,
        value_name = "DAYS",
        help_heading = "History",
        hide_short_help = true
    )]
    pub history_retention_days: Option<u32>,

    /// Comma-separated apps never recorded to history
    #[arg(
        long,
        value_name = "APPS",
        help_heading = "History",
        hide_short_help = true,
        long_help = "Comma-separated apps whose dictations are never recorded, matched\n\
        against the focused window's class (case, spaces and dots ignored).\n\
        Replaces the built-in list: KeePassXC, Bitwarden, 1Password, Proton Pass.\n\
        Example: --history-exclude-apps \"KeePassXC,Signal\""
    )]
    pub history_exclude_apps: Option<String>,

    /// Record which app each dictation went to (default true)
    #[arg(
        long,
        value_name = "BOOL",
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true,
        help_heading = "History",
        hide_short_help = true
    )]
    pub history_record_app: Option<bool>,

    /// Also record the focused window's title (default false)
    #[arg(
        long,
        value_name = "BOOL",
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true,
        help_heading = "History",
        hide_short_help = true
    )]
    pub history_record_window_title: Option<bool>,

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

    /// build.rs compiles this file without the crate, so the flag lists its
    /// regions literally. Keep it in step with the config's list.
    #[test]
    fn soniox_region_flag_matches_config_regions() {
        use clap::CommandFactory;
        let cmd = Cli::command();
        let arg = cmd
            .get_arguments()
            .find(|a| a.get_id() == "soniox_region")
            .expect("--soniox-region");
        let values: Vec<String> = arg
            .get_possible_values()
            .iter()
            .map(|v| v.get_name().to_string())
            .collect();
        assert_eq!(values, crate::config::SONIOX_REGIONS);
        let cli = Cli::parse_from(["voxtype", "--soniox-region", "eu"]);
        assert_eq!(cli.soniox_region.as_deref(), Some("eu"));
        assert!(Cli::try_parse_from(["voxtype", "--soniox-region", "mars"]).is_err());
    }

    #[test]
    fn keep_failed_recordings_flags_parse_and_conflict() {
        let cli = Cli::parse_from(["voxtype"]);
        assert!(!cli.keep_failed_recordings && !cli.no_keep_failed_recordings);
        let cli = Cli::parse_from(["voxtype", "--no-keep-failed-recordings"]);
        assert!(cli.no_keep_failed_recordings);
        let cli = Cli::parse_from(["voxtype", "--keep-failed-recordings"]);
        assert!(cli.keep_failed_recordings);
        assert!(Cli::try_parse_from([
            "voxtype",
            "--keep-failed-recordings",
            "--no-keep-failed-recordings"
        ])
        .is_err());
    }

    #[test]
    fn keep_undelivered_text_flags_parse_and_conflict() {
        let cli = Cli::parse_from(["voxtype"]);
        assert!(!cli.keep_undelivered_text && !cli.no_keep_undelivered_text);
        let cli = Cli::parse_from(["voxtype", "--no-keep-undelivered-text"]);
        assert!(cli.no_keep_undelivered_text);
        let cli = Cli::parse_from(["voxtype", "--keep-undelivered-text"]);
        assert!(cli.keep_undelivered_text);
        assert!(Cli::try_parse_from([
            "voxtype",
            "--keep-undelivered-text",
            "--no-keep-undelivered-text"
        ])
        .is_err());
    }

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
    fn history_flags_parse() {
        let cli = Cli::parse_from([
            "voxtype",
            "--history",
            "--history-paused=false",
            "--history-retention-days",
            "0",
            "--history-exclude-apps",
            "KeePassXC,Signal",
            "--history-record-window-title",
        ]);
        assert_eq!(cli.history, Some(true));
        assert_eq!(cli.history_paused, Some(false));
        assert_eq!(cli.history_retention_days, Some(0));
        assert_eq!(
            cli.history_exclude_apps.as_deref(),
            Some("KeePassXC,Signal")
        );
        assert_eq!(cli.history_record_app, None);
        assert_eq!(cli.history_record_window_title, Some(true));

        let cli = Cli::parse_from(["voxtype", "--history=false"]);
        assert_eq!(cli.history, Some(false));
        let cli = Cli::parse_from(["voxtype"]);
        assert_eq!(cli.history, None);
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

    #[test]
    fn streaming_recovery_flags_parse() {
        let cli = Cli::parse_from([
            "voxtype",
            "--streaming-recovery=false",
            "--streaming-stop-timeout-secs",
            "10",
        ]);
        assert_eq!(cli.streaming_recovery, Some(false));
        assert_eq!(cli.streaming_stop_timeout_secs, Some(10));

        let cli = Cli::parse_from(["voxtype", "--streaming-recovery"]);
        assert_eq!(cli.streaming_recovery, Some(true));
        assert_eq!(cli.streaming_stop_timeout_secs, None);

        let cli = Cli::parse_from(["voxtype"]);
        assert_eq!(cli.streaming_recovery, None);
    }

    #[test]
    fn streaming_stop_timeout_matches_config_range() {
        for value in ["1", "120"] {
            let cli = Cli::parse_from(["voxtype", "--streaming-stop-timeout-secs", value]);
            assert_eq!(cli.streaming_stop_timeout_secs, value.parse().ok());
        }
        for value in ["0", "121"] {
            assert!(
                Cli::try_parse_from(["voxtype", "--streaming-stop-timeout-secs", value]).is_err(),
                "--streaming-stop-timeout-secs {value} was accepted"
            );
        }
    }
}
