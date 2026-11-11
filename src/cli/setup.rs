//! `voxtype setup` subcommand actions and compositor variants.

use clap::Subcommand;

#[derive(Subcommand)]
pub enum SetupAction {
    /// Check system configuration and dependencies
    Check,

    /// Interactive macOS setup wizard
    #[cfg(target_os = "macos")]
    Macos,

    /// Install voxtype as a systemd user service (Linux)
    Systemd {
        /// Uninstall the service instead of installing
        #[arg(long)]
        uninstall: bool,

        /// Show service status
        #[arg(long)]
        status: bool,
    },

    /// Install voxtype as a LaunchAgent (macOS)
    /// Note: launchd services don't receive microphone permissions.
    /// Use 'app-bundle' instead for full functionality.
    #[cfg(target_os = "macos")]
    Launchd {
        /// Uninstall the service instead of installing
        #[arg(long)]
        uninstall: bool,

        /// Show service status
        #[arg(long)]
        status: bool,
    },

    /// Install Voxtype.app bundle with Login Items (macOS, recommended)
    /// Creates /Applications/Voxtype.app and adds to Login Items.
    /// This method properly receives Accessibility, Input Monitoring,
    /// and Microphone permissions (unlike launchd).
    #[cfg(target_os = "macos")]
    AppBundle {
        /// Uninstall the app bundle
        #[arg(long)]
        uninstall: bool,

        /// Show installation status
        #[arg(long)]
        status: bool,
    },

    /// Set up Hammerspoon hotkey integration (macOS)
    #[cfg(target_os = "macos")]
    Hammerspoon {
        /// Install Hammerspoon config (copy to ~/.hammerspoon/)
        #[arg(long)]
        install: bool,

        /// Show the Hammerspoon configuration snippet
        #[arg(long)]
        show: bool,

        /// Hotkey to configure (default: rightalt)
        #[arg(long, default_value = "rightalt")]
        hotkey: String,

        /// Use toggle mode instead of push-to-talk
        #[arg(long)]
        toggle: bool,
    },

    /// Install bundled Linux tray theme icons (embedded pixmaps work without this).
    #[cfg(target_os = "linux")]
    Icons {
        /// Remove the installed theme icons
        #[arg(long)]
        uninstall: bool,
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

    /// Link the packaged Voxtype Settings panel into Omarchy
    #[command(long_about = "\
        Link the packaged Voxtype Settings panel into Omarchy\n\n\
        Packages install the panel at \
        /usr/share/voxtype/omarchy-plugin/io.voxtype.settings. This command \
        links it into ~/.config/omarchy/plugins/io.voxtype.settings, asks \
        Omarchy's shell to rescan its plugins and enables the panel. The link \
        follows package upgrades: each release loads from its own directory, \
        so a rescan shows the new panel without restarting the shell.\n\n\
        The daemon does the same on its first start on Omarchy, and rescans \
        after an upgrade. Opt out with [omarchy] link_settings_plugin = false, \
        VOXTYPE_OMARCHY_LINK_SETTINGS_PLUGIN=false or \
        --omarchy-link-settings-plugin=false. Once the panel has been linked, \
        the daemon never adds it back after you remove it.\n\n\
        Your own copy (for example from `omarchy plugin add`) or a link to \
        somewhere else is never overwritten. --replace moves it to a hidden \
        backup beside it (.io.voxtype.settings.backup-<time>) and links the \
        packaged panel; it asks first in a terminal and needs --yes \
        otherwise. --yes alone never replaces a copy. --remove removes \
        voxtype's link only, never a copy.\n\n\
        --dismiss-copy answers \"Not now\" to the Settings panel's offer to \
        replace your copy: the panel stops showing it for that copy. \
        --replace, or removing the copy, clears it.\n\n\
        --status --json prints {omarchy, packaged_dir, packaged_version, \
        state, path, installed_version, copy_dismissed}, where state is one \
        of not-omarchy, no-package, absent, linked, linked-stale, copy or \
        foreign-link, and copy_dismissed is true only while a copy you \
        dismissed is present.\n\n\
        Exit codes: 0 on success or when there is nothing to do (no packaged \
        panel, or a copy left in place), 1 when Omarchy is not detected or an \
        action fails.\n\n\
        Examples:\n  \
        voxtype setup omarchy\n  \
        voxtype setup omarchy --status\n  \
        voxtype setup omarchy --status --json\n  \
        voxtype setup omarchy --replace\n  \
        voxtype setup omarchy --replace --yes --restart-shell\n  \
        voxtype setup omarchy --dismiss-copy\n  \
        voxtype setup omarchy --remove")]
    Omarchy {
        /// Show the link state instead of changing anything
        #[arg(long, conflicts_with_all = ["replace", "remove", "restart_shell", "yes"])]
        status: bool,

        /// With --status, print the state as JSON
        #[arg(long, requires = "status")]
        json: bool,

        /// Replace your own copy or link (backed up, never deleted)
        #[arg(long, conflicts_with = "remove")]
        replace: bool,

        /// Remove voxtype's link (never a copy)
        #[arg(long)]
        remove: bool,

        /// Run omarchy-restart-shell afterwards
        #[arg(long)]
        restart_shell: bool,

        /// Answer yes to prompts (required for --replace without a terminal)
        #[arg(long, short = 'y')]
        yes: bool,

        /// Stop offering to replace your copy ("Not now"); --replace clears it
        #[arg(long, conflicts_with_all = ["status", "replace", "remove", "restart_shell"])]
        dismiss_copy: bool,
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

    /// Manage GPU acceleration (Vulkan for Whisper, CUDA/MIGraphX for Parakeet)
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

    /// Manage Intel NPU acceleration through OpenVINO
    Npu {
        /// Enable NPU acceleration and configure OpenVINO
        #[arg(long)]
        enable: bool,

        /// Disable NPU acceleration and revert to Whisper
        #[arg(long)]
        disable: bool,

        /// Show NPU hardware and configuration status
        #[arg(long)]
        status: bool,
    },

    /// Switch the active binary variant (used by `voxtype configure` via pkexec)
    #[command(hide = true)]
    Variant {
        /// Variant binary name (e.g., voxtype-avx512, voxtype-onnx-cuda)
        #[arg(long, value_name = "NAME")]
        to: String,
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

    /// Show, accept or decline model upgrade offers (ONNX to GGUF, Parakeet Ultra)
    #[command(long_about = "\
        Show, accept or decline model upgrade offers\n\n\
        Voxtype offers two kinds of upgrade. A format update redownloads the \
        model you already use in the GGUF format, for better performance. \
        Parakeet v2 and v3 users are also offered Parakeet Ultra, a different \
        model covering the same 25 European languages as Parakeet v3. Nothing \
        changes until you accept an offer.\n\n\
        With no action, prints the offers for the configured model and, in \
        a terminal, asks which one to take. Models \
        that run on ONNX Runtime also get a notice: ONNX support is removed in \
        voxtype 2.0, and the notice gives the command that pins voxtype to \
        this version for your install method.\n\n\
        --accept downloads the model (showing its size and asking first \
        unless --yes is given), verifies its sha256, switches engine and model \
        in one validated write and restarts the daemon if it is running. Any \
        failure, including a cancelled download, leaves config.toml exactly \
        as it was and keeps the offer. The previous model's files are kept.\n\n\
        --decline records \"keep current model\" in \
        ~/.local/state/voxtype/model_upgrade.json, never in config.toml. \
        Declined offers stay listed, marked declined, so you can accept them \
        later.\n\n\
        --cleanup deletes the previous model's files from the models \
        directory, and only after voxtype has transcribed successfully with \
        the accepted model.\n\n\
        Exit codes: 0 on success, 2 for an unknown offer id, 1 for any other \
        failure.\n\n\
        Examples:\n  \
        voxtype setup migrate\n  \
        voxtype setup migrate --json\n  \
        voxtype setup migrate --accept parakeet-ultra\n  \
        voxtype setup migrate --accept format-gguf --yes --progress-format json\n  \
        voxtype setup migrate --decline\n  \
        voxtype setup migrate --decline parakeet-ultra\n  \
        voxtype setup migrate --cleanup")]
    Migrate {
        /// Print the upgrade status object as JSON (the same object as
        /// `voxtype status --format json --extended` reports as model_upgrade)
        #[arg(long)]
        json: bool,

        /// Accept an offer by id (parakeet-ultra or format-gguf)
        #[arg(long, value_name = "OFFER", conflicts_with_all = ["decline", "cleanup"])]
        accept: Option<String>,

        /// Keep the current model. With no OFFER, declines every current offer
        #[arg(
            long,
            value_name = "OFFER",
            num_args = 0..=1,
            default_missing_value = "",
            conflicts_with = "cleanup"
        )]
        decline: Option<String>,

        /// Delete the previous model's files after a successful switch
        #[arg(long)]
        cleanup: bool,

        /// Accept without asking for confirmation (required without a terminal)
        #[arg(long, short = 'y')]
        yes: bool,

        /// Download progress: human, or json for NDJSON events on stdout
        /// (the same events as `voxtype setup --download --progress-format json`)
        #[arg(
            long,
            value_name = "FORMAT",
            value_parser = clap::builder::PossibleValuesParser::new(super::PROGRESS_FORMATS),
        )]
        progress_format: Option<String>,
    },

    /// Install the voxtype-setup skill for your coding agents
    #[command(long_about = "\
        Install the voxtype-setup skill for your coding agents\n\n\
        The skill teaches a coding agent to configure Voxtype for this \
        machine: it inspects the hardware, asks what you dictate, and tunes \
        the engine, model, GPU acceleration, memory use and output settings, \
        asking before it changes anything.\n\n\
        With no options, installs the skill for every agent found: Claude Code \
        (~/.claude/skills) and the agents that read ~/.agents/skills (Codex, \
        Gemini CLI, OpenCode, Cursor, GitHub Copilot, Goose). An agent counts \
        as found when its config directory exists or its command is on PATH. \
        Use --target to install for one that was not found.\n\n\
        The skill is built into this voxtype binary, so the copy it writes \
        matches this version. Run the command again after upgrading voxtype: \
        --status and `voxtype setup check` report a copy that is out of date. \
        Run it as your own user, not with sudo.\n\n\
        A SKILL.md you have edited, or a different skill at the same path, \
        is never replaced or removed without asking. Without a terminal to \
        ask on, it is left alone; --force replaces it.\n\n\
        --remove deletes only the SKILL.md voxtype wrote, and keeps any other \
        files in that directory.\n\n\
        Examples:\n  \
        voxtype setup skill\n  \
        voxtype setup skill --target claude\n  \
        voxtype setup skill --status\n  \
        voxtype setup skill --status --json\n  \
        voxtype setup skill --remove")]
    Skill {
        /// Show where the skill is installed and whether it is current
        #[arg(long, conflicts_with = "remove")]
        status: bool,

        /// With --status, print the state as JSON
        #[arg(long, requires = "status")]
        json: bool,

        /// Remove the skill voxtype installed
        #[arg(long)]
        remove: bool,

        /// Replace or remove a copy you edited, or a different skill at the same path, without asking
        #[arg(long, conflicts_with = "status")]
        force: bool,

        /// Install for (or remove from) these agents instead of the ones found
        #[arg(
            long,
            value_name = "TARGET",
            value_parser = clap::builder::PossibleValuesParser::new(["claude", "agents", "all"]),
        )]
        target: Option<String>,
    },

    /// Download the Silero VAD model for speech detection
    Vad {
        /// Show VAD model status
        #[arg(long)]
        status: bool,
    },

    /// Install or inspect the optional English LARD disfluency model
    Lard {
        /// Install a local model with the pinned release checksum
        #[arg(long, conflicts_with = "url")]
        source: Option<std::path::PathBuf>,
        /// Explicit HTTPS model URL (publication is a separate release approval)
        #[arg(long)]
        url: Option<String>,
        /// Inspect the local model without downloading
        #[arg(long, conflicts_with_all = ["source", "url"])]
        status: bool,
    },

    /// Download the optional punctuation/casing model
    Punctuation {
        /// Show model status without downloading it
        #[arg(long)]
        status: bool,
    },

    /// Apply or list OSD recipe presets, or show which OSD frontend runs
    #[command(long_about = "\
        Apply or list OSD recipe presets, or show which OSD frontend runs\n\n\
        A recipe is a complete [osd] preset: layout, frame, and \
        [[osd.visual.layers]]. Applying one writes its keys into config.toml \
        through the same editor as `voxtype config set`: keys the recipe sets \
        replace the existing ones (never duplicated), [osd] keys it doesn't \
        set are kept, comments are preserved, and the result is validated \
        before it is written. Every changed key is printed with its previous \
        value.\n\n\
        Recipes are looked up in ~/.config/voxtype/osd-recipes, \
        ~/.local/share/voxtype/osd-recipes, then \
        /usr/share/voxtype/osd-recipes; a user copy shadows a shipped one. \
        With no --recipe name, lists the available recipes.\n\n\
        Frontend: [osd] frontend = \"auto\" (the default) runs the Quickshell \
        OSD when voxtype-osd-quickshell is installed, qs (or quickshell) is \
        on PATH and a Wayland session is present; otherwise gtk4 when \
        voxtype-osd-gtk4 is installed, its libraries (gtk4-layer-shell, GTK \
        4) load and a display is present; otherwise native. An explicit gtk4, native or quickshell is always kept.\n\n\
        --status shows the configured frontend, the one that runs and why. \
        --status --json prints {configured, active, reason, \
        quickshell_available, offer, offer_dismissed}, where configured is \
        auto, gtk4, native or quickshell, active is quickshell, gtk4, native \
        or none, and offer is true when an explicit gtk4 or native could \
        switch to the Quickshell OSD and the offer was not dismissed.\n\n\
        --use-auto sets osd.frontend = \"auto\" (validated, like `voxtype \
        config set`) and clears a dismissed offer. --dismiss-offer answers \
        \"Keep\" to the offer; it is stored in \
        ~/.local/state/voxtype/osd-offer.json, never in config.toml. Neither \
        restarts the daemon: restart voxtype to apply a frontend change.\n\n\
        Examples:\n  \
        voxtype setup osd --list\n  \
        voxtype setup osd --recipe showcase-bars --dry-run\n  \
        voxtype setup osd --recipe showcase-bars\n  \
        voxtype setup osd --recipe ./my-recipe.toml\n  \
        voxtype setup osd --status\n  \
        voxtype setup osd --status --json\n  \
        voxtype setup osd --use-auto && systemctl --user restart voxtype\n  \
        voxtype setup osd --dismiss-offer")]
    Osd {
        /// Recipe name from --list, or a path to a recipe .toml file
        #[arg(long, value_name = "NAME", num_args = 0..=1, default_missing_value = "")]
        recipe: Option<String>,

        /// List available recipes
        #[arg(long)]
        list: bool,

        /// Show what would change without writing config.toml
        #[arg(long)]
        dry_run: bool,

        /// Show the configured OSD frontend, the one that runs, and why
        #[arg(long, conflicts_with_all = ["recipe", "list", "dry_run", "use_auto", "dismiss_offer"])]
        status: bool,

        /// With --status, print the state as JSON
        #[arg(long, requires = "status")]
        json: bool,

        /// Set osd.frontend = "auto" and clear a dismissed Quickshell offer
        #[arg(long, conflicts_with_all = ["recipe", "list", "dry_run", "dismiss_offer"])]
        use_auto: bool,

        /// Dismiss the offer to switch to the Quickshell OSD
        #[arg(long, conflicts_with_all = ["recipe", "list", "dry_run"])]
        dismiss_offer: bool,
    },

    /// Install the Quickshell QML tree for the voxtype-osd-quickshell launcher
    ///
    /// Copies shell.qml, OsdSurface.qml, EnginePicker.qml,
    /// MeetingControls.qml, and the voxtype-shared module into
    /// $XDG_DATA_HOME/voxtype/quickshell/ (or ~/.local/share/voxtype/quickshell/
    /// if XDG_DATA_HOME is unset), then prints Hyprland/Sway/River
    /// keybinding examples for the Wave 2 engine-picker and meeting-controls
    /// trigger flags.
    Quickshell {
        /// Override the install target directory.
        #[arg(long, value_name = "DIR")]
        target: Option<std::path::PathBuf>,

        /// Override the QML source directory (otherwise auto-detected).
        ///
        /// Search order: $VOXTYPE_QUICKSHELL_SOURCE_DIR,
        /// <binary>/../share/voxtype/quickshell/, /usr/share/voxtype/quickshell/,
        /// ./quickshell/
        #[arg(long, value_name = "DIR")]
        source: Option<std::path::PathBuf>,

        /// Overwrite an existing install at the target.
        #[arg(long)]
        force: bool,

        /// Skip the file copy; only print the compositor binding examples.
        #[arg(long)]
        print_bindings: bool,

        /// Override the source path of the voxtype-audio-bridge binary.
        ///
        /// Search order (when omitted): $VOXTYPE_AUDIO_BRIDGE_BINARY,
        /// <binary>/../lib/voxtype/voxtype-audio-bridge,
        /// /usr/lib/voxtype/voxtype-audio-bridge, `which voxtype-audio-bridge`,
        /// target/release/voxtype-audio-bridge, target/debug/voxtype-audio-bridge.
        #[arg(long, value_name = "PATH")]
        bridge: Option<std::path::PathBuf>,

        /// Override the symlink location for voxtype-audio-bridge.
        ///
        /// Defaults to $XDG_BIN_HOME/voxtype-audio-bridge or
        /// ~/.local/bin/voxtype-audio-bridge. Must live under the user's
        /// $HOME unless you also pass --force.
        #[arg(long, value_name = "PATH")]
        bridge_target: Option<std::path::PathBuf>,

        /// Skip installing the voxtype-audio-bridge symlink.
        ///
        /// Use this if the bridge is already on PATH (e.g., a packaged
        /// install put it there, or you have your own symlink).
        #[arg(long)]
        skip_bridge: bool,
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
    use crate::cli::*;
    use clap::Parser;

    #[test]
    fn punctuation_setup_status_parses() {
        let cli = Cli::parse_from(["voxtype", "setup", "punctuation", "--status"]);
        match cli.command {
            Some(Commands::Setup {
                action: Some(SetupAction::Punctuation { status }),
                ..
            }) => assert!(status),
            _ => panic!("Expected setup punctuation --status"),
        }
    }

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
    fn test_setup_progress_format() {
        let cli = Cli::parse_from([
            "voxtype",
            "setup",
            "--download",
            "--model",
            "tiny.en",
            "--progress-format",
            "json",
        ]);
        match cli.command {
            Some(Commands::Setup {
                progress_format,
                download,
                model,
                ..
            }) => {
                assert_eq!(progress_format, "json");
                assert!(download);
                assert_eq!(model.as_deref(), Some("tiny.en"));
            }
            _ => panic!("Expected Setup command"),
        }

        let cli = Cli::parse_from(["voxtype", "setup"]);
        match cli.command {
            Some(Commands::Setup {
                progress_format, ..
            }) => assert_eq!(
                progress_format, "human",
                "human output must stay the default"
            ),
            _ => panic!("Expected Setup command"),
        }

        assert!(
            Cli::try_parse_from(["voxtype", "setup", "--progress-format", "ndjson"]).is_err(),
            "unknown formats should be rejected at parse time"
        );
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

    // =========================================================================
    // Omarchy setup tests
    // =========================================================================

    /// (status, json, replace, remove, restart_shell, yes)
    fn parse_omarchy(args: &[&str]) -> Result<(bool, bool, bool, bool, bool, bool), clap::Error> {
        let argv = ["voxtype", "setup", "omarchy"].iter().chain(args);
        match Cli::try_parse_from(argv)?.command {
            Some(Commands::Setup {
                action:
                    Some(SetupAction::Omarchy {
                        status,
                        json,
                        replace,
                        remove,
                        restart_shell,
                        yes,
                        ..
                    }),
                ..
            }) => Ok((status, json, replace, remove, restart_shell, yes)),
            _ => panic!("Expected Setup Omarchy command"),
        }
    }

    #[test]
    fn test_setup_omarchy_default_links() {
        assert_eq!(
            parse_omarchy(&[]).unwrap(),
            (false, false, false, false, false, false)
        );
    }

    #[test]
    fn test_setup_omarchy_status_json() {
        assert_eq!(
            parse_omarchy(&["--status", "--json"]).unwrap(),
            (true, true, false, false, false, false)
        );
        assert!(parse_omarchy(&["--status"]).unwrap().0);
    }

    #[test]
    fn test_setup_omarchy_replace_yes_restart_shell() {
        assert_eq!(
            parse_omarchy(&["--replace", "--yes", "--restart-shell"]).unwrap(),
            (false, false, true, false, true, true)
        );
        assert!(parse_omarchy(&["--replace", "-y"]).unwrap().5);
    }

    #[test]
    fn test_setup_omarchy_remove() {
        assert_eq!(
            parse_omarchy(&["--remove"]).unwrap(),
            (false, false, false, true, false, false)
        );
    }

    #[test]
    fn test_setup_omarchy_dismiss_copy() {
        let dismiss = |args: &[&str]| -> Result<bool, clap::Error> {
            let argv = ["voxtype", "setup", "omarchy"].iter().chain(args);
            match Cli::try_parse_from(argv)?.command {
                Some(Commands::Setup {
                    action: Some(SetupAction::Omarchy { dismiss_copy, .. }),
                    ..
                }) => Ok(dismiss_copy),
                _ => panic!("Expected Setup Omarchy command"),
            }
        };
        assert!(dismiss(&["--dismiss-copy"]).unwrap());
        assert!(!dismiss(&[]).unwrap());
        for other in ["--status", "--replace", "--remove", "--restart-shell"] {
            assert!(dismiss(&["--dismiss-copy", other]).is_err(), "{other}");
        }
    }

    #[test]
    fn test_setup_omarchy_rejects_conflicting_flags() {
        assert!(parse_omarchy(&["--json"]).is_err(), "--json needs --status");
        assert!(parse_omarchy(&["--replace", "--remove"]).is_err());
        assert!(parse_omarchy(&["--status", "--replace"]).is_err());
        assert!(parse_omarchy(&["--status", "--remove"]).is_err());
    }

    #[test]
    fn test_setup_omarchy_help_has_examples() {
        use clap::CommandFactory;
        let mut cmd = Cli::command();
        let setup = cmd.find_subcommand_mut("setup").unwrap();
        let omarchy = setup.find_subcommand_mut("omarchy").unwrap();
        let help = omarchy.render_long_help().to_string();
        assert!(
            help.contains("voxtype setup omarchy --status --json"),
            "{help}"
        );
        assert!(help.contains("link_settings_plugin"), "{help}");
        assert!(help.contains("--restart-shell"), "{help}");
        assert!(
            help.contains("voxtype setup omarchy --dismiss-copy"),
            "{help}"
        );
        assert!(help.contains("copy_dismissed"), "{help}");
    }

    #[test]
    fn test_omarchy_link_opt_out_flag() {
        let cli = Cli::parse_from(["voxtype", "--omarchy-link-settings-plugin=false", "daemon"]);
        assert_eq!(cli.omarchy_link_settings_plugin, Some(false));
        let cli = Cli::parse_from(["voxtype", "--omarchy-link-settings-plugin", "daemon"]);
        assert_eq!(cli.omarchy_link_settings_plugin, Some(true));
        let cli = Cli::parse_from(["voxtype", "daemon"]);
        assert_eq!(cli.omarchy_link_settings_plugin, None);
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
}
