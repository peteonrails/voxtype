//! `voxtype meeting notice`: the recording-consent notice from the command
//! line, against scratch XDG state and runtime directories.

use std::process::{Command, Output};

const NOTICE: &str = "Laws on recording and transcribing conversations differ by place, and some \
require everyone on the call to agree. Voxtype doesn't tell the other people for you, so let them \
know you're using a meeting assistant.";

const NOTICE_ANNOUNCED: &str = "Voxtype will announce the recording to the call. You're still \
responsible for getting consent where the law requires it.";

struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["config/voxtype", "data", "state", "runtime/voxtype"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        Self { dir }
    }

    fn path(&self, sub: &str) -> std::path::PathBuf {
        self.dir.path().join(sub)
    }

    fn voxtype(&self, args: &[&str]) -> Output {
        self.voxtype_env(args, &[])
    }

    fn voxtype_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_voxtype"))
            .env_remove("VOXTYPE_MEETING_ANNOUNCE_RECORDING")
            .env_remove("VOXTYPE_MEETING_ANNOUNCEMENT_FILE")
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("XDG_RUNTIME_DIR", self.path("runtime"))
            .envs(env.iter().copied())
            .args(args)
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        let out = self.voxtype(args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
}

#[test]
fn a_new_install_has_not_acknowledged_the_notice() {
    let s = Scratch::new();
    let v = s.json(&["meeting", "notice", "--json"]);
    assert_eq!(v["notice"], NOTICE);
    assert_eq!(v["acknowledged"], false);
    assert!(v["acknowledged_at"].is_null());

    let out = s.voxtype(&["meeting", "notice"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with(NOTICE), "{text}");
    assert!(
        text.contains("Not acknowledged yet. Run `voxtype meeting notice --ack`"),
        "{text}"
    );
}

#[test]
fn ack_records_it_once_and_tells_the_daemon() {
    let s = Scratch::new();
    let v = s.json(&["meeting", "notice", "--ack", "--json"]);
    assert_eq!(v["acknowledged"], true);
    let at = v["acknowledged_at"].as_str().unwrap().to_string();

    let saved: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(s.path("state/voxtype/recording-notice.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["acknowledged_at"], at.as_str());
    assert!(
        s.path("runtime/voxtype/meeting_notice_ack").exists(),
        "the daemon's trigger is dropped"
    );

    // Later reads and a second --ack keep the first time.
    assert_eq!(
        s.json(&["meeting", "notice", "--json"])["acknowledged_at"],
        at.as_str()
    );
    assert_eq!(
        s.json(&["meeting", "notice", "--ack", "--json"])["acknowledged_at"],
        at.as_str()
    );
    let text = String::from_utf8_lossy(&s.voxtype(&["meeting", "notice"]).stdout).to_string();
    assert!(text.contains("Acknowledged "), "{text}");
}

/// A scratch install `meeting start` accepts: meeting mode on, the default
/// model and GTCRN present (empty files are enough to pass the checks), and
/// a "daemon" lockfile naming this live test process.
fn startable(s: &Scratch) {
    std::fs::write(
        s.path("config/voxtype/config.toml"),
        "[meeting]\nenabled = true\n",
    )
    .unwrap();
    let models = s.path("data/voxtype/models");
    std::fs::create_dir_all(&models).unwrap();
    for f in ["ggml-base.en.bin", "gtcrn_simple.onnx"] {
        std::fs::write(models.join(f), b"").unwrap();
    }
    std::fs::write(
        s.path("runtime/voxtype/voxtype.lock"),
        std::process::id().to_string(),
    )
    .unwrap();
}

#[test]
fn start_prints_the_notice_on_stderr_until_acknowledged() {
    let s = Scratch::new();
    startable(&s);
    let out = s.voxtype(&["meeting", "start", "--title", "Standup"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{stdout}{stderr}");
    assert!(stdout.starts_with("Meeting start requested"), "{stdout}");
    assert!(
        !stdout.contains("Laws on recording"),
        "stdout stays parseable: {stdout}"
    );
    assert!(stderr.contains(&format!("Note: {NOTICE}")), "{stderr}");
    assert!(stderr.contains("voxtype meeting notice --ack"), "{stderr}");

    // Acknowledged: a start says nothing about it.
    std::fs::remove_file(s.path("runtime/voxtype/meeting_start")).unwrap();
    s.json(&["meeting", "notice", "--ack", "--json"]);
    let out = s.voxtype(&["meeting", "start", "--title", "Standup"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("Laws on recording"));
}

/// With the recording announcement on, the notice must not claim voxtype
/// tells nobody.
#[test]
fn announcing_changes_the_wording_everywhere_the_notice_shows() {
    let s = Scratch::new();
    startable(&s);
    let on = [("VOXTYPE_MEETING_ANNOUNCE_RECORDING", "true")];

    let out = s.voxtype_env(&["meeting", "notice", "--json"], &on);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["notice"], NOTICE_ANNOUNCED);
    assert_eq!(v["announce_recording"], true);
    let text =
        String::from_utf8_lossy(&s.voxtype_env(&["meeting", "notice"], &on).stdout).to_string();
    assert!(text.starts_with(NOTICE_ANNOUNCED), "{text}");
    assert!(!text.contains("doesn't tell"), "{text}");

    let out = s.voxtype_env(&["meeting", "start", "--title", "Standup"], &on);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.contains(&format!("Note: {NOTICE_ANNOUNCED}")),
        "{stderr}"
    );
    assert!(!stderr.contains("doesn't tell"), "{stderr}");

    // --no-announce for this meeting: the usual wording again.
    std::fs::remove_file(s.path("runtime/voxtype/meeting_start")).unwrap();
    let out = s.voxtype_env(
        &["meeting", "start", "--title", "Standup", "--no-announce"],
        &on,
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains(&format!("Note: {NOTICE}")), "{stderr}");
}
