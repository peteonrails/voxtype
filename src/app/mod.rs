//! Binary-side entry points. `src/main.rs` is a thin shim: it installs signal
//! handlers, sets up logging, parses CLI, loads config, then hands off to
//! `app::run(cli, config_path, config).await`.
//!
//! The rest of this module is organised by subcommand. Each long handler
//! lives in its own file (`record.rs`, `status.rs`, `meeting.rs`, …); shared
//! plumbing lives in `daemon_pid.rs` and `dispatch.rs`.

use std::path::PathBuf;
use voxtype::{config, Cli};

mod config_set_engine;
mod config_show;
mod daemon_pid;
mod dispatch;
mod info;
#[cfg(target_os = "macos")]
mod macos;
mod meeting;
mod overrides;
mod record;
pub(crate) mod sigpipe;
mod status;
mod transcribe_file;
mod updates;

/// Apply CLI overrides to `config`, then dispatch the subcommand.
///
/// `main` keeps the bare minimum that can't be moved out of the binary
/// entry point: tokio runtime attribute, SIGILL handler, SIGPIPE reset,
/// log init, `Cli::parse()`, and the initial config load. Everything
/// downstream of that flows through here.
pub(crate) async fn run(
    cli: Cli,
    config_path: Option<PathBuf>,
    mut config: config::Config,
) -> anyhow::Result<()> {
    let top_level_model = overrides::apply_cli_overrides(&mut config, &cli);
    dispatch::dispatch(cli, config_path, config, top_level_model).await
}
