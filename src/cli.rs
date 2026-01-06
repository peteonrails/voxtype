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

    /// Override whisper model (tiny, base, small, medium, large-v3, large-v3-turbo)
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,

    /// Override hotkey (e.g., SCROLLLOCK, PAUSE, F13)
    #[arg(long, value_name = "KEY")]
    pub hotkey: Option<String>,

    /// Use toggle mode (press to start/stop) instead of push-to-talk (hold to record)
    #[arg(long)]
    pub toggle: bool,

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

    /// Setup and installation utilities
    Setup {
        #[command(subcommand)]
        action: Option<SetupAction>,

        /// Download model if missing (shorthand for basic setup)
        #[arg(long)]
        download: bool,

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

#[derive(Subcommand)]
pub enum RecordAction {
    /// Start recording (send SIGUSR1 to daemon)
    Start,
    /// Stop recording and transcribe (send SIGUSR2 to daemon)
    Stop,
    /// Toggle recording state
    Toggle,
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
}
