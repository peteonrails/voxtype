//! Process-local SIGPIPE coverage. All state and model paths are private fixtures.
#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "CLI SIGPIPE fixture timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Fixture {
    private: tempfile::TempDir,
    config: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let private = tempfile::tempdir().unwrap();
        let runtime = private.path().join("runtime/voxtype");
        std::fs::create_dir_all(&runtime).unwrap();
        // This live PID is a liveness-only fixture, never a signal target.
        std::fs::write(runtime.join("voxtype.lock"), std::process::id().to_string()).unwrap();
        // More than a pipe buffer so head reliably closes before the final write.
        std::fs::write(runtime.join("state"), "idle\n".repeat(262_144)).unwrap();
        let config = private.path().join("config.toml");
        std::fs::write(&config, "state_file = \"auto\"\n").unwrap();
        Self { private, config }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_voxtype"));
        command
            .args(["--quiet", "--config"])
            .arg(&self.config)
            .env("HOME", self.private.path())
            .env("XDG_RUNTIME_DIR", self.private.path().join("runtime"))
            .env("XDG_CONFIG_HOME", self.private.path().join("config"))
            .env("XDG_DATA_HOME", self.private.path().join("data"))
            .env("XDG_STATE_HOME", self.private.path().join("state"))
            .env("RUST_LOG", "off")
            .stdin(Stdio::null());
        command
    }
}

#[test]
fn status_and_follow_exit_quietly_when_head_closes() {
    for follow in [false, true] {
        let fixture = Fixture::new();
        let mut command = fixture.command();
        command.arg("status");
        if follow {
            command.arg("--follow");
        }
        let mut producer = ChildGuard(
            command
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut head = ChildGuard(
            Command::new("head")
                .arg("-1")
                .stdin(producer.0.stdout.take().unwrap())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        assert!(wait(&mut head.0).success());
        assert_eq!(wait(&mut producer.0).signal(), Some(libc::SIGPIPE));
        let mut output = String::new();
        use std::io::Read;
        head.0
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert_eq!(output, "idle\n");
        let mut errors = String::new();
        producer
            .0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut errors)
            .unwrap();
        assert!(errors.is_empty(), "{errors}");
    }
}

#[test]
fn transcribe_worker_keeps_error_json_on_stdout() {
    let fixture = Fixture::new();
    let missing = fixture.private.path().join("missing-model.bin");
    let mut child = ChildGuard(
        fixture
            .command()
            .args(["transcribe-worker", "--model"])
            .arg(missing)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    assert!(wait(&mut child.0).success());
    use std::io::Read;
    let mut output = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    let response: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(response["ok"], false);
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("Failed to load model"));
    assert!(!output.contains("READY"));
    let mut errors = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut errors)
        .unwrap();
    assert!(errors.contains("[worker] Loading model:"), "{errors}");
}

#[test]
fn transcribe_worker_keeps_default_sigpipe() {
    let fixture = Fixture::new();
    use std::os::fd::OwnedFd;
    let (closed_reader, writer) = std::os::unix::net::UnixStream::pair().unwrap();
    drop(closed_reader);
    let mut child = ChildGuard(
        fixture
            .command()
            .args(["transcribe-worker", "--model"])
            .arg(fixture.private.path().join("missing-model.bin"))
            .stdout(Stdio::from(OwnedFd::from(writer)))
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    assert_eq!(wait(&mut child.0).signal(), Some(libc::SIGPIPE));
}
