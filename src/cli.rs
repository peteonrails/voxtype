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
  voxtype record-toggle    Toggle recording (for compositor keybindings)
  voxtype record-start     Start recording
  voxtype record-stop      Stop recording and transcribe
  voxtype status           Show daemon status (integrates with Waybar)
  voxtype setup            Check dependencies and download models
  voxtype config           Show current configuration

EXAMPLES:
  voxtype setup model      Interactive model selection
  voxtype setup waybar     Show Waybar integration config
  voxtype setup gpu        Manage GPU acceleration
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

    /// Override whisper model (tiny, tiny.en, base, base.en, small, small.en, medium, medium.en, large-v3, large-v3-turbo)
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,

    /// Disable context window optimization for short recordings
    #[arg(long)]
    pub no_whisper_context_optimization: bool,

    /// Override hotkey (e.g., SCROLLLOCK, PAUSE, F13)
    #[arg(long, value_name = "KEY")]
    pub hotkey: Option<String>,

    /// Use toggle mode (press to start/stop) instead of push-to-talk (hold to record)
    #[arg(long)]
    pub toggle: bool,

    /// Delay before typing starts (ms), helps prevent first character drop
    #[arg(long, value_name = "MS")]
    pub pre_type_delay: Option<u32>,

    /// DEPRECATED: Use --pre-type-delay instead
    #[arg(long, value_name = "MS", hide = true)]
    pub wtype_delay: Option<u32>,

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

        /// Specify which model to download (use with --download)
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
}

/// Output mode override for record commands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputModeOverride {
    Type,
    Clipboard,
    Paste,
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
    },
    /// Cancel current recording or transcription (discard without output)
    Cancel,
}

impl RecordAction {
    /// Extract the output mode override from the action flags
    pub fn output_mode_override(&self) -> Option<OutputModeOverride> {
        let (type_mode, clipboard, paste) = match self {
            RecordAction::Start { type_mode, clipboard, paste } => (*type_mode, *clipboard, *paste),
            RecordAction::Stop { type_mode, clipboard, paste } => (*type_mode, *clipboard, *paste),
            RecordAction::Toggle { type_mode, clipboard, paste } => (*type_mode, *clipboard, *paste),
            RecordAction::Cancel => return None,
        };

        if type_mode {
            Some(OutputModeOverride::Type)
        } else if clipboard {
            Some(OutputModeOverride::Clipboard)
        } else if paste {
            Some(OutputModeOverride::Paste)
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

    /// Manage GPU acceleration (switch between CPU and Vulkan backends)
    Gpu {
        /// Enable GPU (Vulkan) acceleration
        #[arg(long)]
        enable: bool,

        /// Disable GPU acceleration (switch back to CPU)
        #[arg(long)]
        disable: bool,

        /// Show current backend status
        #[arg(long)]
        status: bool,
    },

    /// Compositor integration (fixes modifier key interference)
    Compositor {
        #[command(subcommand)]
        compositor_type: CompositorType,
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
            Some(Commands::Setup { no_post_install, quiet, .. }) => {
                assert!(no_post_install, "setup --no-post-install should set no_post_install=true");
                assert!(!quiet, "quiet should be false");
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_without_flags() {
        let cli = Cli::parse_from(["voxtype", "setup"]);
        match cli.command {
            Some(Commands::Setup { quiet, no_post_install, .. }) => {
                assert!(!quiet, "setup without --quiet should have quiet=false");
                assert!(!no_post_install, "setup without --no-post-install should have no_post_install=false");
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_quiet_with_download() {
        let cli = Cli::parse_from(["voxtype", "setup", "--quiet", "--download"]);
        match cli.command {
            Some(Commands::Setup { quiet, download, .. }) => {
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
            Some(Commands::Setup { quiet, no_post_install, .. }) => {
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
            Some(Commands::Setup { quiet, no_post_install, download, .. }) => {
                assert!(!quiet, "quiet should be false");
                assert!(no_post_install, "should have no_post_install=true");
                assert!(download, "should have download=true");
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_all_flags() {
        let cli = Cli::parse_from(["voxtype", "setup", "--quiet", "--no-post-install", "--download"]);
        match cli.command {
            Some(Commands::Setup { quiet, no_post_install, download, .. }) => {
                assert!(quiet, "should have quiet=true");
                assert!(no_post_install, "should have no_post_install=true");
                assert!(download, "should have download=true");
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_model_set_restart_flags() {
        let cli = Cli::parse_from(["voxtype", "setup", "model", "--set", "large-v3", "--restart"]);
        match cli.command {
            Some(Commands::Setup { action: Some(SetupAction::Model { set, restart, .. }), .. }) => {
                assert_eq!(set, Some("large-v3".to_string()));
                assert!(restart, "should have restart=true");
            }
            _ => panic!("Expected Setup Model command"),
        }
    }

    #[test]
    fn test_setup_download_with_model() {
        let cli = Cli::parse_from(["voxtype", "setup", "--download", "--model", "large-v3-turbo"]);
        match cli.command {
            Some(Commands::Setup { download, model, .. }) => {
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
            Some(Commands::Setup { download, model, .. }) => {
                assert!(!download, "download should be false");
                assert_eq!(model, Some("small.en".to_string()));
            }
            _ => panic!("Expected Setup command"),
        }
    }

    #[test]
    fn test_setup_download_model_quiet() {
        // Full non-interactive setup command
        let cli = Cli::parse_from(["voxtype", "setup", "--download", "--model", "large-v3-turbo", "--quiet"]);
        match cli.command {
            Some(Commands::Setup { download, model, quiet, .. }) => {
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
            Some(Commands::Record { action: RecordAction::Cancel }) => {
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
                assert_eq!(action.output_mode_override(), Some(OutputModeOverride::Paste));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_clipboard_override() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--clipboard"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.output_mode_override(), Some(OutputModeOverride::Clipboard));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_type_override() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--type"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.output_mode_override(), Some(OutputModeOverride::Type));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_stop_paste_override() {
        let cli = Cli::parse_from(["voxtype", "record", "stop", "--paste"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.output_mode_override(), Some(OutputModeOverride::Paste));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_toggle_paste_override() {
        let cli = Cli::parse_from(["voxtype", "record", "toggle", "--paste"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.output_mode_override(), Some(OutputModeOverride::Paste));
            }
            _ => panic!("Expected Record command"),
        }
    }
}
