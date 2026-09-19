//! Process-isolated configuration tests: no model, GPU, microphone, or daemon.
//! Using subprocesses avoids racing other tests on process-global environment.

use std::process::{Command, Output};

fn query(
    file_backend: Option<&str>,
    env_backend: Option<&str>,
    cli_backend: Option<&str>,
) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut text =
        "engine = \"cohere\"\n[cohere]\nmodel = \"cohere-transcribe-q4f16\"\n".to_owned();
    if let Some(backend) = file_backend {
        text.push_str(&format!("encoder_backend = {backend:?}\n"));
    }
    std::fs::write(&path, text).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtype"));
    // A contributor's own shell settings must not alter the test's inputs.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("VOXTYPE_") {
            cmd.env_remove(key);
        }
    }
    cmd.arg("--config").arg(path);
    if let Some(backend) = env_backend {
        cmd.env("VOXTYPE_COHERE_ENCODER_BACKEND", backend);
    }
    if let Some(backend) = cli_backend {
        cmd.args(["--cohere-encoder-backend", backend]);
    }
    cmd.args(["config", "get", "cohere.encoder_backend"])
        .output()
        .unwrap()
}

#[test]
fn old_config_keeps_onnx_backend() {
    let out = query(None, None, None);
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), "onnx");
}

#[test]
fn cohere_backend_cli_overrides_environment_overrides_file() {
    for (file, env, cli, expected) in [
        ("openvino_gpu", None, None, "openvino_gpu"),
        ("onnx", Some("openvino_gpu"), None, "openvino_gpu"),
        ("openvino_gpu", Some("onnx"), None, "onnx"),
        ("onnx", Some("openvino_gpu"), Some("onnx"), "onnx"),
        (
            "openvino_gpu",
            Some("onnx"),
            Some("openvino_gpu"),
            "openvino_gpu",
        ),
    ] {
        let out = query(Some(file), env, cli);
        assert!(out.status.success(), "{:?}", out);
        assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), expected);
    }
}

#[test]
fn invalid_backend_is_rejected_at_each_boundary() {
    for (file, env, cli) in [
        (Some("auto"), None, None),
        (None, Some("auto"), None),
        (None, None, Some("auto")),
        // Loading is strict: valid higher-precedence settings don't conceal a
        // malformed file or environment value.
        (Some("auto"), Some("onnx"), Some("onnx")),
        (None, Some("auto"), Some("onnx")),
    ] {
        let out = query(file, env, cli);
        assert!(
            !out.status.success(),
            "invalid backend was silently accepted"
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            text.contains("openvino_gpu"),
            "missing valid-choice guidance: {text}"
        );
    }
}
