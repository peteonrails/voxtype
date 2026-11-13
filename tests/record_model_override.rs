//! CLI admission only: a private signal sink stands in for the daemon.
//! These tests never open audio or launch an output driver.
#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

struct Fixture {
    root: tempfile::TempDir,
    sink: Child,
}

impl Fixture {
    fn new(whisper: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("runtime/voxtype");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::create_dir_all(root.path().join("data/voxtype/models")).unwrap();
        std::fs::write(runtime.join("state"), "idle").unwrap();
        std::fs::write(
            root.path().join("config.toml"),
            format!("engine = \"whisper\"\nstate_file = \"auto\"\n[whisper]\n{whisper}\n"),
        )
        .unwrap();
        let mut command = Command::new("/bin/sleep");
        command.arg("120").env_clear().stdin(Stdio::null());
        // SAFETY: only async-signal-safe libc calls run between fork and exec.
        unsafe {
            command.pre_exec(|| {
                libc::signal(libc::SIGUSR1, libc::SIG_IGN);
                libc::signal(libc::SIGUSR2, libc::SIG_IGN);
                Ok(())
            });
        }
        let sink = command.spawn().unwrap();
        std::fs::write(runtime.join("voxtype.lock"), sink.id().to_string()).unwrap();
        Self { root, sink }
    }

    fn root(&self) -> &Path {
        self.root.path()
    }

    fn run(&self, args: &[&str]) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_voxtype"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.root())
            .env("XDG_RUNTIME_DIR", self.root().join("runtime"))
            .env("XDG_CONFIG_HOME", self.root().join("config"))
            .env("XDG_DATA_HOME", self.root().join("data"))
            .env("XDG_STATE_HOME", self.root().join("state"))
            .current_dir(self.root())
            .arg("--config")
            .arg(self.root().join("config.toml"))
            .args(args)
            .output()
            .unwrap();
        eprintln!("CLI {args:?}: {output:?}");
        output
    }

    fn install(&self, filename: &str) {
        std::fs::write(
            self.root().join("data/voxtype/models").join(filename),
            b"lmgg\x00\x01\x02\x03",
        )
        .unwrap();
    }

    fn assert_rejected(&self, args: &[&str], remedy: &str) {
        let before = self.runtime_files();
        let output = self.run(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(remedy),
            "{args:?}: {output:?}"
        );
        assert_eq!(self.runtime_files(), before, "rejection changed IPC files");
        assert_eq!(
            std::fs::read_to_string(self.root().join("runtime/voxtype/state")).unwrap(),
            "idle"
        );
    }

    fn runtime_files(&self) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = std::fs::read_dir(self.root().join("runtime/voxtype"))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    }

    fn assert_admitted(&self, args: &[&str], model: &str) {
        let output = self.run(args);
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert_eq!(
            std::fs::read_to_string(self.root().join("runtime/voxtype/model_override")).unwrap(),
            model
        );
        assert!(self.root().join("runtime/voxtype/start_requested").exists());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.sink.kill();
        let _ = self.sink.wait();
    }
}

#[test]
fn invalid_local_model_rejects_both_flag_positions_and_start_toggle_without_ipc() {
    for action in ["start", "toggle"] {
        let f = Fixture::new("model = \"base.en\"");
        f.assert_rejected(
            &[
                "--model",
                "nonexistent",
                "record",
                action,
                "--clipboard",
                "--no-osd",
            ],
            "voxtype info models",
        );
        f.assert_rejected(
            &[
                "record",
                action,
                "--model",
                "nonexistent",
                "--clipboard",
                "--no-osd",
            ],
            "voxtype info models",
        );
    }
}

#[test]
fn missing_local_models_reject_even_when_configured() {
    let f = Fixture::new(
        "model = \"base.en\"\nsecondary_model = \"small.en\"\navailable_models = [\"tiny.en\"]",
    );
    for model in [
        "base.en",
        "small.en",
        "tiny.en",
        "missing.bin",
        "/missing/model.bin",
    ] {
        f.assert_rejected(&["record", "start", "--model", model], "not found");
    }
}

#[test]
fn valid_local_names_aliases_and_paths_keep_model_manager_admission() {
    let f = Fixture::new(
        "model = \"base.en\"\nsecondary_model = \"small.en\"\navailable_models = [\"tiny.en\"]",
    );
    for file in [
        "ggml-base.en.bin",
        "ggml-small.en.bin",
        "ggml-tiny.en.bin",
        "ggml-large-v1.bin",
        "custom.bin",
    ] {
        f.install(file);
    }
    for model in [
        "base.en",
        "small.en",
        "tiny.en",
        "large",
        "large-v1",
        "custom.bin",
    ] {
        f.assert_admitted(&["record", "start", "--model", model], model);
    }
    let absolute = f.root().join("data/voxtype/models/custom.bin");
    let absolute = absolute.to_str().unwrap();
    f.assert_admitted(&["record", "toggle", "--model", absolute], absolute);
    std::fs::write(f.root().join("relative.bin"), b"lmgg").unwrap();
    f.assert_admitted(
        &["record", "start", "--model", "./relative.bin"],
        "./relative.bin",
    );
    std::fs::create_dir(f.root().join("models")).unwrap();
    std::fs::write(f.root().join("models/ggml-medium.en.bin"), b"lmgg").unwrap();
    f.assert_admitted(&["record", "start", "--model", "medium.en"], "medium.en");
}

#[test]
fn subcommand_model_wins_over_global_for_start_and_toggle() {
    for action in ["start", "toggle"] {
        let f = Fixture::new("model = \"base.en\"");
        f.install("ggml-base.en.bin");
        f.assert_admitted(
            &[
                "--model",
                "nonexistent",
                "record",
                action,
                "--model",
                "base.en",
            ],
            "base.en",
        );
        let f = Fixture::new("model = \"base.en\"");
        f.install("ggml-base.en.bin");
        f.assert_rejected(
            &[
                "--model",
                "base.en",
                "record",
                action,
                "--model",
                "nonexistent",
            ],
            "voxtype info models",
        );
    }
}

#[test]
fn remote_cli_and_legacy_backend_keep_arbitrary_names() {
    for settings in [
        "mode = \"remote\"",
        "mode = \"cli\"",
        "backend = \"remote\"",
        "backend = \"cli\"",
        "mode = \"remote\"\nbackend = \"local\"",
    ] {
        for action in ["start", "toggle"] {
            let f = Fixture::new(&format!("model = \"base.en\"\n{settings}"));
            f.assert_admitted(
                &["record", action, "--model", "provider-only-name"],
                "provider-only-name",
            );
        }
    }
    let f = Fixture::new("model = \"base.en\"\nmode = \"local\"\nbackend = \"remote\"");
    f.assert_rejected(
        &["record", "start", "--model", "provider-only-name"],
        "voxtype info models",
    );
}

#[test]
fn config_command_keeps_intentional_unknown_model_fallback() {
    let f = Fixture::new("model = \"base.en\"");
    let output = f.run(&["--model", "nonexistent", "config"]);
    assert!(output.status.success(), "{output:?}");
    let logs = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(logs.contains("using default model"), "{output:?}");
}

#[test]
fn transcribe_mode_flags_admit_provider_names_before_reading_audio() {
    for mode in ["remote", "cli"] {
        let f = Fixture::new("model = \"base.en\"");
        let before = f.runtime_files();
        let output = f.run(&[
            "--whisper-mode",
            mode,
            "--model",
            "provider-only-name",
            "transcribe",
            "missing-audio.wav",
        ]);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("Loading audio file:"),
            "{mode}: {output:?}"
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Unknown model"));
        assert_eq!(f.runtime_files(), before);
    }
}

#[test]
fn transcribe_explicit_local_mode_rejects_remote_config_provider_name() {
    let f = Fixture::new("model = \"base.en\"\nmode = \"remote\"");
    f.assert_rejected(
        &[
            "--whisper-mode",
            "local",
            "--model",
            "provider-only-name",
            "transcribe",
            "missing-audio.wav",
        ],
        "voxtype info models",
    );
}
