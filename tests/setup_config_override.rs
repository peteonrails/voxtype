//! `--config` must redirect every config write, not just reads. `setup model
//! --set` used to edit the XDG config even when `--config` named another
//! file, which is how a test against a copy rewrote a real user config.

use std::process::Command;

#[test]
fn setup_model_set_writes_the_config_file_and_not_the_default_one() {
    let dir = tempfile::tempdir().unwrap();
    let xdg_config = dir.path().join("config");
    let xdg_data = dir.path().join("data");
    std::fs::create_dir_all(xdg_config.join("voxtype")).unwrap();
    std::fs::create_dir_all(xdg_data.join("voxtype/models")).unwrap();
    // `--set` for a Whisper model only checks that the file exists.
    std::fs::write(xdg_data.join("voxtype/models/ggml-base.en.bin"), b"").unwrap();

    let original = "engine = \"parakeet\"\n\n[parakeet]\nmodel = \"parakeet-tdt-0.6b-v2\"\nmodel_type = \"tdt\"\n";
    let default_file = xdg_config.join("voxtype/config.toml");
    let target = dir.path().join("target.toml");
    std::fs::write(&default_file, original).unwrap();
    std::fs::write(&target, original).unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_voxtype"))
        .env("XDG_CONFIG_HOME", &xdg_config)
        .env("XDG_DATA_HOME", &xdg_data)
        .arg("--config")
        .arg(&target)
        .args(["setup", "model", "--set", "base.en"])
        .status()
        .unwrap();
    assert!(status.success());

    assert_eq!(
        std::fs::read_to_string(&default_file).unwrap(),
        original,
        "the default config must be untouched"
    );
    let written: toml::Table = toml::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(written["engine"].as_str(), Some("whisper"));
    assert_eq!(written["whisper"]["model"].as_str(), Some("base.en"));
    assert_eq!(written["parakeet"]["model_type"].as_str(), Some("tdt"));
}
