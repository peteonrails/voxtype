//! History commands run only against private synthetic data, without desktop IO.
use std::process::{Command, Output};
use voxtype::history::{HistoryEntry, HistoryStore, Outcome};

struct Fixture {
    root: tempfile::TempDir,
    id: String,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("config.toml"),
            "[history]\nenabled = true\n",
        )
        .unwrap();
        let store = HistoryStore::open(root.path().join("data/voxtype/history")).unwrap();
        let mut entry = HistoryEntry::new(chrono::Local::now());
        entry.raw_text = "um synthetic invoice".into();
        entry.text = "Synthetic invoice.".into();
        entry.set_outcome(Outcome::Delivered, None);
        store.append(&entry).unwrap();
        Self { root, id: entry.id }
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_voxtype"))
            .env_clear()
            .env("HOME", self.root.path())
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("XDG_RUNTIME_DIR", self.root.path().join("runtime"))
            .env("PATH", "")
            .args([
                "--config",
                self.root.path().join("config.toml").to_str().unwrap(),
                "history",
            ])
            .args(args)
            .output()
            .unwrap()
    }
    fn json(&self, args: &[&str]) -> serde_json::Value {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
}

#[test]
fn history_lists_and_shows_synthetic_entries_as_json() {
    let f = Fixture::new();
    let recent = f.json(&["--json"]);
    assert_eq!(recent["entries"][0]["id"], f.id);
    assert_eq!(recent["enabled"], true);
    assert_eq!(recent["paused"], false);
    let entry = f.json(&["show", &f.id, "--json"]);
    assert_eq!(entry["text"], "Synthetic invoice.");
    let corrected = f.json(&["correct", &f.id, "Synthetic purchase order.", "--json"]);
    assert_eq!(corrected["corrected_text"], "Synthetic purchase order.");
    let hits = f.json(&["search", "purchase", "--json"]);
    assert_eq!(hits[0]["source"], "dictation");
    assert_eq!(hits[0]["id"], f.id);
    assert_eq!(f.json(&["pause", "--json"])["paused"], true);
    assert_eq!(f.json(&["resume", "--json"])["paused"], false);
    assert_eq!(f.json(&["disable", "--json"])["enabled"], false);
    assert_eq!(f.json(&["enable", "--json"])["enabled"], true);
    let before = f.json(&["reindex", "--json"]);
    assert!(before["days"].as_u64().is_some());
    let deletion = f.json(&["delete", &f.id, "--json"]);
    assert_eq!(deletion["deleted"], f.id);
    assert!(f
        .json(&["search", "purchase", "--json"])
        .as_array()
        .unwrap()
        .is_empty());
    assert!(f.json(&["list", "--json"])["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!f
        .run(&["clear", "--before", "not-a-date", "--yes", "--json"])
        .status
        .success());
    assert_eq!(f.json(&["clear", "--yes", "--json"])["days"], 0);
}

#[test]
fn history_has_a_terminal_view_frontdoor() {
    let f = Fixture::new();
    let result = f.run(&["view", "--help"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("--tui"));
}

#[test]
fn history_view_json_is_noninteractive_and_uses_the_same_authoritative_entries() {
    let f = Fixture::new();
    let value = f.json(&["view", "--json"]);
    assert_eq!(value["entries"][0]["id"], f.id);
    assert_eq!(value["entries"][0]["raw_text"], "um synthetic invoice");
    assert_eq!(value["enabled"], true);
}
