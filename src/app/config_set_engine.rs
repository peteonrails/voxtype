//! `voxtype config set engine <NAME>` — small dispatcher over
//! `config_set::set_engine`.

use std::path::PathBuf;
use voxtype::{config, config_set};

/// Resolve the config file path the same way the daemon does — honoring
/// `--config <FILE>` first, then the existing user/system path, then the
/// XDG default. The default path is used even when nothing is on disk so
/// the file gets created in a predictable location on first write.
fn resolve_config_path_for_write(cli_override: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(p) = cli_override {
        return Ok(p);
    }
    if let Some(p) = config::Config::resolve_existing_path() {
        return Ok(p);
    }
    config::Config::default_path().ok_or_else(|| {
        anyhow::anyhow!(
            "Cannot determine config path. Set $XDG_CONFIG_HOME or $HOME, \
             or pass --config <FILE>."
        )
    })
}

/// Dispatcher for `voxtype config set engine <NAME>`. Exits the process
/// with code 2 on validation errors (bad name or missing feature) and code
/// 1 on filesystem failures, matching the contract documented in
/// `voxtype config set --help`.
pub(crate) fn run_config_set_engine(
    cli_override: Option<PathBuf>,
    name: &str,
) -> anyhow::Result<()> {
    let path = resolve_config_path_for_write(cli_override)?;
    match config_set::set_engine(path, name) {
        Ok(written) => {
            println!("Set engine = \"{}\" in {}", name, written.display());
            println!("Restart voxtype to apply: systemctl --user restart voxtype");
            Ok(())
        }
        Err(e @ config_set::ConfigSetError::UnknownEngine(_))
        | Err(e @ config_set::ConfigSetError::FeatureNotCompiled(_)) => {
            eprintln!("error: {}", e);
            std::process::exit(2);
        }
        Err(e @ config_set::ConfigSetError::Editor(_)) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}
