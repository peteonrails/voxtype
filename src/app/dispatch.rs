//! Top-level `match cli.command` dispatch. Each arm calls into a sibling
//! submodule (or directly into a library crate) — the body of every long
//! subcommand handler lives in its own file.

use std::path::PathBuf;
#[cfg(target_os = "macos")]
use voxtype::menubar;
use voxtype::{
    config, daemon, setup, transcribe, Cli, Commands, ConfigAction, ConfigSetKey, SetupAction,
};

use super::config_set_engine::run_config_set_engine;
use super::config_show::show_config;
use super::info::run_info_command;
use super::meeting::run_meeting_command;
use super::record::send_record_command;
use super::status::run_status;
use super::transcribe_file::transcribe_file;
use super::updates::check_for_updates;

/// Check if running as root and warn for commands that don't need elevated privileges.
/// Returns true if running as root.
fn warn_if_root(command_name: &str) -> bool {
    // SAFETY: getuid() is always safe to call
    let is_root = unsafe { libc::getuid() } == 0;
    if is_root {
        eprintln!(
            "Warning: Running 'voxtype setup {}' as root is not recommended.",
            command_name
        );
        eprintln!("  - Models will download to /root/.local/share/voxtype/ instead of your user directory");
        eprintln!(
            "  - Config changes will apply to /root/.config/voxtype/ instead of your user config"
        );
        eprintln!("  - Cannot restart your user's voxtype daemon from root");
        eprintln!();
        eprintln!("Run without sudo: voxtype setup {}", command_name);
        eprintln!();
    }
    is_root
}

/// Dispatch the parsed subcommand. `top_level_model` is the clone of
/// `cli.model` captured before `apply_cli_overrides` consumed any of the
/// flags; `send_record_command` needs it so a subcommand-level `--model`
/// can still defer to the global flag.
pub(crate) async fn dispatch(
    cli: Cli,
    config_path: Option<PathBuf>,
    mut config: config::Config,
    top_level_model: Option<String>,
) -> anyhow::Result<()> {
    // On macOS, detect if launched as app bundle executable (no subcommand, binary inside .app)
    #[cfg(target_os = "macos")]
    let default_command = if cli.command.is_none() {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(|s| s.contains(".app/Contents/MacOS/")))
            .unwrap_or(false)
            .then_some(Commands::AppLaunch)
            .unwrap_or(Commands::Daemon)
    } else {
        Commands::Daemon // unused, cli.command is Some
    };
    #[cfg(not(target_os = "macos"))]
    let default_command = Commands::Daemon;

    // Run the appropriate command
    match cli.command.unwrap_or(default_command) {
        Commands::Daemon => {
            let mut daemon = daemon::Daemon::new(config, config_path);
            daemon.run().await?;
        }
        #[cfg(target_os = "macos")]
        Commands::Menubar => {
            let state_file = config
                .resolve_state_file()
                .ok_or_else(|| anyhow::anyhow!("state_file not configured"))?;
            menubar::run(state_file);
            // Note: menubar::run() never returns (runs macOS event loop)
        }
        #[cfg(target_os = "macos")]
        Commands::AppLaunch => {
            // Launched by Voxtype.app: start daemon in background, run menubar in foreground.
            // The binary must be the CFBundleExecutable (not exec'd from a wrapper script)
            // so macOS Control Center can register the status bar scene correctly.
            let logs_dir = dirs::home_dir()
                .map(|h| h.join("Library/Logs/voxtype"))
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp/voxtype"));
            let _ = std::fs::create_dir_all(&logs_dir);

            // First-launch auto-setup: create config and download model if needed
            super::macos::first_launch_setup(&config).await;

            // Kill any existing instances
            let _ = std::process::Command::new("pkill")
                .args(["-9", "-f", "voxtype-bin daemon"])
                .status();
            let _ = std::process::Command::new("pkill")
                .args(["-9", "-f", "voxtype-bin menubar"])
                .status();
            let _ = std::fs::remove_file("/tmp/voxtype/voxtype.lock");
            let _ = std::fs::remove_file("/tmp/voxtype/menubar.lock");

            // Start daemon as a child process with logging
            let exe = std::env::current_exe()?;
            let stdout = std::fs::File::create(logs_dir.join("stdout.log"))?;
            let stderr = std::fs::File::create(logs_dir.join("stderr.log"))?;
            let _daemon = std::process::Command::new(&exe)
                .arg("daemon")
                .stdout(stdout)
                .stderr(stderr)
                .spawn()?;

            // Run menubar in this process (keeps the app alive with menu bar icon)
            let state_file = config
                .resolve_state_file()
                .ok_or_else(|| anyhow::anyhow!("state_file not configured"))?;
            menubar::run(state_file);
        }

        Commands::Transcribe { file, engine } => {
            if let Some(engine_name) = engine {
                match engine_name.to_lowercase().as_str() {
                    "whisper" => config.engine = config::TranscriptionEngine::Whisper,
                    "parakeet" => config.engine = config::TranscriptionEngine::Parakeet,
                    "moonshine" => config.engine = config::TranscriptionEngine::Moonshine,
                    "sensevoice" => config.engine = config::TranscriptionEngine::SenseVoice,
                    "paraformer" => config.engine = config::TranscriptionEngine::Paraformer,
                    "dolphin" => config.engine = config::TranscriptionEngine::Dolphin,
                    "omnilingual" => config.engine = config::TranscriptionEngine::Omnilingual,
                    "cohere" => config.engine = config::TranscriptionEngine::Cohere,
                    "soniox" => config.engine = config::TranscriptionEngine::Soniox,
                    _ => {
                        eprintln!("Error: Invalid engine '{}'. Valid options: whisper, parakeet, moonshine, sensevoice, paraformer, dolphin, omnilingual, cohere, soniox", engine_name);
                        std::process::exit(1);
                    }
                }
            }
            transcribe_file(&config, &file)?;
        }

        Commands::TranscribeWorker {
            model,
            language,
            translate,
            threads,
        } => {
            // Internal command: run transcription worker process
            // This is spawned by the daemon when gpu_isolation is enabled
            // Use command-line overrides if provided, otherwise use config
            let mut whisper_config = config.whisper.clone();
            if let Some(m) = model {
                whisper_config.model = m;
            }
            if let Some(l) = language {
                // Parse comma-separated language string back to LanguageConfig
                whisper_config.language = config::LanguageConfig::from_comma_separated(&l);
            }
            if translate {
                whisper_config.translate = true;
            }
            if let Some(t) = threads {
                whisper_config.threads = Some(t);
            }
            transcribe::worker::run_worker(&whisper_config)?;
        }

        Commands::Setup {
            action,
            download,
            model,
            quiet,
            no_post_install,
        } => {
            match action {
                Some(SetupAction::Check) => {
                    warn_if_root("check");
                    setup::run_checks(&config).await?;
                }
                Some(SetupAction::Systemd { uninstall, status }) => {
                    warn_if_root("systemd");
                    if status {
                        setup::systemd::status().await?;
                    } else if uninstall {
                        setup::systemd::uninstall().await?;
                    } else {
                        setup::systemd::install().await?;
                    }
                }
                #[cfg(target_os = "macos")]
                Some(SetupAction::Launchd { uninstall, status }) => {
                    if status {
                        setup::launchd::status().await?;
                    } else if uninstall {
                        setup::launchd::uninstall().await?;
                    } else {
                        setup::launchd::install().await?;
                    }
                }
                #[cfg(target_os = "macos")]
                Some(SetupAction::AppBundle { uninstall, status }) => {
                    if status {
                        setup::app_bundle::status().await?;
                    } else if uninstall {
                        setup::app_bundle::uninstall().await?;
                    } else {
                        setup::app_bundle::install().await?;
                    }
                }
                #[cfg(target_os = "macos")]
                Some(SetupAction::Hammerspoon {
                    install,
                    show,
                    hotkey,
                    toggle,
                }) => {
                    setup::hammerspoon::run(install, show, &hotkey, toggle).await?;
                }
                #[cfg(target_os = "macos")]
                Some(SetupAction::Macos) => {
                    setup::macos::run().await?;
                }
                Some(SetupAction::Waybar {
                    json,
                    css,
                    install,
                    uninstall,
                }) => {
                    warn_if_root("waybar");
                    if install {
                        setup::waybar::install()?;
                    } else if uninstall {
                        setup::waybar::uninstall()?;
                    } else if json {
                        println!("{}", setup::waybar::get_json_config());
                    } else if css {
                        println!("{}", setup::waybar::get_css_config());
                    } else {
                        setup::waybar::print_config();
                    }
                }
                Some(SetupAction::Dms {
                    install,
                    uninstall,
                    qml,
                }) => {
                    warn_if_root("dms");
                    if install {
                        setup::dms::install()?;
                    } else if uninstall {
                        setup::dms::uninstall()?;
                    } else if qml {
                        println!("{}", setup::dms::get_qml_config());
                    } else {
                        setup::dms::print_config();
                    }
                }
                Some(SetupAction::Model { list, set, restart }) => {
                    warn_if_root("model");
                    if list {
                        setup::model::list_installed();
                    } else if let Some(model_name) = set {
                        setup::model::set_model(&model_name, restart).await?;
                    } else {
                        setup::model::interactive_select().await?;
                    }
                }
                Some(SetupAction::Gpu {
                    enable,
                    disable,
                    status,
                }) => {
                    if status {
                        setup::gpu::show_status();
                    } else if enable {
                        setup::gpu::enable()?;
                    } else if disable {
                        setup::gpu::disable()?;
                    } else {
                        // Default: show status
                        setup::gpu::show_status();
                    }
                }
                Some(SetupAction::Variant { to }) => {
                    let variant =
                        setup::binary::Variant::from_binary_name(&to).ok_or_else(|| {
                            anyhow::anyhow!(
                                "Unknown variant '{}'. Expected one of: {}",
                                to,
                                setup::binary::Variant::ALL
                                    .iter()
                                    .map(|v| v.binary_name())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        })?;
                    setup::binary::switch_to(variant)?;
                    println!("Switched /usr/bin/voxtype to {}.", variant.binary_name());
                }
                Some(SetupAction::Onnx {
                    enable,
                    disable,
                    status,
                })
                | Some(SetupAction::Parakeet {
                    enable,
                    disable,
                    status,
                }) => {
                    warn_if_root("onnx");
                    if status {
                        setup::parakeet::show_status();
                    } else if enable {
                        setup::parakeet::enable()?;
                    } else if disable {
                        setup::parakeet::disable()?;
                    } else {
                        // Default: show status
                        setup::parakeet::show_status();
                    }
                }
                Some(SetupAction::Compositor { compositor_type }) => {
                    warn_if_root("compositor");
                    setup::compositor::run(&compositor_type).await?;
                }
                Some(SetupAction::Vad { status }) => {
                    warn_if_root("vad");
                    if status {
                        setup::vad::show_status();
                    } else {
                        setup::vad::download_model()?;
                    }
                }
                Some(SetupAction::Quickshell {
                    target,
                    source,
                    force,
                    print_bindings,
                    bridge,
                    bridge_target,
                    skip_bridge,
                }) => {
                    warn_if_root("quickshell");
                    setup::quickshell::run(
                        target,
                        source,
                        force,
                        print_bindings,
                        bridge,
                        bridge_target,
                        skip_bridge,
                    )?;
                }
                None => {
                    // Default: run setup (non-blocking)
                    warn_if_root("");
                    setup::run_setup(&config, download, model.as_deref(), quiet, no_post_install)
                        .await?;
                }
            }
        }

        Commands::Config { action } => match action {
            None => show_config(&config).await?,
            Some(ConfigAction::Set { key }) => match key {
                ConfigSetKey::Engine { name } => {
                    run_config_set_engine(cli.config.clone(), &name)?;
                }
            },
        },

        Commands::Info { action } => {
            run_info_command(action)?;
        }

        Commands::Configure { force_package_mode } => {
            voxtype::tui::run(force_package_mode)?;
        }

        Commands::Status {
            follow,
            format,
            extended,
            icon_theme,
        } => {
            run_status(&config, follow, &format, extended, icon_theme).await?;
        }

        Commands::Record { action } => {
            send_record_command(&config, action, top_level_model.as_deref())?;
        }

        Commands::Meeting { action } => {
            run_meeting_command(&config, action).await?;
        }

        Commands::CheckUpdate => {
            check_for_updates().await?;
        }
    }

    Ok(())
}
