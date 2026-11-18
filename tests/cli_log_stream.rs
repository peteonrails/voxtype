//! A command that prints data keeps its log off stdout.
//!
//! The log used to go to stdout unless a command was on a short list, so a
//! warning from loading the config came out of `voxtype config get` ahead
//! of the value, and a script reading stdout got the warning. These run the
//! real binary with a config whose `[audio]` section cannot be read, which
//! makes the loader warn on every command.

use std::path::Path;
use std::process::{Command, Output};

const WARNING: &str = "Config section 'audio' could not be read";

fn run(dir: &Path, args: &[&str]) -> Output {
    let config = dir.join("config.toml");
    std::fs::write(&config, "[audio]\nsample_rate = \"not-a-number\"\n").unwrap();
    Command::new(env!("CARGO_BIN_EXE_voxtype"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .env_remove("RUST_LOG")
        .arg("--config")
        .arg(&config)
        .args(args)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn config_get_prints_only_the_value_when_the_config_loader_warns() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(dir.path(), &["config", "get", "engine"]);

    assert!(out.status.success(), "stderr: {}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "whisper\n");
    assert!(
        text(&out.stderr).contains(WARNING),
        "the warning should still be shown, on stderr: {}",
        text(&out.stderr)
    );
}

#[test]
fn config_get_json_stays_parseable_at_the_most_verbose_log_level() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(dir.path(), &["-vv", "config", "get", "engine", "--json"]);

    assert!(out.status.success(), "stderr: {}", text(&out.stderr));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", text(&out.stdout)));
    assert_eq!(value["value"], "whisper");
}

/// The other commands share the logger, so the same warning reached their
/// output too. `config schema --json` is one a frontend parses.
#[test]
fn config_schema_json_stays_parseable_when_the_config_loader_warns() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(dir.path(), &["config", "schema", "--json"]);

    assert!(out.status.success(), "stderr: {}", text(&out.stderr));
    assert!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).is_ok(),
        "stdout is not JSON: {}",
        text(&out.stdout).chars().take(300).collect::<String>()
    );
    assert!(text(&out.stderr).contains(WARNING));
}
