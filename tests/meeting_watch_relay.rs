//! `voxtype meeting watch --json` relays meeting.sock as NDJSON, through
//! the real binary, against a fake socket in a private runtime dir.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn command(runtime: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtype"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("VOXTYPE_") {
            cmd.env_remove(name);
        }
    }
    cmd.env("XDG_RUNTIME_DIR", runtime)
        .env("HOME", runtime)
        .env("XDG_CONFIG_HOME", runtime.join("config"))
        .env("XDG_DATA_HOME", runtime.join("data"))
        .env("XDG_STATE_HOME", runtime.join("state"));
    cmd
}

#[test]
fn json_relays_events_and_a_new_hello_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let sock_dir = dir.path().join("voxtype");
    std::fs::create_dir_all(&sock_dir).unwrap();
    let sock = sock_dir.join("meeting.sock");

    let mut child = command(dir.path())
        .args(["meeting", "watch", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let next = || {
        rx.recv_timeout(Duration::from_secs(10))
            .expect("a relayed line")
    };

    let hello =
        r#"{"event":"hello","status":"recording","meeting_id":"m1","chunk_count":0,"segments":[]}"#;
    let segments = r#"{"event":"segments","chunk_id":0,"segments":[{"id":0,"start_ms":0,"end_ms":900,"text":"hi","source":"microphone","chunk_id":0}]}"#;

    // Started before the daemon: it waits and connects when it appears.
    std::thread::sleep(Duration::from_millis(300));
    let listener = UnixListener::bind(&sock).unwrap();
    let (mut conn, _) = listener.accept().unwrap();
    writeln!(conn, "{hello}\n{segments}").unwrap();
    assert_eq!(next(), hello);
    assert_eq!(next(), segments);

    // The daemon restarts; the relay reconnects and passes the new hello on.
    drop(conn);
    drop(listener);
    std::fs::remove_file(&sock).unwrap();
    let listener = UnixListener::bind(&sock).unwrap();
    let (mut conn, _) = listener.accept().unwrap();
    let hello2 = r#"{"event":"hello","status":"idle","chunk_count":0,"segments":[]}"#;
    writeln!(conn, "{hello2}").unwrap();
    assert_eq!(next(), hello2);

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn the_screen_needs_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let out = command(dir.path())
        .args(["meeting", "watch"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("voxtype meeting watch --json"), "{err}");
}

#[test]
fn meeting_view_says_why_nothing_opens() {
    let dir = tempfile::tempdir().unwrap();
    for (choice, why) in [
        ("none", "configured none"),
        ("gtk4", "configured gtk4, but"),
    ] {
        let out = command(dir.path())
            .args(["meeting", "view"])
            .env("VOXTYPE_MEETING_VIEW", choice)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(!out.status.success(), "{choice}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("No meeting view to open"), "{err}");
        assert!(err.contains(why), "{choice}: {err}");
        assert!(err.contains("voxtype meeting watch"), "{err}");
    }
}

/// Run `meeting watch <args>` against a fake meeting.sock that sends `lines`
/// and return the first `want` lines the relay prints.
fn relayed(args: &[&str], lines: &[&str], want: usize) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let sock_dir = dir.path().join("voxtype");
    std::fs::create_dir_all(&sock_dir).unwrap();
    let listener = UnixListener::bind(sock_dir.join("meeting.sock")).unwrap();
    let mut child = command(dir.path())
        .args(["meeting", "watch"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (mut conn, _) = listener.accept().unwrap();
    for line in lines {
        writeln!(conn, "{line}").unwrap();
    }
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let out = (0..want)
        .map(|_| {
            rx.recv_timeout(Duration::from_secs(10))
                .expect("a relayed line")
        })
        .collect();
    // Nothing more arrives than expected.
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    let _ = child.kill();
    let _ = child.wait();
    out
}

#[test]
fn levels_are_relayed_only_when_asked_for() {
    let hello =
        r#"{"event":"hello","status":"recording","meeting_id":"m1","chunk_count":0,"segments":[]}"#;
    let levels = r#"{"event":"levels","meeting_id":"m1","t_ms":1200,"mic":-23.4,"mic_peak":-15.1,"loopback":-41.0,"loopback_peak":-33.2}"#;
    let segments = r#"{"event":"segments","chunk_id":0,"segments":[]}"#;
    let all = [hello, levels, segments];

    assert_eq!(relayed(&["--levels"], &all, 1), vec![levels]);
    assert_eq!(relayed(&["--json"], &all, 2), vec![hello, segments]);
    assert_eq!(
        relayed(&["--json", "--levels"], &all, 3),
        vec![hello, levels, segments]
    );
}
