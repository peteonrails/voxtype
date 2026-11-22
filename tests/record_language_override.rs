//! `record start --language` (#484), CLI admission only: a private signal
//! sink stands in for the daemon. These tests never open audio or launch an
//! output driver.
#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

struct Fixture {
    root: tempfile::TempDir,
    sink: Child,
}

impl Fixture {
    fn new(config: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("runtime/voxtype");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::create_dir_all(root.path().join("data/voxtype/models")).unwrap();
        std::fs::write(runtime.join("state"), "idle").unwrap();
        std::fs::write(
            root.path().join("config.toml"),
            format!("state_file = \"auto\"\n{config}\n"),
        )
        .unwrap();
        // Reexecute this test binary: Nix sandboxes need not provide /bin/sleep.
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "private_signal_sink"])
            .env_clear()
            .env("VOXTYPE_RECORD_LANGUAGE_SIGNAL_SINK", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
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
            .env("PATH", self.root().join("empty-path"))
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

    fn language_file(&self) -> Option<String> {
        std::fs::read_to_string(self.root().join("runtime/voxtype/language_override")).ok()
    }

    fn started(&self) -> bool {
        self.root().join("runtime/voxtype/start_requested").exists()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.sink.kill();
        let _ = self.sink.wait();
    }
}

#[test]
fn private_signal_sink() {
    if std::env::var_os("VOXTYPE_RECORD_LANGUAGE_SIGNAL_SINK").is_some() {
        std::thread::sleep(std::time::Duration::from_secs(120));
    }
}

const MULTILINGUAL_WHISPER: &str =
    "engine = \"whisper\"\n[whisper]\nmodel = \"large-v3-turbo\"\nlanguage = \"es\"";

#[test]
fn start_and_toggle_hand_the_language_to_the_daemon() {
    for action in ["start", "toggle"] {
        let f = Fixture::new(MULTILINGUAL_WHISPER);
        let output = f.run(&["record", action, "--language", "en"]);
        assert!(output.status.success(), "{action}: {output:?}");
        assert_eq!(f.language_file().as_deref(), Some("en"), "{action}");
        assert!(f.started(), "{action}");
    }
}

#[test]
fn the_subcommand_flag_wins_and_the_top_level_flag_also_applies() {
    let f = Fixture::new(MULTILINGUAL_WHISPER);
    let output = f.run(&["--language", "de", "record", "start", "--language", "fr"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(f.language_file().as_deref(), Some("fr"));

    let f = Fixture::new(MULTILINGUAL_WHISPER);
    let output = f.run(&["--language", "de", "record", "start"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(f.language_file().as_deref(), Some("de"));
}

#[test]
fn a_recording_without_a_language_writes_none() {
    let f = Fixture::new(MULTILINGUAL_WHISPER);
    let output = f.run(&["record", "start"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(f.language_file(), None);
}

#[test]
fn a_language_that_cannot_apply_is_refused_before_recording() {
    for (config, language, reason) in [
        (
            "engine = \"parakeet\"\n[parakeet]\nmodel = \"parakeet-tdt-0.6b-v3\"",
            "de",
            "has no language setting",
        ),
        (
            "engine = \"whisper\"\n[whisper]\nmodel = \"base.en\"",
            "de",
            "writes only 'en'",
        ),
        (
            "engine = \"cohere\"\n[cohere]\nlanguage = \"en\"",
            "auto",
            "Cohere cannot take language 'auto'",
        ),
    ] {
        let f = Fixture::new(config);
        let output = f.run(&["record", "start", "--language", language]);
        assert_eq!(output.status.code(), Some(1), "{config}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(reason), "{config}: {stderr}");
        assert_eq!(f.language_file(), None, "{config}");
        assert!(!f.started(), "{config}");
    }
}

#[test]
fn a_top_level_language_that_cannot_apply_only_warns() {
    // `voxtype --language en record start` was accepted and ignored before
    // #484, so a keybinding that passes it keeps recording.
    let f = Fixture::new("engine = \"parakeet\"\n[parakeet]\nmodel = \"parakeet-tdt-0.6b-v3\"");
    let output = f.run(&["--language", "de", "record", "start"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("ignored for this recording"));
    assert_eq!(f.language_file(), None);
    assert!(f.started());
}
