//! Regression coverage for independent RC2 review findings.
use voxtype::config::{Config, TranscriptionEngine};

#[test]
fn curated_runtime_overrides_seed_architecture_and_compatible_language() {
    let base = Config::default();
    let qwen = voxtype::model_catalog::override_config(
        &base,
        TranscriptionEngine::Gguf,
        "Qwen3-ASR-0.6B-Q8_0.gguf",
    );
    let g = qwen.gguf.as_ref().unwrap();
    assert_eq!(g.architecture, "qwen3_asr");
    assert_eq!(g.language, "auto");
    let mut qwen = qwen;
    qwen.gguf.as_mut().unwrap().language = "de".into();
    let same = voxtype::model_catalog::override_config(
        &qwen,
        TranscriptionEngine::Gguf,
        "Qwen3-ASR-0.6B-Q8_0.gguf",
    );
    assert_eq!(same.gguf.unwrap().language, "de");
    let giga = voxtype::model_catalog::override_config(
        &qwen,
        TranscriptionEngine::Gguf,
        "gigaam-v3-e2e-rnnt-Q8_0.gguf",
    );
    let g = giga.gguf.unwrap();
    assert_eq!(g.architecture, "gigaam");
    assert!(voxtype::model_selection::validate_gguf_choice(
        &g.model,
        &g.architecture,
        &g.language,
        g.streaming
    )
    .is_ok());
}

#[cfg(feature = "gguf")]
#[test]
fn cli_can_inspect_and_repair_invalid_saved_gguf_selection() {
    use std::process::Command;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let data = dir.path().join("data");
    let models = data.join("voxtype/models");
    std::fs::create_dir_all(&models).unwrap();
    let mut model = std::fs::File::create(models.join("Qwen3-ASR-0.6B-Q8_0.gguf")).unwrap();
    model.set_len(850_423_456).unwrap();
    use std::io::Write;
    model.write_all(b"GGUF").unwrap();
    let worker = dir.path().join("worker-fixture");
    std::fs::write(&worker, "#!/bin/sh\nexit 99\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let valid = "engine='gguf'\n[gguf]\nmodel='Qwen3-ASR-0.6B-Q8_0.gguf'\narchitecture='qwen3_asr'\nlanguage='de'\nstreaming=false\n";
    for (key, wrong, right) in [
        ("streaming", "true", "false"),
        ("architecture", "gigaam", "qwen3_asr"),
        ("language", "invalid", "de"),
    ] {
        let old = format!(
            "{key}={}",
            if key == "streaming" {
                right.into()
            } else {
                format!("'{right}'")
            }
        );
        let bad = format!(
            "{key}={}",
            if key == "streaming" {
                wrong.into()
            } else {
                format!("'{wrong}'")
            }
        );
        std::fs::write(&path, valid.replace(&old, &bad)).unwrap();
        for args in [
            vec!["config", "schema", "--json"],
            vec!["config", "set", &format!("gguf.{key}"), right],
        ] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtype"));
            for (name, _) in std::env::vars_os() {
                if name.to_string_lossy().starts_with("VOXTYPE_") {
                    cmd.env_remove(name);
                }
            }
            let output = cmd
                .env("HOME", dir.path())
                .env("XDG_DATA_HOME", &data)
                .env("XDG_CONFIG_HOME", dir.path().join("config"))
                .env("XDG_CACHE_HOME", dir.path().join("cache"))
                .env("XDG_RUNTIME_DIR", dir.path().join("runtime"))
                .env("VOXTYPE_GGUF_WORKER", &worker)
                .arg("--config")
                .arg(&path)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let saved: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            saved["gguf"][key].to_string(),
            if key == "streaming" {
                "false".into()
            } else {
                format!("\"{right}\"")
            }
        );
    }
}
