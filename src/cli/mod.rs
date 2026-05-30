// Command-line interface definitions for voxtype
//
// This module tree is shared between the binary (main.rs) and build.rs
// (for man-page generation). Each subcommand enum lives in its own file
// under `src/cli/`; this module re-exports them so external code can
// continue to import `crate::cli::Cli` etc. unchanged.

mod commands;
mod config;
mod info;
mod meeting;
mod record;
mod root;
mod setup;

pub use commands::Commands;
pub use config::{ConfigAction, ConfigSetKey};
pub use info::InfoAction;
pub use meeting::MeetingAction;
pub use record::{OutputModeOverride, RecordAction};
pub use root::Cli;
pub use setup::{CompositorType, SetupAction};

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

    // =========================================================================
    // Smart auto-submit flag tests
    // =========================================================================

    #[test]
    fn test_record_start_smart_auto_submit_enable() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--smart-auto-submit"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.smart_auto_submit_override(), Some(true));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_no_smart_auto_submit() {
        let cli = Cli::parse_from(["voxtype", "record", "start", "--no-smart-auto-submit"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.smart_auto_submit_override(), Some(false));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_start_smart_auto_submit_mutual_exclusion() {
        let result = Cli::try_parse_from([
            "voxtype",
            "record",
            "start",
            "--smart-auto-submit",
            "--no-smart-auto-submit",
        ]);
        assert!(
            result.is_err(),
            "Should not allow both flags simultaneously"
        );
    }

    #[test]
    fn test_record_start_smart_auto_submit_no_flags_returns_none() {
        let cli = Cli::parse_from(["voxtype", "record", "start"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.smart_auto_submit_override(), None);
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_toggle_smart_auto_submit_enable() {
        let cli = Cli::parse_from(["voxtype", "record", "toggle", "--smart-auto-submit"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.smart_auto_submit_override(), Some(true));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_toggle_no_smart_auto_submit() {
        let cli = Cli::parse_from(["voxtype", "record", "toggle", "--no-smart-auto-submit"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.smart_auto_submit_override(), Some(false));
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_record_stop_has_no_smart_auto_submit_override() {
        let cli = Cli::parse_from(["voxtype", "record", "stop"]);
        match cli.command {
            Some(Commands::Record { action }) => {
                assert_eq!(action.smart_auto_submit_override(), None);
            }
            _ => panic!("Expected Record command"),
        }
    }

    #[test]
    fn test_meeting_start_diarization_simple_flag() {
        let cli = Cli::parse_from(["voxtype", "meeting", "start", "--diarization", "simple"]);
        match cli.command {
            Some(Commands::Meeting {
                action: MeetingAction::Start { diarization, .. },
            }) => {
                assert_eq!(diarization.as_deref(), Some("simple"));
            }
            _ => panic!("Expected Meeting Start command"),
        }
    }

    #[test]
    fn test_meeting_start_diarization_ml_flag() {
        let cli = Cli::parse_from([
            "voxtype",
            "meeting",
            "start",
            "--diarization",
            "ml",
            "--title",
            "standup",
        ]);
        match cli.command {
            Some(Commands::Meeting {
                action: MeetingAction::Start { diarization, title },
            }) => {
                assert_eq!(diarization.as_deref(), Some("ml"));
                assert_eq!(title.as_deref(), Some("standup"));
            }
            _ => panic!("Expected Meeting Start command"),
        }
    }

    #[test]
    fn test_meeting_start_diarization_rejects_invalid() {
        let result = Cli::try_parse_from(["voxtype", "meeting", "start", "--diarization", "bogus"]);
        assert!(
            result.is_err(),
            "clap should reject diarization values outside [\"simple\", \"ml\"]"
        );
    }

    /// Env-var wiring is exercised together with the "no override" case in a
    /// single test to avoid `VOXTYPE_MEETING_DIARIZATION` leaking between
    /// tests that run in parallel — env vars are process-global, so two
    /// independent #[test] functions would race.
    #[test]
    fn test_meeting_start_diarization_env_and_default() {
        // Make sure no stale value is set from the host or a sibling test.
        std::env::remove_var("VOXTYPE_MEETING_DIARIZATION");

        // No flag, no env var → no override.
        let cli = Cli::parse_from(["voxtype", "meeting", "start"]);
        match cli.command {
            Some(Commands::Meeting {
                action: MeetingAction::Start { diarization, title },
            }) => {
                assert_eq!(diarization, None);
                assert_eq!(title, None);
            }
            _ => panic!("Expected Meeting Start command"),
        }

        // Env var alone should be picked up by clap's #[arg(env = ...)].
        std::env::set_var("VOXTYPE_MEETING_DIARIZATION", "ml");
        let cli = Cli::parse_from(["voxtype", "meeting", "start"]);
        std::env::remove_var("VOXTYPE_MEETING_DIARIZATION");
        match cli.command {
            Some(Commands::Meeting {
                action: MeetingAction::Start { diarization, .. },
            }) => {
                assert_eq!(diarization.as_deref(), Some("ml"));
            }
            _ => panic!("Expected Meeting Start command"),
        }
    }
}
