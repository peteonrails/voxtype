// Command-line interface definitions for voxtype
//
// This module tree is shared between the binary (main.rs) and build.rs
// (for man-page generation). Each subcommand enum lives in its own file
// under `src/cli/`; this module re-exports them so external code can
// continue to import `crate::cli::Cli` etc. unchanged.

mod commands;
mod config;
mod history;
mod info;
mod meeting;
mod record;
mod root;
mod setup;

pub use commands::Commands;
pub use config::ConfigAction;
pub use history::{HistoryAction, HISTORY_AFTER_HELP};
pub use info::InfoAction;
pub use meeting::{
    AssistantAction, CalendarAction, MeetingAction, TodoistAction, CALENDAR_LINK_HELP,
};
pub use record::{OutputModeOverride, RecordAction};
pub use root::Cli;
pub use setup::{CompositorType, SetupAction};

/// The version string every user-facing surface prints.
///
/// `build.rs` stamps `VOXTYPE_BUILD_VERSION` with the crate version when HEAD
/// sits exactly on the matching release tag, and `{version}+g{sha}` for any
/// other commit, so a build made between the Cargo.toml bump and the tag no
/// longer claims to be the release (see `expose_build_version` in build.rs
/// for the incident behind this). When the env var is absent — builds without
/// git such as the AUR source tarball, and this module's inclusion in the
/// build script itself — this degrades to the bare crate version.
///
/// This constant lives in the CLI module for the same reason as
/// `ENGINE_NAMES_CSV` below: `build.rs` includes this module standalone for
/// man-page generation and cannot reach into the rest of the crate.
pub const VERSION: &str = match option_env!("VOXTYPE_BUILD_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

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
pub const ENGINE_NAMES_CSV: &str =
    "whisper, parakeet, moonshine, sensevoice, paraformer, dolphin, omnilingual, cohere, gguf, openvino, soniox, elevenlabs";

/// Diarization backends the daemon dispatches on. Used by the CLI's
/// `value_parser` for `--diarization` so unknown values are rejected at
/// parse time instead of falling through the runtime match arm.
///
/// The authoritative dispatch lives in `src/meeting/diarization/mod.rs`'s
/// `match backend.as_str()` block; a test in `src/config/meeting.rs` pins
/// this list against those arms.
pub(crate) const DIARIZATION_BACKENDS: &[&str] = &["simple", "ml", "hybrid"];

/// Voices of the built-in recording announcement, for `meeting start
/// --announcement-voice`. `crate::meeting::announce::VOICES` is this list;
/// it lives here because `build.rs` includes this module standalone.
pub(crate) const ANNOUNCEMENT_VOICES: &[&str] = &["kokoro", "jessica"];

/// Parse `meeting start --announcement-volume`: a number from 0.0 to 1.0,
/// refused with the range otherwise. `crate::meeting::announce::parse_volume`
/// is this function; it lives here because `build.rs` includes this module
/// standalone.
pub(crate) fn parse_announcement_volume(raw: &str) -> Result<f32, String> {
    let volume: f32 = raw
        .trim()
        .parse()
        .map_err(|_| format!("{:?} is not a number; give a volume from 0.0 to 1.0", raw))?;
    if !(0.0..=1.0).contains(&volume) {
        return Err(format!(
            "{} is out of range; give a volume from 0.0 (silent) to 1.0 (full level)",
            raw.trim()
        ));
    }
    Ok(volume)
}

/// Formats `voxtype meeting export --format` accepts, each with its short
/// alias. Kept here rather than derived from
/// `crate::meeting::export::ExportFormat` because `build.rs` includes this
/// module standalone for man pages; a test in `src/meeting/export/mod.rs`
/// pins every name and alias against `ExportFormat::parse`.
pub(crate) const EXPORT_FORMATS: &[(&str, Option<&str>)] = &[
    ("markdown", Some("md")),
    ("text", Some("txt")),
    ("json", None),
    ("srt", None),
    ("vtt", None),
];

/// `value_parser` for `meeting export --format`: an unknown format is
/// refused at parse time with the list of valid ones.
pub(crate) fn export_format_parser() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(EXPORT_FORMATS.iter().map(|&(name, alias)| {
        let value = clap::builder::PossibleValue::new(name);
        match alias {
            Some(alias) => value.alias(alias),
            None => value,
        }
    }))
}

/// Values `voxtype setup --progress-format` accepts. `human` is curl's
/// progress bar and the usual status lines; `json` emits one NDJSON event per
/// update on stdout for a GUI to render.
///
/// Declared here rather than derived from
/// `crate::setup::progress::ProgressFormat` because `build.rs` includes this
/// module standalone for man-page generation and can't reach into the rest of
/// the crate. A test in `src/setup/progress.rs` pins the two together.
pub const PROGRESS_FORMATS: &[&str] = &["human", "json"];
