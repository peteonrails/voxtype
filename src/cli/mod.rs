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
