//! #1102: CLI writes through a symlinked config.toml (GNU Stow, dotfiles)
//! land in the file the link names and leave the link in place.
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Stowed {
    root: tempfile::TempDir,
    link: PathBuf,
    target: PathBuf,
}

const LINK: &str = "../../dotfiles/voxtype/config.toml";

impl Stowed {
    fn new(contents: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let dotfiles = root.path().join("dotfiles/voxtype");
        let config = root.path().join("config/voxtype");
        std::fs::create_dir_all(&dotfiles).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        let target = dotfiles.join("config.toml");
        std::fs::write(&target, contents).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
        let link = config.join("config.toml");
        symlink(LINK, &link).unwrap();
        Self { root, link, target }
    }

    /// Run voxtype with every XDG dir inside the fixture. `config` is passed
    /// as `--config`; without it the default path (the link) is used.
    fn run(&self, config: Option<&Path>, args: &[&str]) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_voxtype"));
        command
            .env_clear()
            .env("HOME", self.root.path())
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .env("XDG_STATE_HOME", self.root.path().join("state"))
            .env("XDG_RUNTIME_DIR", self.root.path().join("runtime"))
            .env(
                "VOXTYPE_TEST_SYSTEM_CONFIG",
                self.root.path().join("etc/voxtype/config.toml"),
            )
            .env("PATH", "");
        if let Some(config) = config {
            command.arg("--config").arg(config);
        }
        let output = command.args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn assert_link_intact(&self) {
        assert_eq!(std::fs::read_link(&self.link).unwrap(), Path::new(LINK));
    }
}

#[test]
fn config_set_writes_through_a_symlinked_config() {
    let original = "# from my dotfiles\n[hotkey]\nkey = \"SCROLLLOCK\"\n";
    let stowed = Stowed::new(original);

    stowed.run(Some(&stowed.link), &["config", "set", "hotkey.key", "F13"]);
    stowed.assert_link_intact();
    let text = std::fs::read_to_string(&stowed.target).unwrap();
    assert!(
        text.contains("F13") && text.contains("# from my dotfiles"),
        "{text}"
    );
    assert_eq!(
        std::fs::metadata(&stowed.target)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    // The first-save backup is of the dotfiles file, beside it.
    let bak = stowed.target.with_file_name("config.toml.bak");
    assert_eq!(std::fs::read_to_string(bak).unwrap(), original);
    assert!(!stowed.link.with_file_name("config.toml.bak").exists());

    // The default path resolves to the same link.
    stowed.run(None, &["config", "unset", "hotkey.key"]);
    stowed.assert_link_intact();
    assert!(!std::fs::read_to_string(&stowed.target)
        .unwrap()
        .contains("F13"));
}

/// The link path and the file it names are one config: a pause through
/// either advances one shared privacy generation, so a daemon that knows
/// the config by its link sees a pause made with `-c <dotfiles file>`.
#[test]
fn link_and_target_share_one_history_privacy_state() {
    let stowed = Stowed::new("[history]\nenabled = true\n");
    stowed.run(None, &["history", "pause"]);
    stowed.run(Some(&stowed.target), &["history", "resume"]);
    stowed.run(Some(&stowed.target), &["history", "pause"]);
    stowed.assert_link_intact();
    assert!(std::fs::read_to_string(&stowed.target)
        .unwrap()
        .contains("paused = true"));

    let control = stowed.root.path().join("data/voxtype/history-control");
    let keys: Vec<_> = std::fs::read_dir(&control)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(keys.len(), 1, "one config, one privacy key: {keys:?}");
    let generation = std::fs::read(keys[0].join("generation")).unwrap();
    assert_eq!(u64::from_le_bytes(generation.try_into().unwrap()), 2);
}

/// A link whose target is missing (a Stow tree that is not mounted) is still
/// the user's config. Voxtype used to skip it and load the system config,
/// so `config set` and `history pause` then tried to write that instead.
#[test]
fn a_dangling_link_is_the_users_config_even_with_a_system_config() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config/voxtype");
    std::fs::create_dir_all(&config).unwrap();
    let link = config.join("config.toml");
    symlink(LINK, &link).unwrap();
    let target = root.path().join("dotfiles/voxtype/config.toml");
    assert!(!target.exists() && !target.parent().unwrap().exists());

    // Stands in for /etc/voxtype/config.toml: present, and not the user's.
    let etc = root.path().join("etc/voxtype");
    std::fs::create_dir_all(&etc).unwrap();
    let system = etc.join("config.toml");
    let system_text = "# system config\n[hotkey]\nkey = \"F12\"\n";
    std::fs::write(&system, system_text).unwrap();
    std::fs::set_permissions(&system, std::fs::Permissions::from_mode(0o444)).unwrap();
    std::fs::set_permissions(&etc, std::fs::Permissions::from_mode(0o555)).unwrap();

    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_voxtype"))
            .env_clear()
            .env("HOME", root.path())
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_STATE_HOME", root.path().join("state"))
            .env("XDG_RUNTIME_DIR", root.path().join("runtime"))
            .env("VOXTYPE_TEST_SYSTEM_CONFIG", &system)
            .env("PATH", "")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };

    // Loading gives the defaults, not the system config's F12, and says why.
    let get = run(&["config", "get", "hotkey.key"]);
    let value = String::from_utf8_lossy(&get.stdout);
    assert!(!value.contains("F12"), "loaded the system config: {value}");
    let warning = String::from_utf8_lossy(&get.stderr);
    assert!(
        warning.contains(&link.display().to_string())
            && warning.contains("does not exist")
            && warning.contains("dotfiles/voxtype/config.toml"),
        "{warning}"
    );

    // Saving creates the file the link names and leaves the link alone.
    run(&["config", "set", "hotkey.key", "F13"]);
    assert_eq!(std::fs::read_link(&link).unwrap(), Path::new(LINK));
    assert!(std::fs::read_to_string(&target).unwrap().contains("F13"));

    run(&["history", "pause"]);
    assert_eq!(std::fs::read_link(&link).unwrap(), Path::new(LINK));
    let saved = std::fs::read_to_string(&target).unwrap();
    assert!(
        saved.contains("paused = true") && saved.contains("F13"),
        "{saved}"
    );

    assert_eq!(std::fs::read_to_string(&system).unwrap(), system_text);
    assert_eq!(
        std::fs::read_dir(&etc).unwrap().count(),
        1,
        "nothing may be written beside the system config"
    );
    std::fs::set_permissions(&etc, std::fs::Permissions::from_mode(0o755)).unwrap();
}
