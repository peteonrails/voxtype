//! `voxtype meeting soundcheck` end to end, through the real binary, with
//! the audio tools faked: `parec` plays a sine (0.1 for the echo
//! canceller's mic source, 0.5 for the loopback), `pactl` lists the
//! canceller's source as ready, and `pipewire` only waits. So the check
//! opens the meeting audio exactly as a meeting with `echo_cancel = "webrtc"`
//! would, and the levels are known: -23.0 dBFS on the mic, -9.0 on the
//! loopback.

#![cfg(target_os = "linux")]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct World {
    dir: tempfile::TempDir,
}

impl World {
    /// `None` when python3, which plays the fake parec, is missing.
    fn new() -> Option<Self> {
        which_python()?;
        let w = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        let bin = w.p("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // write_script runs its scripts through sh first, so the sine is a
        // Python file of its own that the sh `parec` hands its arguments to.
        std::fs::write(
            bin.join("sine.py"),
            r#"import sys, math, struct, time
dev = sys.argv[sys.argv.index("--device") + 1] if "--device" in sys.argv else ""
amp = 0.1 if "aec" in dev else 0.5
out = sys.stdout.buffer
t = 0
while True:
    block = b"".join(struct.pack("<f", amp * math.sin(2 * math.pi * 440 * (t + i) / 16000)) for i in range(1600))
    t += 1600
    try:
        out.write(block); out.flush()
    except BrokenPipeError:
        break
    time.sleep(0.1)
"#,
        )
        .unwrap();
        common::write_script(
            &bin.join("parec"),
            &format!(
                "#!/bin/sh\nexec python3 -I '{}' \"$@\"\n",
                bin.join("sine.py").display()
            ),
        );
        common::write_script(
            &bin.join("pactl"),
            "#!/bin/sh\ncase \"$*\" in\n  \"list short sources\") printf '1\\tvoxtype_meeting_aec_source\\tPipeWire\\tfloat32le 1ch 48000Hz\\tRUNNING\\n' ;;\nesac\n",
        );
        common::write_script(&bin.join("pipewire"), "#!/bin/sh\nexec sleep 1000\n");
        let config = w.p("config/voxtype");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("config.toml"),
            "[hotkey]\nenabled = false\n\n[osd]\nenabled = false\n\n[whisper]\non_demand_loading = true\n\n\
             [meeting]\nenabled = true\n\n[meeting.audio]\nloopback_device = \"fake.monitor\"\necho_cancel = \"webrtc\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(w.p("run/voxtype")).unwrap();
        Some(w)
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtype"));
        let path = format!(
            "{}:{}",
            self.p("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        cmd.env_clear()
            .env("PATH", path)
            .env("HOME", self.dir.path())
            .env("XDG_RUNTIME_DIR", self.p("run"))
            .env("XDG_CONFIG_HOME", self.p("config"))
            .env("XDG_DATA_HOME", self.p("data"))
            .env("XDG_STATE_HOME", self.p("state"))
            .env("XDG_CACHE_HOME", self.p("cache"))
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env("RUST_LOG", "warn")
            .args(args)
            .stdin(Stdio::null());
        cmd
    }
}

fn which_python() -> Option<()> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .any(|d| Path::new(&d).join("python3").is_file())
        .then_some(())
}

fn events(stdout: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

/// A sound check that ran two seconds: started with the meeting's echo
/// path and devices, about twenty level events at the fakes' levels, then
/// stopped because its time was up.
fn assert_two_second_check(events: &[serde_json::Value]) {
    let first = &events[0];
    assert_eq!(first["event"], "soundcheck");
    assert_eq!(first["state"], "running");
    assert_eq!(first["seconds"], 2);
    assert_eq!(first["echo_path"], "webrtc");
    assert_eq!(first["mic"], "voxtype_meeting_aec_source");
    assert_eq!(first["loopback"], true);

    let levels: Vec<&serde_json::Value> =
        events.iter().filter(|e| e["event"] == "levels").collect();
    assert!((15..=25).contains(&levels.len()), "{} levels", levels.len());
    let last = levels.last().unwrap();
    assert_eq!(last["soundcheck"], true);
    assert!(last["meeting_id"].is_null());
    assert_eq!(last["mic"], -23.0);
    assert_eq!(last["loopback"], -9.0);

    let end = events.last().unwrap();
    assert_eq!(end["event"], "soundcheck");
    assert_eq!(end["state"], "stopped");
    assert_eq!(end["reason"], "done");
}

#[test]
fn without_the_daemon_the_command_opens_the_audio_itself() {
    let Some(w) = World::new() else {
        eprintln!("skipping: python3 not found for the fake parec");
        return;
    };
    let out = w
        .command(&["meeting", "soundcheck", "--seconds", "2", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_two_second_check(&events(&out.stdout));
}

#[test]
fn with_the_daemon_running_the_daemon_does_the_check() {
    let Some(w) = World::new() else {
        eprintln!("skipping: python3 not found for the fake parec");
        return;
    };
    let mut daemon = w
        .command(&["daemon"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while std::os::unix::net::UnixStream::connect(w.p("run/voxtype/meeting.sock")).is_err() {
        assert!(started.elapsed() < Duration::from_secs(30), "no daemon");
        std::thread::sleep(Duration::from_millis(50));
    }

    let out = w
        .command(&["meeting", "soundcheck", "--seconds", "2", "--json"])
        .output()
        .unwrap();
    let _ = daemon.kill();
    let _ = daemon.wait();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_two_second_check(&events(&out.stdout));
}

#[test]
fn a_recording_meeting_already_has_its_levels() {
    let Some(w) = World::new() else {
        return;
    };
    std::fs::write(w.p("run/voxtype/meeting_state"), "recording\nm1").unwrap();
    let out = w.command(&["meeting", "soundcheck"]).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("voxtype meeting watch --levels"), "{err}");
}
