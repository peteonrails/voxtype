//! `voxtype info` subcommand actions.

use clap::Subcommand;

#[derive(Subcommand)]
pub enum InfoAction {
    /// Show installed binary variants and which one is active
    Variants {
        /// Emit machine-readable JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },

    /// List audio capture devices
    ///
    /// The values accepted by `voxtype config set audio.device`. The
    /// synthetic `default` entry follows the system default at record time.
    Devices {
        /// Emit machine-readable JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },

    /// List downloadable models per engine and which are installed
    ///
    /// Every listing runs a cheap integrity check: file sizes against the
    /// manifest recorded at download time, and the ggml header for whisper
    /// models. A model that fails is reported as not installed.
    Models {
        /// Emit machine-readable JSON instead of human-readable text
        #[arg(long)]
        json: bool,

        /// Restrict output to one engine
        #[arg(long, value_name = "NAME")]
        engine: Option<String>,

        /// Also hash every file of every installed model against the manifest
        /// recorded at download time. Thorough and slow: it reads every byte
        /// of every model, which is minutes for a full models directory.
        #[arg(long)]
        verify: bool,
    },

    /// Report whether the running daemon is GPU-accelerated
    ///
    /// Answers from observation, not configuration: VRAM held by the daemon's
    /// process, the GPU markers in its journal, and the variant of the binary
    /// behind its PID. When the evidence doesn't decide, the state is
    /// `unknown` rather than a guess.
    Accel {
        /// Emit machine-readable JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },

    /// List transcription engines and which are compiled into this binary
    Engines {
        /// Emit machine-readable JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },

    /// List installed OSD styles (Quickshell frontend)
    ///
    /// The values `voxtype config set osd.style` accepts: the built-in
    /// default plus every style package found in ~/.config/voxtype/osd,
    /// ~/.local/share/voxtype/osd, and /usr/share/voxtype/osd. A user copy
    /// shadows a system package with the same name.
    Styles {
        /// Emit machine-readable JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
}
