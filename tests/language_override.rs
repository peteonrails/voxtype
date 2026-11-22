//! `--language` and `VOXTYPE_LANGUAGE` set the active engine's language,
//! not only `whisper.language` (#860). Runs the compiled CLI with a private
//! HOME and XDG directories; nothing reaches a daemon.
use std::path::Path;
use std::process::{Command, Output};

const CONFIG: &str = r#"engine = "whisper"
[whisper]
model = "large-v3-turbo"
language = "en"
[cohere]
language = "en"
[sensevoice]
language = "auto"
[gguf]
language = "en"
"#;

fn run(root: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtype"));
    cmd.env_clear()
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .arg("--config")
        .arg(root.join("config.toml"));
    for (name, value) in env {
        cmd.env(name, value);
    }
    cmd.args(args).output().unwrap()
}

fn get(root: &Path, env: &[(&str, &str)], flags: &[&str], key: &str) -> String {
    let args: Vec<&str> = flags
        .iter()
        .copied()
        .chain(["config", "get", key])
        .collect();
    let out = run(root, env, &args);
    assert!(
        out.status.success(),
        "{env:?} {flags:?} {key}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("config.toml"), CONFIG).unwrap();
    root
}

#[test]
fn the_flag_and_the_variable_reach_the_active_engine() {
    let root = fixture();
    let root = root.path();
    for engine in ["cohere", "sensevoice", "gguf"] {
        let key = format!("{engine}.language");
        assert_eq!(
            get(root, &[], &["--engine", engine, "--language", "de"], &key),
            "de",
            "--language with {engine}"
        );
        assert_eq!(
            get(
                root,
                &[("VOXTYPE_ENGINE", engine), ("VOXTYPE_LANGUAGE", "de")],
                &[],
                &key
            ),
            "de",
            "VOXTYPE_LANGUAGE with {engine}"
        );
        // The engine that is not running keeps its own setting.
        assert_eq!(
            get(
                root,
                &[],
                &["--engine", engine, "--language", "de"],
                "whisper.language"
            ),
            "en",
            "whisper.language with {engine}"
        );
    }
    assert_eq!(
        get(root, &[], &["--language", "de"], "whisper.language"),
        "de"
    );
}

#[test]
fn the_flag_wins_over_the_variable_and_follows_a_cli_engine() {
    let root = fixture();
    let root = root.path();
    let env = [("VOXTYPE_LANGUAGE", "de")];
    assert_eq!(
        get(
            root,
            &env,
            &["--engine", "cohere", "--language", "fr"],
            "cohere.language"
        ),
        "fr"
    );
    // The variable was applied to whisper when the config loaded; --engine
    // then picked cohere, and the variable follows it there.
    assert_eq!(
        get(root, &env, &["--engine", "cohere"], "cohere.language"),
        "de"
    );
    // Without either, the file's value stands.
    assert_eq!(
        get(root, &[], &["--engine", "cohere"], "cohere.language"),
        "en"
    );
}

#[test]
fn the_engine_specific_gguf_language_stays_more_specific() {
    let root = fixture();
    let root = root.path();
    assert_eq!(
        get(
            root,
            &[
                ("VOXTYPE_ENGINE", "gguf"),
                ("VOXTYPE_LANGUAGE", "de"),
                ("VOXTYPE_GGUF_LANGUAGE", "fr")
            ],
            &[],
            "gguf.language"
        ),
        "fr"
    );
}

#[test]
fn a_value_the_engine_cannot_take_warns_and_keeps_the_configured_language() {
    let root = fixture();
    let root = root.path();
    let out = run(
        root,
        &[],
        &[
            "--engine",
            "cohere",
            "--language",
            "auto",
            "config",
            "get",
            "cohere.language",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "en");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Cohere cannot take language 'auto'"),
        "{stderr}"
    );

    let out = run(
        root,
        &[],
        &[
            "--engine",
            "parakeet",
            "--language",
            "de",
            "config",
            "get",
            "whisper.language",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("has no language setting"), "{stderr}");
}
