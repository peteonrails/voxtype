//! Focus failures are exercised only through private synthetic IPC and a
//! reexecuted helper with a clean environment, never the developer's compositor.
#![cfg(unix)]

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use voxtype::history::focus::{focused_window, FocusedWindow};

#[tokio::test(flavor = "current_thread")]
#[ignore = "administrative subprocess helper, invoked by the private IPC cases"]
async fn focus_probe_helper() {
    let case = std::env::var("VOXTYPE_TEST_FOCUS_CASE").expect("private focus helper marker");
    let start = std::time::Instant::now();
    let result = focused_window().await;
    if case == "valid" {
        assert_eq!(
            result,
            Some(FocusedWindow {
                app: "synthetic-editor".into(),
                title: "Synthetic document".into()
            })
        );
    } else {
        assert_eq!(result, None);
    }
    if case == "timeout" {
        assert!(start.elapsed() >= Duration::from_millis(250));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}

async fn probe(case: &str) {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("private-niri.sock");
    let server = if case != "unavailable" {
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let case = case.to_string();
        Some(tokio::spawn(async move {
            let (stream, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut stream = BufReader::new(stream);
            let mut request = String::new();
            stream.read_line(&mut request).await.unwrap();
            assert_eq!(request, "\"FocusedWindow\"\n");
            match case.as_str() {
                "valid" => stream.get_mut().write_all(b"{\"Ok\":{\"FocusedWindow\":{\"app_id\":\"synthetic-editor\",\"title\":\"Synthetic document\"}}}\n").await.unwrap(),
                "invalid" => stream.get_mut().write_all(b"not valid JSON\n").await.unwrap(),
                "dropped" => {},
                "timeout" => tokio::time::sleep(Duration::from_secs(8)).await,
                other => panic!("unknown synthetic focus case: {other}"),
            }
        }))
    } else {
        None
    };
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--ignored", "--exact", "focus_probe_helper", "--nocapture"])
        .env_clear()
        .env("HOME", root.path())
        .env("XDG_RUNTIME_DIR", root.path())
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("NIRI_SOCKET", &socket)
        .env("VOXTYPE_TEST_FOCUS_CASE", case)
        .env("PATH", "")
        .kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .unwrap()
        .unwrap();
    if let Some(server) = server {
        if case == "timeout" {
            server.abort();
        } else {
            server.await.unwrap();
        }
    }
    assert!(
        result.status.success(),
        "{case}: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[tokio::test]
async fn history_focus_private_reply_is_parsed() {
    probe("valid").await;
}

#[tokio::test]
async fn history_focus_unavailable_socket_is_unknown() {
    probe("unavailable").await;
}

#[tokio::test]
async fn history_focus_invalid_reply_is_unknown() {
    probe("invalid").await;
}

#[tokio::test]
async fn history_focus_dropped_connection_is_unknown() {
    probe("dropped").await;
}

#[tokio::test]
async fn history_focus_unresponsive_compositor_is_bounded_and_unknown() {
    probe("timeout").await;
}

#[test]
fn history_unknown_focus_preserves_the_documented_recording_default_without_a_title() {
    let settings = voxtype::config::HistoryConfig {
        enabled: true,
        ..Default::default()
    };
    let mut entry = voxtype::history::HistoryEntry::new(chrono::Local::now());
    assert!(!settings.exclude_apps.is_empty());
    assert!(voxtype::history::recorder::apply_context(
        &settings, &mut entry, None, None
    ));
    assert!(entry.app.is_none());
    assert!(entry.window_title.is_none());
}
