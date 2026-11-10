//! Native Linux tray configuration defaults on while preserving explicit opt-out.
use std::process::Command;

#[test]
fn cli_env_file_precedence_and_comment_preserving_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "# Keep this comment\n[hotkey]\nkey = 'F13' # keep hotkey\n",
    )
    .unwrap();
    let run = |args: &[&str], env: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_voxtype"));
        command
            .arg("--config")
            .arg(&path)
            .args(args)
            .env_remove("VOXTYPE_TRAY_ENABLED");
        if let Some(value) = env {
            command.env("VOXTYPE_TRAY_ENABLED", value);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    assert_eq!(run(&["config", "get", "tray.enabled"], None).trim(), "true");
    assert_eq!(
        run(&["config", "get", "tray.enabled"], Some("true")).trim(),
        "true"
    );
    assert_eq!(
        run(
            &["--tray=false", "config", "get", "tray.enabled"],
            Some("true")
        )
        .trim(),
        "false"
    );
    assert_eq!(
        run(&["--tray", "config", "get", "tray.enabled"], Some("false")).trim(),
        "true"
    );
    assert_eq!(
        run(&["config", "get", "tray.enabled"], Some("false")).trim(),
        "false"
    );
    run(&["config", "set", "tray.enabled", "false"], None);
    assert_eq!(
        run(&["config", "get", "tray.enabled"], None).trim(),
        "false"
    );
    assert_eq!(
        run(&["config", "get", "tray.enabled"], Some("true")).trim(),
        "true"
    );
    assert_eq!(
        run(
            &["--tray=false", "config", "get", "tray.enabled"],
            Some("true")
        )
        .trim(),
        "false"
    );
    run(&["config", "set", "tray.enabled", "true"], None);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("# Keep this comment")
            && text.contains("# keep hotkey")
            && text.contains("F13")
    );
    assert_eq!(run(&["config", "get", "tray.enabled"], None).trim(), "true");
    run(&["config", "unset", "tray.enabled"], None);
    assert_eq!(run(&["config", "get", "tray.enabled"], None).trim(), "true");
}
#[test]
fn tray_defaults_enabled_for_omitted_empty_and_new_configs() {
    use voxtype::config::Config;
    assert!(Config::default().tray.enabled);
    for text in ["", "[tray]", "[tray]\nenabled = true"] {
        let config: Config = toml::from_str(text).unwrap();
        assert!(config.tray.enabled, "{text}");
    }
    let generated: Config = toml::from_str(&voxtype::config::default_config_content()).unwrap();
    assert!(generated.tray.enabled);
    let shipped: Config = toml::from_str(include_str!("../config/default.toml")).unwrap();
    assert!(shipped.tray.enabled);
    let disabled: Config = toml::from_str("[tray]\nenabled = false").unwrap();
    assert!(!disabled.tray.enabled);
}
