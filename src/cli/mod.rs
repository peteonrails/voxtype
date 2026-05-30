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

/// Comma-separated list of every transcription engine name as it appears in
/// CLI help text.
///
/// This constant lives in the CLI module — rather than being derived from
/// `crate::config::TranscriptionEngine::names_csv()` at use sites — because
/// `build.rs` includes this module via `#[path = "src/cli/mod.rs"] mod cli;`
/// for man-page generation and cannot reach into `crate::config` from that
/// context. The constant is pinned to the enum by a test in
/// `src/config/engines/mod.rs` so a new engine variant forces this string
/// to update or the build breaks.
pub(crate) const ENGINE_NAMES_CSV: &str =
    "whisper, parakeet, moonshine, sensevoice, paraformer, dolphin, omnilingual, cohere, soniox";

/// Diarization backends the daemon dispatches on. Used by the CLI's
/// `value_parser` for `--diarization` so unknown values are rejected at
/// parse time instead of falling through the runtime match arm.
///
/// The authoritative dispatch lives in `src/meeting/diarization/mod.rs`'s
/// `match backend.as_str()` block; a test in `src/config/meeting.rs` pins
/// this list against those arms.
pub(crate) const DIARIZATION_BACKENDS: &[&str] = &["simple", "ml"];
