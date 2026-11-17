//! `voxtype meeting calendar`, through the real binary, against scratch
//! XDG directories and a local server playing the calendar. The private
//! link must never show up in output, errors or config.toml.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const FIXTURE: &str = include_str!("fixtures/calendar/fixture.ics");
const SECRET: &str = "private-9f8e7d6c5b4a";

/// Serves the fixture at any path, except 404 for paths containing "gone".
fn calendar_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let (status, body) = if first.contains("gone") {
                (404, "")
            } else {
                (200, FIXTURE)
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: text/calendar\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    base
}

struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["config/voxtype", "data", "runtime", "state"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        std::fs::write(
            dir.path().join("config/voxtype/config.toml"),
            "[meeting]\nenabled = true\n",
        )
        .unwrap();
        Self { dir }
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.dir.path().join(sub)
    }

    fn voxtype(&self, args: &[&str], stdin: &str) -> Output {
        self.run(args, stdin, "warn")
    }

    /// With every log line on, to show none of them names the link.
    fn voxtype_traced(&self, args: &[&str], stdin: &str) -> Output {
        self.run(args, stdin, "trace")
    }

    fn run(&self, args: &[&str], stdin: &str, log: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_voxtype"))
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("XDG_RUNTIME_DIR", self.path("runtime"))
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env("RUST_LOG", log)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn a_link_from_stdin_is_checked_kept_private_and_cleared() {
    let base = calendar_server();
    let s = Scratch::new();
    let link = format!("{base}/calendar/ical/{SECRET}/basic.ics");

    let traced = s.voxtype_traced(
        &["meeting", "calendar", "set-ics", "-"],
        &format!("{link}\n"),
    );
    assert!(traced.status.success(), "{}", text(&traced));
    assert!(!text(&traced).contains(SECRET), "{}", text(&traced));
    let set = s.voxtype(
        &["meeting", "calendar", "set-ics", "-", "--json"],
        &format!("{link}\n"),
    );
    assert!(set.status.success(), "{}", text(&set));
    assert!(!text(&set).contains(SECRET), "{}", text(&set));
    let reply: serde_json::Value = serde_json::from_slice(&set.stdout).unwrap();
    assert_eq!(reply["set"], true);
    assert_eq!(reply["host"], base.trim_start_matches("http://"));
    assert!(reply["events"].is_u64());

    use std::os::unix::fs::PermissionsExt;
    let file = s.path("state/voxtype/calendar.json");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(std::fs::read_to_string(&file).unwrap().contains(SECRET));
    let config = std::fs::read_to_string(s.path("config/voxtype/config.toml")).unwrap();
    assert!(!config.contains(SECRET), "never in config.toml");

    let traced = s.voxtype_traced(&["meeting", "calendar", "status"], "");
    assert!(!text(&traced).contains(SECRET), "{}", text(&traced));
    let status = s.voxtype(&["meeting", "calendar", "status", "--json"], "");
    assert!(status.status.success(), "{}", text(&status));
    assert!(!text(&status).contains(SECRET), "{}", text(&status));
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["source"], "calendar_link");
    assert_eq!(status["calendar_link"]["set"], true);
    assert_eq!(
        status["calendar_link"]["last_error"],
        serde_json::Value::Null
    );
    assert!(status["link_help"]
        .as_str()
        .unwrap()
        .contains("Secret address in iCal format"));

    let clear = s.voxtype(&["meeting", "calendar", "clear", "--json"], "");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&clear.stdout).unwrap()["cleared"],
        true
    );
    assert!(!file.exists());
    assert!(!s.path("state/voxtype/calendar.ics").exists());
}

#[test]
fn a_link_that_fails_is_refused_and_its_secret_never_printed() {
    let base = calendar_server();
    let s = Scratch::new();
    let out = s.voxtype_traced(
        &[
            "meeting",
            "calendar",
            "set-ics",
            &format!("{base}/gone/{SECRET}/basic.ics"),
        ],
        "",
    );
    assert!(!out.status.success());
    let all = text(&out);
    assert!(all.contains("404") && all.contains("reset"), "{all}");
    assert!(!all.contains(SECRET), "{all}");
    assert!(!s.path("state/voxtype/calendar.json").exists(), "not kept");

    let bad = s.voxtype(
        &["meeting", "calendar", "set-ics", "calendar.example.com"],
        "",
    );
    assert!(!bad.status.success());
    assert!(text(&bad).contains("not a calendar link"));

    // --no-check keeps a link that cannot be reached right now.
    let kept = s.voxtype(
        &[
            "meeting",
            "calendar",
            "set-ics",
            "--no-check",
            &format!("webcal://cal.example.com/{SECRET}"),
        ],
        "",
    );
    assert!(kept.status.success(), "{}", text(&kept));
    assert!(!text(&kept).contains(SECRET));
    let saved = std::fs::read_to_string(s.path("state/voxtype/calendar.json")).unwrap();
    assert!(
        saved.contains("https://cal.example.com/"),
        "webcal is https"
    );
}

#[test]
fn help_says_where_to_find_the_link() {
    let s = Scratch::new();
    let out = s.voxtype(&["meeting", "calendar", "set-ics", "--help"], "");
    let help = String::from_utf8_lossy(&out.stdout);
    for provider in ["Google Calendar", "Outlook", "iCloud", "Fastmail"] {
        assert!(help.contains(provider), "{help}");
    }
    assert!(help.contains("lets anyone who has it read your calendar"));
}
