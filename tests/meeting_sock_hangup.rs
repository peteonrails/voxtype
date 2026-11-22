//! A client that hangs up on meeting.sock must not take the daemon down.
//!
//! The daemon writes every meeting.sock event to every subscriber, and level
//! events come ten times a second, so a meeting view or `meeting watch`
//! closing mid-meeting means writes to a socket whose peer has gone. The
//! daemon runs with SIGPIPE at its default (terminate) for now, so this
//! holds only because those socket writes come back as EPIPE errors rather
//! than the signal. This test keeps it that way.
//!
//! Clients connect, read their `hello` and hang up. Then the daemon is made
//! to send an event to all of them: a `playback` event, which it sends when
//! `meeting play`'s trigger file appears, from the state file `meeting play`
//! writes. The daemon must still be running afterwards.

#![cfg(target_os = "linux")]

use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn clients_hanging_up_on_meeting_sock_do_not_kill_the_daemon() {
    let home = tempfile::tempdir().unwrap();
    let xdg_runtime = home.path().join("run");
    std::fs::create_dir_all(&xdg_runtime).unwrap();
    let config_dir = home.path().join("config/voxtype");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        "[hotkey]\nenabled = false\n\n[osd]\nenabled = false\n\n[whisper]\non_demand_loading = true\n",
    )
    .unwrap();

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_voxtype"))
        .arg("daemon")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", &xdg_runtime)
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let socket = xdg_runtime.join("voxtype/meeting.sock");
    let started = Instant::now();
    while UnixStream::connect(&socket).is_err() {
        if let Some(status) = daemon.try_wait().unwrap() {
            panic!("daemon exited before binding meeting.sock: {status}");
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "meeting.sock never appeared"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    for _ in 0..5 {
        let mut stream = UnixStream::connect(&socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut hello = [0u8; 64];
        let n = std::io::Read::read(&mut stream, &mut hello).unwrap();
        assert!(n > 0, "no hello");
        drop(stream);
    }

    let runtime = xdg_runtime.join("voxtype");
    for round in 0..3 {
        std::fs::write(
            runtime.join("meeting_playback.json"),
            format!(
                r#"{{"meeting_id":"m{round}","track":"mixed","from_ms":0,"pid":1,"start_ticks":0,"started_at":"2026-11-11T00:00:00Z","token":"t","file":"/nonexistent"}}"#
            ),
        )
        .unwrap();
        std::fs::write(runtime.join("meeting_playback"), "").unwrap();
        std::thread::sleep(Duration::from_millis(500));
    }

    let status = daemon.try_wait().unwrap();
    let _ = daemon.kill();
    let _ = daemon.wait();
    assert!(
        status.is_none(),
        "the daemon died while clients hung up: {status:?}"
    );
}
