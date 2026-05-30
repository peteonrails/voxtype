//! Top-level subcommand enum.

use clap::Subcommand;

use super::{ConfigAction, InfoAction, MeetingAction, RecordAction, SetupAction};

#[derive(Subcommand)]
pub enum Commands {
    /// Run as daemon (default if no command specified)
    Daemon,

    /// Run menu bar helper (macOS)
    #[cfg(target_os = "macos")]
    Menubar,

    /// Launch daemon + menubar (used by Voxtype.app bundle)
    #[cfg(target_os = "macos")]
    #[command(hide = true)]
    AppLaunch,

    /// Transcribe an audio file (WAV, 16kHz, mono)
    Transcribe {
        /// Path to audio file
        file: std::path::PathBuf,

        /// Override transcription engine
        #[arg(
            long,
            value_name = "ENGINE",
            long_help = format!("Override transcription engine: {}", super::ENGINE_NAMES_CSV),
        )]
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

    /// Show or modify configuration
    ///
    /// With no subcommand, prints the resolved configuration. Use `voxtype
    /// config set engine <NAME>` to change the active transcription engine
    /// in the on-disk config file (preserving comments and other settings).
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },

    /// Inspect runtime/install information
    Info {
        #[command(subcommand)]
        action: InfoAction,
    },

    /// Open the interactive configuration TUI
    Configure {
        /// Render as if installed from a package (for testing source builds).
        #[arg(long, hide = true)]
        force_package_mode: bool,
    },

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

    /// Meeting transcription mode
    ///
    /// Continuous meeting transcription with chunked processing,
    /// speaker attribution, and export capabilities.
    Meeting {
        #[command(subcommand)]
        action: MeetingAction,
    },

    /// Check for updates
    CheckUpdate,
}
