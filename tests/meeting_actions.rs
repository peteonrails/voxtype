//! `voxtype meeting actions`, through the real binary, against scratch XDG
//! directories: the listing the meeting view reads, and reminders sent
//! through a stand-in omarchy-reminder.

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use voxtype::meeting::data::{ActionItem, MeetingMetadata, MeetingSummary};
use voxtype::meeting::{MeetingStorage, StorageConfig};

struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["config/voxtype", "data/voxtype", "runtime", "state", "bin"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        let s = Self { dir };
        std::fs::write(
            s.path("config/voxtype/config.toml"),
            "[meeting]\nenabled = true\nmy_name = \"Pete\"\n\n[meeting.actions]\nremind_in_mins = 45\n",
        )
        .unwrap();
        s
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.dir.path().join(sub)
    }

    fn meeting(&self, items: Vec<ActionItem>) -> String {
        let storage = MeetingStorage::open(StorageConfig {
            storage_path: self.path("data/voxtype/meetings"),
            retain_audio: false,
            max_meetings: 0,
            my_name: None,
        })
        .unwrap();
        let metadata = MeetingMetadata::new(Some("Release sync".into()));
        storage.create_meeting(&metadata).unwrap();
        storage
            .save_summary(
                &metadata.id,
                &MeetingSummary {
                    summary: "Shipping.".into(),
                    key_points: vec![],
                    action_items: items,
                    decisions: vec![],
                    chapters: vec![],
                    generated_at: chrono::Utc::now(),
                    model: None,
                },
            )
            .unwrap();
        metadata.id.to_string()
    }

    /// A stand-in omarchy-reminder that logs its arguments.
    fn fake_reminder(&self) {
        let path = self.path("bin/omarchy-reminder");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n[ \"$1\" = probe ] && exit 0\necho \"$1|$2\" >> {}\n",
                self.path("reminders").display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A process the other test forks meanwhile can hold the write
        // descriptor open; exec until the kernel stops saying ETXTBSY.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match Command::new(&path).arg("probe").status() {
                Err(e) if e.raw_os_error() == Some(26) && std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                other => {
                    assert!(other.unwrap().success());
                    break;
                }
            }
        }
    }

    fn voxtype(&self, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.path("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_voxtype"))
            .env_clear()
            .env("PATH", path)
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("XDG_RUNTIME_DIR", self.path("runtime"))
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env("RUST_LOG", "warn")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

fn ok(output: &Output) -> String {
    assert!(
        output.status.success(),
        "voxtype failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn item(text: &str, owner: &str, due: Option<&str>) -> ActionItem {
    ActionItem {
        description: text.into(),
        assignee: Some(owner.into()),
        due_date: due.map(Into::into),
        ..Default::default()
    }
}

#[test]
fn actions_list_and_go_out_as_reminders_once() {
    let s = Scratch::new();
    let id = s.meeting(vec![
        item("Tag the release", "Dana", Some("tomorrow")),
        item("Write the notes", "Pete", None),
    ]);

    let listing: serde_json::Value =
        serde_json::from_str(&ok(&s.voxtype(&["meeting", "actions", &id, "--json"]))).unwrap();
    assert_eq!(listing["title"], "Release sync");
    assert_eq!(listing["summarized"], true);
    assert_eq!(listing["todoist_connected"], false);
    // Whether this machine has omarchy-reminder; the sends below use a
    // stand-in either way.
    assert!(listing["reminder_available"].is_boolean());
    let items = listing["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["number"], 1);
    assert_eq!(items[0]["owner"], "Dana");
    assert_eq!(items[0]["mine"], false);
    assert_eq!(items[0]["due_said"], "tomorrow");
    assert!(items[0]["due"].is_string());
    assert_eq!(items[1]["mine"], true);

    let mine: serde_json::Value = serde_json::from_str(&ok(
        &s.voxtype(&["meeting", "actions", &id, "--mine", "--json"])
    ))
    .unwrap();
    assert_eq!(mine["items"].as_array().unwrap().len(), 1);

    let text = ok(&s.voxtype(&["meeting", "actions", "latest"]));
    assert!(text.contains("1. Tag the release (Dana, due "), "{text}");
    assert!(text.contains("2. Write the notes (Pete (you))"), "{text}");

    s.fake_reminder();
    let sent: serde_json::Value = serde_json::from_str(&ok(&s.voxtype(&[
        "meeting", "actions", &id, "--send", "reminder", "--pick", "2", "--json",
    ])))
    .unwrap();
    assert_eq!(sent["results"][0]["status"], "sent");
    assert_eq!(sent["results"][0]["minutes"], 45);
    assert_eq!(
        std::fs::read_to_string(s.path("reminders")).unwrap(),
        "45|Write the notes (from Release sync)\n"
    );

    // All by default; the one already sent is skipped.
    let out = ok(&s.voxtype(&[
        "meeting", "actions", &id, "--send", "reminder", "--in", "10",
    ]));
    assert!(out.contains("Sent 1 of 2 to reminders."), "{out}");
    assert!(out.contains("already sent"), "{out}");
    assert_eq!(
        std::fs::read_to_string(s.path("reminders"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    let listing: serde_json::Value =
        serde_json::from_str(&ok(&s.voxtype(&["meeting", "actions", &id, "--json"]))).unwrap();
    assert!(listing["items"][0]["sent"]["reminder"]["at"].is_i64());
}

#[test]
fn todoist_needs_connecting_and_unsummarized_meetings_say_so() {
    let s = Scratch::new();
    let id = s.meeting(vec![item("Tag the release", "Dana", None)]);
    let out = s.voxtype(&["meeting", "actions", &id, "--send", "todoist"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("voxtype meeting todoist connect"));
    assert!(!s.path("state/voxtype/todoist.json").exists());

    let bad = s.voxtype(&["meeting", "actions", &id, "--pick", "7"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("1 to 1"));

    let storage = MeetingStorage::open(StorageConfig {
        storage_path: s.path("data/voxtype/meetings"),
        retain_audio: false,
        max_meetings: 0,
        my_name: None,
    })
    .unwrap();
    let bare = MeetingMetadata::new(Some("No summary".into()));
    storage.create_meeting(&bare).unwrap();
    let text = ok(&s.voxtype(&["meeting", "actions", &bare.id.to_string()]));
    assert!(text.contains("has no summary yet"), "{text}");
}
