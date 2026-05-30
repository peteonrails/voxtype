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

    /// Download the Silero VAD model for speech detection
    Vad {
        /// Show VAD model status
        #[arg(long)]
        status: bool,
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
