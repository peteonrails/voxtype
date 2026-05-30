//! `voxtype config` subcommand actions.

use clap::Subcommand;

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Modify a single configuration value in the on-disk config file
    ///
    /// Only `engine` is supported today. Comments and other fields are
    /// preserved. A restart of the voxtype daemon is required for the
    /// new value to take effect.
    Set {
        #[command(subcommand)]
        key: ConfigSetKey,
    },
}

#[derive(Subcommand)]
pub enum ConfigSetKey {
    /// Set the active transcription engine
    ///
    /// Valid engines: whisper, parakeet, moonshine, sensevoice, paraformer,
    /// dolphin, omnilingual, cohere. The engine must be compiled into this
    /// binary; check `voxtype info variants` if unsure.
    ///
    /// Examples:
    ///   voxtype config set engine whisper
    ///   voxtype config set engine parakeet
    Engine {
        /// Engine name (one of: whisper, parakeet, moonshine, sensevoice,
        /// paraformer, dolphin, omnilingual, cohere)
        #[arg(value_name = "NAME")]
        name: String,
    },
}
