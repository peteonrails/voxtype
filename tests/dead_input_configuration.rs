use clap::Parser;
use std::path::Path;
use std::process::Command;

use voxtype::config::Config;
use voxtype::Cli;

fn get(path: &Path, key: &str, env: Option<(&str, &str)>, flags: &[&str]) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_voxtype"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("VOXTYPE_") {
            command.env_remove(name);
        }
    }
    if let Some((name, value)) = env {
        command.env(name, value);
    }
    let output = command
        .arg("--config")
        .arg(path)
        .args(flags)
        .args(["config", "get", key])
        .output()
        .expect("config get should start");
    assert!(
        output.status.success(),
        "{}: {}",
        key,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("config get output should be UTF-8")
        .trim()
        .to_owned()
}

#[test]
fn absent_dead_input_table_keeps_the_guard_enabled() {
    let config: Config = toml::from_str("").expect("empty configuration should load");

    assert!(config.audio.dead_input.enabled);
    assert_eq!(config.audio.dead_input.window_ms, 1_500);
    assert_eq!(config.audio.dead_input.threshold_dbfs, -100.0);
}

#[test]
fn partial_audio_config_keeps_dead_input_defaults() {
    let config: Config = toml::from_str(
        r#"
            [audio]
            device = "default"

            [audio.dead_input]
            window_ms = 900
        "#,
    )
    .expect("an existing partial audio config should load");

    assert!(config.audio.dead_input.enabled);
    assert_eq!(config.audio.dead_input.window_ms, 900);
    assert_eq!(config.audio.dead_input.threshold_dbfs, -100.0);
}

#[test]
fn dead_input_cli_options_parse_together() {
    let cli = Cli::try_parse_from([
        "voxtype",
        "--no-dead-input-check",
        "--dead-input-window",
        "900",
        "--dead-input-threshold",
        "-80",
    ])
    .expect("dead-input options should parse");

    assert!(cli.no_dead_input_check);
    assert_eq!(cli.dead_input_window, Some(900));
    assert_eq!(cli.dead_input_threshold, Some(-80.0));
}

#[test]
fn dead_input_file_environment_and_cli_overrides_are_ordered() {
    let dir = tempfile::tempdir().expect("temporary config directory");
    let path = dir.path().join("config.toml");
    let source = r#"
        [audio.dead_input]
        enabled = false
        window_ms = 900
        threshold_dbfs = -90.0
    "#;
    std::fs::write(&path, source).expect("write config");

    assert_eq!(get(&path, "audio.dead_input.enabled", None, &[]), "false");
    assert_eq!(
        get(
            &path,
            "audio.dead_input.enabled",
            Some(("VOXTYPE_DEAD_INPUT_DETECTION", "true")),
            &[]
        ),
        "true"
    );
    assert_eq!(
        get(
            &path,
            "audio.dead_input.enabled",
            Some(("VOXTYPE_DEAD_INPUT_DETECTION", "true")),
            &["--no-dead-input-check"]
        ),
        "false"
    );
    assert_eq!(
        get(
            &path,
            "audio.dead_input.enabled",
            Some(("VOXTYPE_DEAD_INPUT_DETECTION", "false")),
            &["--dead-input-check"]
        ),
        "true"
    );
    assert_eq!(
        get(
            &path,
            "audio.dead_input.window_ms",
            Some(("VOXTYPE_DEAD_INPUT_WINDOW_MS", "1000")),
            &[]
        ),
        "1000"
    );
    assert_eq!(
        get(
            &path,
            "audio.dead_input.window_ms",
            Some(("VOXTYPE_DEAD_INPUT_WINDOW_MS", "1000")),
            &["--dead-input-window", "1100"]
        ),
        "1100"
    );
    assert_eq!(
        get(
            &path,
            "audio.dead_input.threshold_dbfs",
            Some(("VOXTYPE_DEAD_INPUT_THRESHOLD_DBFS", "-85")),
            &[]
        ),
        "-85.0"
    );
    assert_eq!(
        get(
            &path,
            "audio.dead_input.threshold_dbfs",
            Some(("VOXTYPE_DEAD_INPUT_THRESHOLD_DBFS", "-85")),
            &["--dead-input-threshold", "-80"]
        ),
        "-80.0"
    );
    assert_eq!(std::fs::read_to_string(&path).expect("read config"), source);
}

#[test]
fn dead_input_enable_flags_conflict() {
    assert!(
        Cli::try_parse_from(["voxtype", "--dead-input-check", "--no-dead-input-check",]).is_err()
    );
}
