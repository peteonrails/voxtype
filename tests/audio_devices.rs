//! `voxtype info devices --json`, the list the Settings panel shows,
//! through the real binary.
//!
//! Runs on CI hosts with no sound card, so the only guarantee is the
//! synthetic `default` entry and no duplicates. Where no sound server
//! answers (sandboxes, clean chroot package builds), the listing used to
//! wait forever in ALSA; it must now give up in time with a clear error,
//! and then there is no list to check.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn info_devices_lists_default_once_or_reports_no_sound_server_in_time() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_voxtype"))
        .args(["info", "devices", "--json"])
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("XDG_STATE_HOME", dir.path().join("state"))
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // 5 s for the sound server, plus room for a slow machine.
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            panic!("voxtype info devices still running after 15 s");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !status.success() {
        assert!(
            stderr.contains("No sound server answered within 5 s"),
            "failed for another reason: {stderr}"
        );
        eprintln!("skipping the device list checks: no sound server answered");
        return;
    }

    let devices: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let devices = devices.as_array().unwrap();
    let defaults: Vec<_> = devices
        .iter()
        .filter(|d| d["default"] == serde_json::Value::Bool(true))
        .collect();
    assert_eq!(defaults.len(), 1, "got {devices:?}");
    assert_eq!(defaults[0]["name"], "default");

    let mut names: Vec<&str> = devices
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), total, "duplicate device names: {devices:?}");
}
