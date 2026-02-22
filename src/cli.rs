// Command-line interface definitions for voxtype
//
// This module is separate so it can be used by both the binary (main.rs)
// and build.rs for generating man pages.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "voxtype")]
#[command(author, version, about = "Push-to-talk voice-to-text for Linux")]
#[command(long_about = "
Voxtype is a push-to-talk voice-to-text tool for Linux.
Optimized for Wayland, works on X11 too.

COMMANDS:
  voxtype                  Start the daemon (with evdev hotkey detection)
  voxtype daemon           Same as above
  voxtype record toggle    Toggle recording (for compositor keybindings)
  voxtype record start     Start recording
  voxtype record stop      Stop recording and transcribe
  voxtype status           Show daemon status (integrates with Waybar)
  voxtype setup            Check dependencies and download models
  voxtype config           Show current configuration

EXAMPLES:
  voxtype setup model      Interactive model selection (Whisper, Parakeet, or Moonshine)
  voxtype setup waybar     Show Waybar integration config
  voxtype setup gpu        Manage GPU acceleration (Vulkan/CUDA/ROCm)
  voxtype setup onnx       Switch between Whisper and ONNX engines
  voxtype status --follow --format json   Waybar integration

See 'voxtype <command> --help' for more info on a command.
See 'man voxtype' or docs/INSTALL.md for setup instructions.
")]
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

    /// Force clipboard mode (don't try to type)
    #[arg(long)]
    pub clipboard: bool,

    /// Force paste mode (clipboard + Ctrl+V)
    #[arg(long)]
    pub paste: bool,

    /// Restore clipboard content after paste mode completes
    /// Saves clipboard before transcription and restores it after paste
    #[arg(long)]
    pub restore_clipboard: bool,

    /// Delay in milliseconds after paste before restoring clipboard (default: 200)
    #[arg(long, value_name = "MS")]
    pub restore_clipboard_delay_ms: Option<u32>,

    /// Override model for transcription.
    /// Whisper: tiny, base, small, medium, large-v3, large-v3-turbo (and .en variants).
    /// Parakeet: parakeet-tdt-0.6b-v3, parakeet-tdt-0.6b-v3-int8
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,

    /// Override transcription engine: whisper, parakeet, moonshine, sensevoice, paraformer, dolphin, omnilingual
    #[arg(long, value_name = "ENGINE")]
    pub engine: Option<String>,

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

    // -- Whisper --

    /// Disable context window optimization for short recordings
    #[arg(long, help_heading = "Whisper")]
    pub no_whisper_context_optimization: bool,

    /// Initial prompt to provide context for transcription.
    /// Hints at terminology, proper nouns, or formatting conventions.
    #[arg(long, value_name = "PROMPT", help_heading = "Whisper")]
    pub initial_prompt: Option<String>,

    /// Language for transcription (e.g., en, fr, auto, or comma-separated: en,fr,de)
    #[arg(long, value_name = "LANG", help_heading = "Whisper")]
    pub language: Option<String>,

    /// Translate non-English speech to English
    #[arg(long, help_heading = "Whisper")]
    pub translate: bool,

    /// Number of CPU threads for inference
    #[arg(long, value_name = "N", help_heading = "Whisper")]
    pub threads: Option<usize>,

    /// Run transcription in a subprocess to release GPU memory after each recording
    #[arg(long, help_heading = "Whisper")]
    pub gpu_isolation: bool,

    /// Load model on-demand when recording starts instead of keeping it loaded
    #[arg(long, help_heading = "Whisper")]
    pub on_demand_loading: bool,

    /// Whisper execution mode: local, remote, or cli
    #[arg(long, value_name = "MODE", help_heading = "Whisper")]
    pub whisper_mode: Option<String>,

    /// Secondary model for difficult audio (used with --model-modifier)
    #[arg(long, value_name = "MODEL", help_heading = "Whisper")]
    pub secondary_model: Option<String>,

    /// Enable eager input processing (transcribe chunks while recording continues)
    #[arg(long, help_heading = "Whisper")]
    pub eager_processing: bool,

    /// Remote server endpoint URL (for remote whisper mode)
    #[arg(long, value_name = "URL", help_heading = "Whisper")]
    pub remote_endpoint: Option<String>,

    /// Model name to send to remote server
    #[arg(long, value_name = "MODEL", help_heading = "Whisper")]
    pub remote_model: Option<String>,

    /// API key for remote server (or use VOXTYPE_WHISPER_API_KEY env var)
    #[arg(long, value_name = "KEY", help_heading = "Whisper")]
    pub remote_api_key: Option<String>,

    // -- Audio --

    /// Audio input device name (or "default" for system default)
    #[arg(long, value_name = "DEVICE", help_heading = "Audio")]
    pub audio_device: Option<String>,

    /// Maximum recording duration in seconds (safety limit)
    #[arg(long, value_name = "SECS", help_heading = "Audio")]
    pub max_duration: Option<u32>,

    /// Enable audio feedback sounds (beeps when recording starts/stops)
    #[arg(long, help_heading = "Audio")]
    pub audio_feedback: bool,

    /// Disable audio feedback sounds
    #[arg(long, help_heading = "Audio")]
    pub no_audio_feedback: bool,

    // -- Output --

    /// Delay before typing starts (ms), helps prevent first character drop
    #[arg(long, value_name = "MS", help_heading = "Output")]
    pub pre_type_delay: Option<u32>,

    /// DEPRECATED: Use --pre-type-delay instead
    #[arg(long, value_name = "MS", hide = true)]
    pub wtype_delay: Option<u32>,

    /// Text to append after each transcription (e.g., " " for a trailing space).
    /// Appended before auto_submit. Useful for separating sentences when dictating incrementally.
    #[arg(long, value_name = "TEXT", help_heading = "Output")]
    pub append_text: Option<String>,

    /// Output driver order for type mode (comma-separated).
    /// Available: wtype, dotool, ydotool, clipboard.
    /// Example: --driver=ydotool,wtype,clipboard
    #[arg(long, value_name = "DRIVERS", help_heading = "Output")]
    pub driver: Option<String>,

    /// Auto-submit (press Enter) after outputting transcribed text
    #[arg(long, help_heading = "Output")]
    pub auto_submit: bool,

    /// Disable auto-submit (overrides config auto_submit = true)
    #[arg(long, conflicts_with = "auto_submit", help_heading = "Output")]
    pub no_auto_submit: bool,

    /// Convert newlines to Shift+Enter instead of regular Enter
    #[arg(long, help_heading = "Output")]
    pub shift_enter_newlines: bool,

    /// Disable Shift+Enter newlines (overrides config)
    #[arg(long, conflicts_with = "shift_enter_newlines", help_heading = "Output")]
    pub no_shift_enter_newlines: bool,

    /// Delay between typed characters in milliseconds (0 = fastest)
    #[arg(long, value_name = "MS", help_heading = "Output")]
    pub type_delay: Option<u32>,

    /// Fall back to clipboard if typing fails
    #[arg(long, help_heading = "Output")]
    pub fallback_to_clipboard: bool,

    /// Disable clipboard fallback
    #[arg(long, conflicts_with = "fallback_to_clipboard", help_heading = "Output")]
    pub no_fallback_to_clipboard: bool,

    /// Enable spoken punctuation conversion (e.g., say "period" to get ".")
    #[arg(long, help_heading = "Output")]
    pub spoken_punctuation: bool,

    /// Keystroke for paste mode (e.g., ctrl+v, shift+insert, ctrl+shift+v)
    #[arg(long, value_name = "KEYS", help_heading = "Output")]
    pub paste_keys: Option<String>,

    /// Keyboard layout for dotool (e.g., de, fr)
    #[arg(long, value_name = "LAYOUT", help_heading = "Output")]
    pub dotool_xkb_layout: Option<String>,

    /// Keyboard layout variant for dotool (e.g., nodeadkeys)
    #[arg(long, value_name = "VARIANT", help_heading = "Output")]
    pub dotool_xkb_variant: Option<String>,

    /// File path for file output mode
    #[arg(long, value_name = "PATH", help_heading = "Output")]
    pub file_path: Option<std::path::PathBuf>,

    /// File write mode: overwrite or append
    #[arg(long, value_name = "MODE", help_heading = "Output")]
    pub file_mode: Option<String>,

    /// Command to run before typing output (e.g., compositor submap switch)
    #[arg(long, value_name = "CMD", help_heading = "Output")]
    pub pre_output_command: Option<String>,

    /// Command to run after typing output (e.g., reset compositor submap)
    #[arg(long, value_name = "CMD", help_heading = "Output")]
    pub post_output_command: Option<String>,

    /// Command to run when recording starts (e.g., switch to compositor submap)
    #[arg(long, value_name = "CMD", help_heading = "Output")]
    pub pre_recording_command: Option<String>,

    // -- VAD --

    /// Enable Voice Activity Detection (filter silence before transcription)
    #[arg(long, help_heading = "VAD")]
    pub vad: bool,

    /// VAD speech detection threshold (0.0-1.0, default: 0.5).
    /// Lower = more sensitive, Higher = less sensitive
    #[arg(long, value_name = "THRESHOLD", help_heading = "VAD")]
    pub vad_threshold: Option<f32>,

    /// VAD backend: auto, energy, whisper
    #[arg(long, value_name = "BACKEND", help_heading = "VAD")]
    pub vad_backend: Option<String>,

    /// Minimum speech duration in milliseconds for VAD
    #[arg(long, value_name = "MS", help_heading = "VAD")]
    pub vad_min_speech_ms: Option<u32>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run as daemon (default if no command specified)
    Daemon,

    /// Transcribe an audio file (WAV, 16kHz, mono)
    Transcribe {
        /// Path to audio file
        file: std::path::PathBuf,

        /// Override transcription engine: whisper, parakeet, moonshine, sensevoice, paraformer, dolphin, omnilingual
        #[arg(long, value_name = "ENGINE")]
        engine: Option<String>,
    },

    /// Internal: Worker process for GPU-isolated transcription
    /// Reads audio from stdin, writes transcription result to stdout
    #[command(hide = true)]
    TranscribeWorker {
        /// Model name or path (passed from parent process)
        #[arg(long)]
        model: Option<String>,

        /// Language code (passed from parent process)
        #[arg(long)]
        language: Option<String>,

        /// Enable translation to English (passed from parent process)
        #[arg(long)]
        translate: bool,

        /// Number of threads for inference (passed from parent process)
        #[arg(long)]
        threads: Option<usize>,
    },

    /// Setup and installation utilities
    Setup {
        #[command(subcommand)]
        action: Option<SetupAction>,

        /// Download model if missing (shorthand for basic setup)
        #[arg(long)]
        download: bool,

        /// Specify which model to download (use with --download).
        /// Whisper: tiny, base, small, medium, large-v3, large-v3-turbo (and .en variants).
        /// Parakeet: parakeet-tdt-0.6b-v3, parakeet-tdt-0.6b-v3-int8
        #[arg(long, value_name = "NAME")]
        model: Option<String>,

        /// Suppress all output (for scripting/automation)
        #[arg(long)]
        quiet: bool,

        /// Suppress only "Next steps" instructions
        #[arg(long)]
        no_post_install: bool,
    },

    /// Show current configuration
    Config,

    /// Show daemon status (for Waybar/polybar integration)
    Status {
        /// Continuously output status changes as JSON (for Waybar exec)
        #[arg(long)]
        follow: bool,

        /// Output format: "text" (default) or "json" (for Waybar)
        #[arg(long, default_value = "text")]
        format: String,

        /// Include extended info in JSON (model, device, backend)
        #[arg(long)]
        extended: bool,

        /// Icon theme for JSON output (emoji, nerd-font, material, phosphor, codicons, omarchy, minimal, dots, arrows, text, or path to custom theme)
        #[arg(long, value_name = "THEME")]
        icon_theme: Option<String>,
    },

    /// Control recording from external sources (compositor keybindings, scripts)
    Record {
        #[command(subcommand)]
        action: RecordAction,
    },

    /// Meeting transcription mode (Pro feature)
    ///
    /// Continuous meeting transcription with chunked processing,
    /// speaker attribution, and export capabilities.
    Meeting {
        #[command(subcommand)]
        action: MeetingAction,
    },
}

/// Output mode override for record commands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputModeOverride {
    Type,
    Clipboard,
    Paste,
    File,
}

#[derive(Subcommand)]
pub enum RecordAction {
    /// Start recording (send SIGUSR1 to daemon)
    Start {
        /// Override output mode to simulate keyboard typing
        #[arg(long = "type", group = "output_mode")]
        type_mode: bool,

        /// Override output mode to clipboard only
        #[arg(long, group = "output_mode")]
        clipboard: bool,

        /// Override output mode to paste (clipboard + Ctrl+V)
        #[arg(long, group = "output_mode")]
        paste: bool,

        /// Write transcription to a file
        /// Use --file alone to use file_path from config, or --file=path.txt for explicit path
        #[arg(long, value_name = "FILE", group = "output_mode", num_args = 0..=1, default_missing_value = "")]
        file: Option<String>,

        /// Use a specific model for this transcription (e.g., large-v3-turbo)
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,

        /// Use a named profile for post-processing (e.g., --profile slack)
        /// Profiles are defined in config.toml under [profiles.name]
        #[arg(long, value_name = "NAME")]
        profile: Option<String>,

        /// Auto-submit (press Enter) after this transcription
        #[arg(long)]
        auto_submit: bool,

        /// Disable auto-submit for this transcription (overrides config)
        #[arg(long, conflicts_with = "auto_submit")]
        no_auto_submit: bool,

        /// Use Shift+Enter for newlines in this transcription
        #[arg(long)]
        shift_enter_newlines: bool,

        /// Disable Shift+Enter newlines for this transcription (overrides config)
        #[arg(long, conflicts_with = "shift_enter_newlines")]
        no_shift_enter_newlines: bool,
    },
    /// Stop recording and transcribe (send SIGUSR2 to daemon)
    Stop {
        /// Override output mode to simulate keyboard typing
        #[arg(long = "type", group = "output_mode")]
        type_mode: bool,

        /// Override output mode to clipboard only
        #[arg(long, group = "output_mode")]
        clipboard: bool,

        /// Override output mode to paste (clipboard + Ctrl+V)
        #[arg(long, group = "output_mode")]
        paste: bool,
    },
    /// Toggle recording state
    Toggle {
        /// Override output mode to simulate keyboard typing
        #[arg(long = "type", group = "output_mode")]
        type_mode: bool,

        /// Override output mode to clipboard only
        #[arg(long, group = "output_mode")]
        clipboard: bool,

        /// Override output mode to paste (clipboard + Ctrl+V)
        #[arg(long, group = "output_mode")]
        paste: bool,

        /// Write transcription to a file
        /// Use --file alone to use file_path from config, or --file=path.txt for explicit path
        #[arg(long, value_name = "FILE", group = "output_mode", num_args = 0..=1, default_missing_value = "")]
        file: Option<String>,

        /// Use a specific model for this transcription (e.g., large-v3-turbo)
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,

        /// Use a named profile for post-processing (e.g., --profile slack)
        /// Profiles are defined in config.toml under [profiles.name]
        #[arg(long, value_name = "NAME")]
        profile: Option<String>,

        /// Auto-submit (press Enter) after this transcription
        #[arg(long)]
        auto_submit: bool,

        /// Disable auto-submit for this transcription (overrides config)
        #[arg(long, conflicts_with = "auto_submit")]
        no_auto_submit: bool,

        /// Use Shift+Enter for newlines in this transcription
        #[arg(long)]
        shift_enter_newlines: bool,

        /// Disable Shift+Enter newlines for this transcription (overrides config)
        #[arg(long, conflicts_with = "shift_enter_newlines")]
        no_shift_enter_newlines: bool,
    },
    /// Cancel current recording or transcription (discard without output)
    Cancel,
}

/// Meeting mode actions
#[derive(Subcommand)]
pub enum MeetingAction {
    /// Start a new meeting transcription
    Start {
        /// Meeting title (optional)
        #[arg(long, short)]
        title: Option<String>,
    },
    /// Stop the current meeting
    Stop,
    /// Pause the current meeting
    Pause,
    /// Resume a paused meeting
    Resume,
    /// Show meeting status
    Status,
    /// List past meetings
    List {
        /// Maximum number of meetings to show
        #[arg(long, short, default_value = "10")]
        limit: u32,
    },
    /// Export a meeting transcript
    Export {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// Output format: text, markdown, json
        #[arg(long, short, default_value = "markdown")]
        format: String,

        /// Output file path (default: stdout)
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,

        /// Include timestamps in output
        #[arg(long)]
        timestamps: bool,

        /// Include speaker labels in output
        #[arg(long)]
        speakers: bool,

        /// Include metadata header in output
        #[arg(long)]
        metadata: bool,
    },
    /// Show meeting details
    Show {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,
    },
    /// Delete a meeting
    Delete {
        /// Meeting ID
        meeting_id: String,

        /// Skip confirmation prompt
        #[arg(long, short)]
        force: bool,
    },
    /// Label a speaker in a meeting transcript
    ///
    /// Assigns a human-readable name to an auto-generated speaker ID.
    /// Use with ML diarization to replace "SPEAKER_00" with "Alice".
    Label {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// Speaker ID to label (e.g., "SPEAKER_00" or just "0")
        speaker_id: String,

        /// Human-readable label to assign
        label: String,
    },
    /// Generate an AI summary of a meeting
    ///
    /// Uses Ollama or a remote API to generate a summary with
    /// key points, action items, and decisions.
    Summarize {
        /// Meeting ID (or "latest" for most recent)
        meeting_id: String,

        /// Output format: text, json, or markdown
        #[arg(long, short, default_value = "markdown")]
        format: String,

        /// Output file path (default: stdout)
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,
    },
}

impl RecordAction {
    /// Extract the output mode override from the action flags
    /// Returns (mode_override, optional_file_path)
    pub fn output_mode_override(&self) -> Option<OutputModeOverride> {
        let (type_mode, clipboard, paste, file) = match self {
            RecordAction::Start {
                type_mode,
                clipboard,
                paste,
                file,
                ..
            } => (*type_mode, *clipboard, *paste, file.as_ref()),
            RecordAction::Stop {
                type_mode,
                clipboard,
                paste,
            } => (*type_mode, *clipboard, *paste, None),
            RecordAction::Toggle {
                type_mode,
                clipboard,
                paste,
                file,
                ..
            } => (*type_mode, *clipboard, *paste, file.as_ref()),
            RecordAction::Cancel => return None,
        };

        if type_mode {
            Some(OutputModeOverride::Type)
        } else if clipboard {
            Some(OutputModeOverride::Clipboard)
        } else if paste {
            Some(OutputModeOverride::Paste)
        } else if file.is_some() {
            Some(OutputModeOverride::File)
        } else {
            None
        }
    }

    /// Get the file path for --file flag (if specified with explicit path)
    /// Returns Some("") if --file was used without a path (use config's file_path)
    /// Returns Some(path) if --file=path was used
    /// Returns None if --file was not used
    pub fn file_path(&self) -> Option<&str> {
        match self {
            RecordAction::Start { file, .. } | RecordAction::Toggle { file, .. } => file.as_deref(),
            RecordAction::Stop { .. } | RecordAction::Cancel => None,
        }
    }

    /// Extract the model override from the action flags
    /// Note: --model is only available on start/toggle, not stop (model is selected at recording start)
    pub fn model_override(&self) -> Option<&str> {
        match self {
            RecordAction::Start { model, .. } => model.as_deref(),
            RecordAction::Toggle { model, .. } => model.as_deref(),
            RecordAction::Stop { .. } | RecordAction::Cancel => None,
        }
    }

    /// Get the profile name from --profile flag
    /// Returns the profile name if specified on start or toggle commands
    pub fn profile(&self) -> Option<&str> {
        match self {
            RecordAction::Start { profile, .. } => profile.as_deref(),
            RecordAction::Toggle { profile, .. } => profile.as_deref(),
            RecordAction::Stop { .. } | RecordAction::Cancel => None,
        }
    }

    /// Get the auto_submit override from --auto-submit / --no-auto-submit flags
    /// Returns Some(true) for --auto-submit, Some(false) for --no-auto-submit, None if unset
    pub fn auto_submit_override(&self) -> Option<bool> {
        let (auto_submit, no_auto_submit) = match self {
            RecordAction::Start {
                auto_submit,
                no_auto_submit,
                ..
            } => (*auto_submit, *no_auto_submit),
            RecordAction::Toggle {
                auto_submit,
                no_auto_submit,
                ..
            } => (*auto_submit, *no_auto_submit),
            RecordAction::Stop { .. } | RecordAction::Cancel => return None,
        };

        if auto_submit {
            Some(true)
        } else if no_auto_submit {
            Some(false)
        } else {
            None
        }
    }

    /// Get the shift_enter_newlines override from --shift-enter-newlines / --no-shift-enter-newlines flags
    /// Returns Some(true) to enable, Some(false) to disable, None if unset
    pub fn shift_enter_newlines_override(&self) -> Option<bool> {
        let (shift_enter, no_shift_enter) = match self {
            RecordAction::Start {
                shift_enter_newlines,
                no_shift_enter_newlines,
                ..
            } => (*shift_enter_newlines, *no_shift_enter_newlines),
            RecordAction::Toggle {
                shift_enter_newlines,
                no_shift_enter_newlines,
                ..
            } => (*shift_enter_newlines, *no_shift_enter_newlines),
            RecordAction::Stop { .. } | RecordAction::Cancel => return None,
        };

        if shift_enter {
            Some(true)
        } else if no_shift_enter {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Subcommand)]
pub enum SetupAction {
    /// Check system configuration and dependencies
    Check,

    /// Install voxtype as a systemd user service
    Systemd {
        /// Uninstall the service instead of installing
        #[arg(long)]
        uninstall: bool,

        /// Show service status
        #[arg(long)]
        status: bool,
    },

    /// Show Waybar configuration snippets
    Waybar {
        /// Output only the JSON config (for scripting)
        #[arg(long)]
        json: bool,

        /// Output only the CSS config (for scripting)
        #[arg(long)]
        css: bool,

        /// Install waybar integration (inject config and CSS)
        #[arg(long)]
        install: bool,

        /// Uninstall waybar integration (remove config and CSS)
        #[arg(long)]
        uninstall: bool,
    },

    /// DankMaterialShell (DMS) integration
    Dms {
        /// Install DMS plugin (create widget directory and QML file)
        #[arg(long)]
        install: bool,

        /// Uninstall DMS plugin (remove widget directory)
        #[arg(long)]
        uninstall: bool,

        /// Output only the QML content (for scripting)
        #[arg(long)]
        qml: bool,
    },

    /// Interactive model selection and download
    Model {
        /// List installed models instead of interactive selection
        #[arg(long)]
        list: bool,

        /// Set a specific model as default (must already be downloaded)
        #[arg(long, value_name = "NAME")]
        set: Option<String>,

        /// Restart the daemon after changing model (use with --set)
        #[arg(long)]
        restart: bool,
    },

    /// Manage GPU acceleration (Vulkan for Whisper, CUDA/ROCm for Parakeet)
    Gpu {
        /// Enable GPU acceleration (auto-detects best backend)
        #[arg(long)]
        enable: bool,

        /// Disable GPU acceleration (switch back to CPU)
        #[arg(long)]
        disable: bool,

        /// Show current backend status
        #[arg(long)]
        status: bool,
    },

    /// Switch between Whisper and ONNX transcription engines
    Onnx {
        /// Enable ONNX engine (switch to ONNX binary)
        #[arg(long)]
        enable: bool,

        /// Disable ONNX engine (switch back to Whisper binary)
        #[arg(long)]
        disable: bool,

        /// Show current ONNX backend status
        #[arg(long)]
        status: bool,
    },

    /// Hidden alias for 'onnx' (backwards compatibility)
    #[command(hide = true)]
    Parakeet {
        #[arg(long)]
        enable: bool,

        #[arg(long)]
        disable: bool,

        #[arg(long)]
        status: bool,
    },

    /// Compositor integration (fixes modifier key interference)
    Compositor {
        #[command(subcommand)]
        compositor_type: CompositorType,
    },

    /// Download the Silero VAD model for speech detection
    Vad {
        /// Show VAD model status
        #[arg(long)]
        status: bool,
    },
}

#[derive(Subcommand)]
pub enum CompositorType {
    /// Hyprland compositor configuration
    Hyprland {
        /// Uninstall the compositor integration
        #[arg(long)]
        uninstall: bool,

        /// Show installation status
        #[arg(long)]
        status: bool,

        /// Show config without installing (print to stdout)
        #[arg(long)]
        show: bool,
    },
    /// Sway compositor configuration
    Sway {
        /// Uninstall the compositor integration
        #[arg(long)]
        uninstall: bool,

        /// Show installation status
        #[arg(long)]
        status: bool,

        /// Show config without installing (print to stdout)
        #[arg(long)]
        show: bool,
    },
    /// River compositor configuration
    River {
        /// Uninstall the compositor integration
        #[arg(long)]
        uninstall: bool,

        /// Show installation status
        #[arg(long)]
        status: bool,

        /// Show config without installing (print to stdout)
        #[arg(long)]
        show: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_setup_quiet_flag() {
        let cli = Cli::parse_from(["voxtype", "setup", "--quiet"]);
        match cli.command {
            Some(Commands::Setup { quiet, .. }) => {
                assert!(quiet, "setup --quiet should set quiet=true");
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_no_post_install_flag() {
        let cli = Cli::parse_from(["voxtype", "setup", "--no-post-install"]);
        match cli.command {
            Some(Commands::Setup {
                no_post_install,
                quiet,
                ..
            }) => {
                assert!(
                    no_post_install,
                    "setup --no-post-install should set no_post_install=true"
                );
                assert!(!quiet, "quiet should be false");
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_without_flags() {
        let cli = Cli::parse_from(["voxtype", "setup"]);
        match cli.command {
            Some(Commands::Setup {
                quiet,
                no_post_install,
                ..
            }) => {
                assert!(!quiet, "setup without --quiet should have quiet=false");
                assert!(
                    !no_post_install,
                    "setup without --no-post-install should have no_post_install=false"
                );
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_quiet_with_download() {
        let cli = Cli::parse_from(["voxtype", "setup", "--quiet", "--download"]);
        match cli.command {
            Some(Commands::Setup {
                quiet, download, ..
            }) => {
                assert!(quiet, "should have quiet=true");
                assert!(download, "should have download=true");
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_both_quiet_flags() {
        // Both flags can be used together (quiet takes precedence)
        let cli = Cli::parse_from(["voxtype", "setup", "--quiet", "--no-post-install"]);
        match cli.command {
            Some(Commands::Setup {
                quiet,
                no_post_install,
                ..
            }) => {
                assert!(quiet, "should have quiet=true");
                assert!(no_post_install, "should have no_post_install=true");
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_no_post_install_with_download() {
        let cli = Cli::parse_from(["voxtype", "setup", "--no-post-install", "--download"]);
        match cli.command {
            Some(Commands::Setup {
                quiet,
                no_post_install,
                download,
                ..
            }) => {
                assert!(!quiet, "quiet should be false");
                assert!(no_post_install, "should have no_post_install=true");
                assert!(download, "should have download=true");
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_all_flags() {
        let cli = Cli::parse_from([
            "voxtype",
            "setup",
            "--quiet",
            "--no-post-install",
            "--download",
        ]);
        match cli.command {
            Some(Commands::Setup {
                quiet,
                no_post_install,
                download,
                ..
            }) => {
                assert!(quiet, "should have quiet=true");
                assert!(no_post_install, "should have no_post_install=true");
                assert!(download, "should have download=true");
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_model_set_restart_flags() {
        let cli = Cli::parse_from([
            "voxtype",
            "setup",
            "model",
            "--set",
            "large-v3",
            "--restart",
        ]);
        match cli.command {
            Some(Commands::Setup {
                action: Some(SetupAction::Model { set, restart, .. }),
                ..
            }) => {
                assert_eq!(set, Some("large-v3".to_string()));
                assert!(restart, "should have restart=true");
            }
            _ => panic!("Expected Setup Model command"),
        }
    }

    #[test]
    fn test_setup_download_with_model() {
        let cli = Cli::parse_from([
            "voxtype",
            "setup",
            "--download",
            "--model",
            "large-v3-turbo",
        ]);
        match cli.command {
            Some(Commands::Setup {
                download, model, ..
            }) => {
                assert!(download, "should have download=true");
                assert_eq!(model, Some("large-v3-turbo".to_string()));
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_model_without_download() {
        // --model can be specified without --download (for validation/config update of existing model)
        let cli = Cli::parse_from(["voxtype", "setup", "--model", "small.en"]);
        match cli.command {
            Some(Commands::Setup {
                download, model, ..
            }) => {
                assert!(!download, "download should be false");
                assert_eq!(model, Some("small.en".to_string()));
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_download_model_quiet() {
        // Full non-interactive setup command
        let cli = Cli::parse_from([
            "voxtype",
            "setup",
            "--download",
            "--model",
            "large-v3-turbo",
            "--quiet",
        ]);
        match cli.command {
            Some(Commands::Setup {
                download,
                model,
                quiet,
                ..
            }) => {
                assert!(download, "should have download=true");
                assert_eq!(model, Some("large-v3-turbo".to_string()));
                assert!(quiet, "should have quiet=true");
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_record_cancel() {
        let cli = Cli::parse_from(["voxtype", "record", "cancel"]);
        match cli.command {
            Some(Commands::Record {
                action: RecordAction::Cancel,
            }) => {
                // Success - cancel action parsed correctly
            }
            _ => panic!("Expected Record Cancel command"),
        }
    }

    #[test]
    fn test_record_start_no_override() {
        let cli = Cli::parse_from(["voxtype", "record", "start"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.output_mode_override(), None);
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_paste_override() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--paste"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::Paste)
                );
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_clipboard_override() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--clipboard"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::Clipboard)
                );
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_type_override() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--type"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::Type)
                );
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_stop_paste_override() {
        let cli = Cli::parse_from(["voxtype", "record", "stop", "--paste"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::Paste)
                );
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_toggle_paste_override() {
        let cli = Cli::parse_from(["voxtype", "record", "toggle", "--paste"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::Paste)
                );
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_file_with_path() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--file=out.txt"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::File)
                );
                assert_eq!(action.file_path(), Some("out.txt"));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_model_override() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--model", "large-v3-turbo"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.model_override(), Some("large-v3-turbo"));
                assert_eq!(action.output_mode_override(), None);
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_file_without_path() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--file"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::File)
                );
                assert_eq!(action.file_path(), Some("")); // Empty string means use config path
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_model_and_output_override() {
        let cli = Cli::parse_from([
            "voxtype",
            "record",
            "start",
            "--model",
            "large-v3-turbo",
            "--clipboard",
        ]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.model_override(), Some("large-v3-turbo"));
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::Clipboard)
                );
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_file_with_absolute_path() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--file=/tmp/output.txt"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::File)
                );
                assert_eq!(action.file_path(), Some("/tmp/output.txt"));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_toggle_model_override() {
        let cli = Cli::parse_from(["voxtype", "record", "toggle", "--model", "medium.en"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.model_override(), Some("medium.en"));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_toggle_file_with_path() {
        let cli = Cli::parse_from(["voxtype", "record", "toggle", "--file=out.txt"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::File)
                );
                assert_eq!(action.file_path(), Some("out.txt"));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_toggle_file_without_path() {
        let cli = Cli::parse_from(["voxtype", "record", "toggle", "--file"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::File)
                );
                assert_eq!(action.file_path(), Some("")); // Empty string means use config path
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_toggle_file_mutually_exclusive_with_clipboard() {
        let result = Cli::try_parse_from([
            "voxtype",
            "record",
            "toggle",
            "--file=out.txt",
            "--clipboard",
        ]);
        assert!(
            result.is_err(),
            "Should not allow both --file and --clipboard on toggle"
        );
    }

    #[test]
    fn test_record_start_file_mutually_exclusive_with_paste() {
        let result =
            Cli::try_parse_from(["voxtype", "record", "start", "--file=out.txt", "--paste"]);
        assert!(result.is_err(), "Should not allow both --file and --paste");
    }

    #[test]
    fn test_record_start_file_mutually_exclusive_with_clipboard() {
        let result = Cli::try_parse_from([
            "voxtype",
            "record",
            "start",
            "--file=out.txt",
            "--clipboard",
        ]);
        assert!(
            result.is_err(),
            "Should not allow both --file and --clipboard"
        );
    }

    #[test]
    fn test_record_start_file_mutually_exclusive_with_type() {
        let result =
            Cli::try_parse_from(["voxtype", "record", "start", "--file=out.txt", "--type"]);
        assert!(result.is_err(), "Should not allow both --file and --type");
    }

    #[test]
    fn test_record_cancel_no_model() {
        let cli = Cli::parse_from(["voxtype", "record", "cancel"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.model_override(), None);
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_with_profile() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--profile", "slack"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.profile(), Some("slack"));
            }
            _ => panic!("Expected Record command"),
        }
    }

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
    // Profile flag tests
    // =========================================================================

    #[test]
    fn test_record_toggle_with_profile() {
        let cli = Cli::parse_from(["voxtype", "record", "toggle", "--profile", "code"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.profile(), Some("code"));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_without_profile() {
        let cli = Cli::parse_from(["voxtype", "record", "start"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.profile(), None);
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_stop_has_no_profile() {
        // Stop command doesn't have --profile flag
        let cli = Cli::parse_from(["voxtype", "record", "stop"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.profile(), None);
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_cancel_has_no_profile() {
        let cli = Cli::parse_from(["voxtype", "record", "cancel"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.profile(), None);
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_profile_with_output_mode() {
        // Profile can be used together with output mode overrides
        let cli = Cli::parse_from([
            "voxtype",
            "record",
            "start",
            "--profile",
            "slack",
            "--clipboard",
        ]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.profile(), Some("slack"));
                assert_eq!(
                    action.output_mode_override(),
                    Some(OutputModeOverride::Clipboard)
                );
            }
            _ => panic!("Expected Record command"),
        }
    }

    // =========================================================================
    // DMS setup tests
    // =========================================================================

    #[test]
    fn test_setup_dms_install() {
        let cli = Cli::parse_from(["voxtype", "setup", "dms", "--install"]);
        match cli.command {
            Some(Commands::Setup {
                action:
                    Some(SetupAction::Dms {
                        install,
                        uninstall,
                        qml,
                    }),
                ..
            }) => {
                assert!(install, "should have install=true");
                assert!(!uninstall, "should have uninstall=false");
                assert!(!qml, "should have qml=false");
            }
            _ => panic!("Expected Setup Dms command"),
        }
    }

    #[test]
    fn test_setup_dms_uninstall() {
        let cli = Cli::parse_from(["voxtype", "setup", "dms", "--uninstall"]);
        match cli.command {
            Some(Commands::Setup {
                action:
                    Some(SetupAction::Dms {
                        install,
                        uninstall,
                        qml,
                    }),
                ..
            }) => {
                assert!(!install, "should have install=false");
                assert!(uninstall, "should have uninstall=true");
                assert!(!qml, "should have qml=false");
            }
            _ => panic!("Expected Setup Dms command"),
        }
    }

    #[test]
    fn test_setup_dms_qml() {
        let cli = Cli::parse_from(["voxtype", "setup", "dms", "--qml"]);
        match cli.command {
            Some(Commands::Setup {
                action:
                    Some(SetupAction::Dms {
                        install,
                        uninstall,
                        qml,
                    }),
                ..
            }) => {
                assert!(!install, "should have install=false");
                assert!(!uninstall, "should have uninstall=false");
                assert!(qml, "should have qml=true");
            }
            _ => panic!("Expected Setup Dms command"),
        }
    }

    #[test]
    fn test_setup_dms_default() {
        let cli = Cli::parse_from(["voxtype", "setup", "dms"]);
        match cli.command {
            Some(Commands::Setup {
                action:
                    Some(SetupAction::Dms {
                        install,
                        uninstall,
                        qml,
                    }),
                ..
            }) => {
                assert!(!install, "should have install=false");
                assert!(!uninstall, "should have uninstall=false");
                assert!(!qml, "should have qml=false");
            }
            _ => panic!("Expected Setup Dms command"),
        }
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

    #[test]
    fn test_record_start_auto_submit() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--auto-submit"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.auto_submit_override(), Some(true));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_no_auto_submit() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--no-auto-submit"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.auto_submit_override(), Some(false));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_auto_submit_default() {
        let cli = Cli::parse_from(["voxtype", "record", "start"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.auto_submit_override(), None);
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_toggle_auto_submit() {
        let cli = Cli::parse_from(["voxtype", "record", "toggle", "--auto-submit"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.auto_submit_override(), Some(true));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_shift_enter_newlines() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--shift-enter-newlines"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.shift_enter_newlines_override(), Some(true));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_no_shift_enter_newlines() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--no-shift-enter-newlines"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.shift_enter_newlines_override(), Some(false));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_shift_enter_default() {
        let cli = Cli::parse_from(["voxtype", "record", "start"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.shift_enter_newlines_override(), None);
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_stop_has_no_auto_submit() {
        let cli = Cli::parse_from(["voxtype", "record", "stop"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.auto_submit_override(), None);
                assert_eq!(action.shift_enter_newlines_override(), None);
            }
            _ => panic!("Expected Record command"),
        }
    }
}
