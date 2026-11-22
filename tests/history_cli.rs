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

/// A scratch PATH holding a fake `wl-copy` (`pbcopy` on macOS) and `which`, plus fake typing
/// tools, all of which log their arguments and stdin instead of touching
/// the real clipboard or keyboard.
struct FakeTools {
    bin: std::path::PathBuf,
    log: std::path::PathBuf,
}

impl FakeTools {
    fn new(root: &std::path::Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join("fake-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = root.join("fake-tools.log");
        let write = |name: &str, body: &str| {
            let path = bin.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        write("which", &format!("[ -x \"{}/$1\" ]", bin.display()));
        // The fakes run with PATH set to this directory only, and the Nix
        // build sandbox has no cat in /bin or /usr/bin, so `command -p cat`
        // finds nothing there. Use the cat this test process can see.
        let cat = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .map(|dir| dir.join("cat"))
            .find(|path| path.is_file())
            .unwrap_or_else(|| std::path::PathBuf::from("/bin/cat"));
        for tool in [
            "wl-copy", "pbcopy", "wtype", "dotool", "ydotool", "eitype", "xclip", "xsel",
        ] {
            write(
                tool,
                &format!(
                    "printf '%s %s <' \"{tool}\" \"$*\" >> \"{log}\"\n\"{cat}\" >> \"{log}\"\nprintf '>\\n' >> \"{log}\"",
                    log = log.display(),
                    cat = cat.display()
                ),
            );
        }
        Self { bin, log }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Fixture {
    fn add(&self, edit: impl FnOnce(&mut HistoryEntry)) -> String {
        let store = HistoryStore::open(self.root.path().join("data/voxtype/history")).unwrap();
        let mut entry = HistoryEntry::new(chrono::Local::now());
        edit(&mut entry);
        store.append(&entry).unwrap();
        entry.id
    }

    fn run_with_tools(&self, tools: &FakeTools, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_voxtype"))
            .env_clear()
            .env("HOME", self.root.path())
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("XDG_RUNTIME_DIR", self.root.path().join("runtime"))
            .env("WAYLAND_DISPLAY", "voxtype-test-no-such-display")
            .env("PATH", &tools.bin)
            .args([
                "--config",
                self.root.path().join("config.toml").to_str().unwrap(),
                "history",
            ])
            .args(args)
            .output()
            .unwrap()
    }
}

#[test]
fn copy_runs_the_clipboard_command_with_the_entry_text() {
    let f = Fixture::new();
    let tools = FakeTools::new(f.root.path());
    let out = f.run_with_tools(&tools, &["copy", &f.id, "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["copied"], f.id);
    // macOS copies with pbcopy, everything else with wl-copy here.
    let clipboard = if cfg!(target_os = "macos") {
        "pbcopy"
    } else {
        "wl-copy"
    };
    assert!(
        tools.log().contains(clipboard) && tools.log().contains("<Synthetic invoice.>"),
        "{}",
        tools.log()
    );
}

#[test]
fn copy_and_paste_refuse_an_entry_with_no_text_and_touch_nothing() {
    let f = Fixture::new();
    let tools = FakeTools::new(f.root.path());
    let corrected_empty = f.add(|e| {
        e.raw_text = "um private thing".into();
        e.text = "Private thing.".into();
        e.set_outcome(Outcome::Delivered, None);
        e.corrected_text = Some(String::new());
    });
    let blank = f.add(|e| e.set_outcome(Outcome::Empty, None));

    for (id, why) in [
        (&corrected_empty, "has an empty correction"),
        (&blank, "has no text"),
    ] {
        for verb in ["copy", "paste"] {
            let json = f.run_with_tools(&tools, &[verb, id, "--json"]);
            assert!(!json.status.success(), "{verb} {id} reported success");
            let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
            let error = value["error"].as_str().unwrap_or_default();
            assert!(
                error.contains(why) && error.contains(&format!("no text to {verb}")),
                "{verb} {id}: {value}"
            );
            assert!(value.get("copied").is_none() && value.get("pasted").is_none());

            let human = f.run_with_tools(&tools, &[verb, id]);
            assert!(!human.status.success(), "{verb} {id} reported success");
            assert!(
                String::from_utf8_lossy(&human.stderr).contains(why),
                "{}",
                String::from_utf8_lossy(&human.stderr)
            );
        }
    }
    // The empty correction must not fall back to the original text, and no
    // clipboard or typing command may have run.
    assert_eq!(tools.log(), "", "a tool ran for an entry with no text");
}
