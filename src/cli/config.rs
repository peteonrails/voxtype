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
    #[command(long_about = format!(
        "Set the active transcription engine\n\n\
         Valid engines: {names}. The engine must be compiled into this binary; \
         check `voxtype info variants` if unsure.\n\n\
         Examples:\n  \
         voxtype config set engine whisper\n  \
         voxtype config set engine parakeet",
        names = super::ENGINE_NAMES_CSV,
    ))]
    Engine {
        /// Engine name
        #[arg(
            value_name = "NAME",
            long_help = format!("Engine name (one of: {})", super::ENGINE_NAMES_CSV),
        )]
        name: String,
    },
}
