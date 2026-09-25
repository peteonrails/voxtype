//! A second `voxtype daemon` must refuse to start without touching any of the
//! running instance's shared state. Before the fix it overwrote the PID file,
//! removed the cancel/override files, marked the live daemon's meetings
//! completed, and unlinked and rebound the OSD audio socket (leaving the live
//! daemon's socket unreachable) before it ever checked the lock.
//!
//! The "running daemon" here is simulated: the test process holds the lock
//! with its own PID and owns a listening audio socket, so no model, audio
//! device, or compositor is needed.

#![cfg(unix)]

use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;

#[test]
fn refused_second_instance_leaves_the_running_daemons_state_alone() {
    let home = tempfile::tempdir().unwrap();
    let xdg_runtime = home.path().join("run");
    let runtime = xdg_runtime.join("voxtype");
    std::fs::create_dir_all(&runtime).unwrap();

    let live_pid = std::process::id().to_string();
    std::fs::write(runtime.join("voxtype.lock"), &live_pid).unwrap();
    std::fs::write(runtime.join("pid"), &live_pid).unwrap();
    std::fs::write(runtime.join("cancel"), "").unwrap();
    let socket = runtime.join("audio.sock");
    let _listener = UnixListener::bind(&socket).unwrap();

    let config_dir = home.path().join("config/voxtype");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        "[hotkey]\nenabled = false\n\n[osd]\nenabled = false\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_voxtype"))
        .arg("daemon")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", &xdg_runtime)
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "second instance must refuse");
    assert!(
        stderr.contains("already running"),
        "expected an already-running error, got: {stderr}"
    );

    assert_eq!(
        std::fs::read_to_string(runtime.join("pid")).unwrap(),
        live_pid,
        "PID file was overwritten"
    );
    assert_eq!(
        std::fs::read_to_string(runtime.join("voxtype.lock")).unwrap(),
        live_pid,
        "lockfile was overwritten"
    );
    assert!(runtime.join("cancel").exists(), "cancel file was removed");

    let conn = UnixStream::connect(&socket);
    assert!(
        conn.is_ok(),
        "audio socket no longer reaches the running daemon: {:?}",
        conn.err()
    );
}
