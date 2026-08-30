//! Top-level subcommand enum.

use clap::builder::PossibleValuesParser;
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

        /// Also switch the config to use the model, the way the interactive
        /// picker does: sets `engine` and `<engine>.model`.
        ///
        /// Off by default. Downloading a model does not select it, so a GUI's
        /// Download button and scripted pre-downloads can't change which
        /// engine the daemon loads. Select a model explicitly with
        /// `voxtype config set <engine>.model <NAME>`.
        #[arg(long)]
        activate: bool,

        /// How download progress is reported: human (curl's progress bar) or
        /// json (one NDJSON event per update on stdout, for a GUI to render).
        ///
        /// json implies --quiet for human status lines; stdout carries only
        /// events. Failures still exit non-zero, with an "error" event
        /// alongside.
        #[arg(
            long,
            value_name = "FORMAT",
            default_value = "human",
            value_parser = PossibleValuesParser::new(super::PROGRESS_FORMATS),
            env = "VOXTYPE_PROGRESS_FORMAT",
        )]
        progress_format: String,
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

    /// Teach [text.replacements] from an edited dictation
    ///
    /// Diffs the corrected text against the last transcript and merges
    /// only word/phrase replacements into the config file. Inserts and
    /// deletes are ignored. Low-similarity selections are refused so a
    /// random highlight cannot poison the dictionary.
    #[command(long_about = "\
        Teach [text.replacements] from an edited dictation\n\n\
        Reads the corrected text (Wayland primary selection by default, \
        falling back to the clipboard), diffs it against the last \
        transcript, and writes replace-opcode phrases into \
        [text.replacements], preserving comments.\n\n\
        The last transcript is the file the daemon writes after each \
        dictation ($XDG_RUNTIME_DIR/voxtype/last-transcript). If that \
        file is missing, the last tracing line `Transcribed: \"...\"` \
        is parsed from `journalctl --user -u voxtype`.\n\n\
        Exit codes: 0 on success (including identical text or \
        insert/delete-only diffs), 1 when the selection is empty, no \
        last transcript exists, or similarity is below 0.35.\n\n\
        Examples:\n  \
        voxtype learn\n  \
        voxtype learn --from-selection\n  \
        voxtype learn --from-clipboard\n  \
        echo 'Het Omarchy menu doet het niet.' | voxtype learn --from-stdin\n\n\
        Hyprland: bind = SUPER SHIFT, F12, exec, voxtype learn --from-selection")]
    Learn {
        /// Read corrected text from the Wayland primary selection (default).
        /// Falls back to the clipboard if the selection is empty.
        #[arg(long)]
        from_selection: bool,

        /// Read corrected text from the clipboard
        #[arg(long, conflicts_with = "from_selection")]
        from_clipboard: bool,

        /// Read corrected text from stdin
        #[arg(long, conflicts_with_all = ["from_selection", "from_clipboard"])]
        from_stdin: bool,
    },

    /// Check for updates
    CheckUpdate,
}
