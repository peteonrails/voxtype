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
}
