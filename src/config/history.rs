//! `[history]`: keep a record of every dictation.
//!
//! Off unless the user asks for it. New installs get `enabled = true` from
//! the config template; an existing config without the table stays off, so
//! an upgrade never starts writing someone's dictation to disk.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Dictation history configuration.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct HistoryConfig {
    /// Record each dictation to `~/.local/share/voxtype/history/`.
    #[serde(default)]
    pub enabled: bool,

    /// Stop recording until resumed, without turning history off. Paused
    /// dictations are never written, not to the day file and not to the
    /// search index.
    #[serde(default)]
    pub paused: bool,

    /// Days of history to keep. Older day files are deleted at daemon start
    /// and once a day. 0 keeps history forever.
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,

    /// Apps whose dictations are never recorded, matched against the
    /// focused window's app (its class or app ID). Case, spaces, dots and
    /// dashes are ignored and a name matches anywhere in the app, so
    /// "KeePassXC" matches "org.keepassxc.KeePassXC". Accepts a TOML array
    /// or a comma-separated string. Replaces the built-in list when set.
    #[serde(
        default = "default_exclude_apps",
        deserialize_with = "super::meeting_detect::string_list"
    )]
    pub exclude_apps: Vec<String>,

    /// Record which app the text went to.
    #[serde(default = "super::default_true")]
    pub record_app: bool,

    /// Also record the focused window's title. Off by default: titles carry
    /// email subjects, document names and URLs.
    #[serde(default)]
    pub record_window_title: bool,
}

/// The password managers excluded out of the box.
pub const DEFAULT_EXCLUDE_APPS: &[&str] = &["KeePassXC", "Bitwarden", "1Password", "Proton Pass"];

fn default_exclude_apps() -> Vec<String> {
    DEFAULT_EXCLUDE_APPS.iter().map(|s| s.to_string()).collect()
}

fn default_retention_days() -> u32 {
    30
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            paused: false,
            retention_days: default_retention_days(),
            exclude_apps: default_exclude_apps(),
            record_app: true,
            record_window_title: false,
        }
    }
}

/// Letters and digits only, lowercased: "org.keepassxc.KeePassXC" and
/// "Proton Pass" become "orgkeepassxckeepassxc" and "protonpass".
fn fold(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl HistoryConfig {
    /// Whether a dictation should be written at all, before looking at the
    /// app it went to.
    pub fn recording(&self) -> bool {
        self.enabled && !self.paused
    }

    /// The `exclude_apps` entry that `app` matches, if any. An unknown app
    /// (empty) matches nothing.
    pub fn excluded_by(&self, app: &str) -> Option<&str> {
        let app = fold(app);
        if app.is_empty() {
            return None;
        }
        self.exclude_apps
            .iter()
            .find(|name| {
                let name = fold(name);
                !name.is_empty() && app.contains(&name)
            })
            .map(String::as_str)
    }
}

/// History settings given on the daemon's command line. Kept apart from the
/// rest of the config because the daemon re-reads `[history]` from disk when
/// it changes (the tray and `voxtype history pause` write it), and a flag
/// must still win over the file after that.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryOverrides {
    pub enabled: Option<bool>,
    pub paused: Option<bool>,
    pub retention_days: Option<u32>,
    pub exclude_apps: Option<Vec<String>>,
    pub record_app: Option<bool>,
    pub record_window_title: Option<bool>,
}

static CLI_OVERRIDES: OnceLock<HistoryOverrides> = OnceLock::new();

impl HistoryOverrides {
    pub fn apply(&self, config: &mut HistoryConfig) {
        if let Some(v) = self.enabled {
            config.enabled = v;
        }
        if let Some(v) = self.paused {
            config.paused = v;
        }
        if let Some(v) = self.retention_days {
            config.retention_days = v;
        }
        if let Some(v) = &self.exclude_apps {
            config.exclude_apps = v.clone();
        }
        if let Some(v) = self.record_app {
            config.record_app = v;
        }
        if let Some(v) = self.record_window_title {
            config.record_window_title = v;
        }
    }

    /// Remember this process's command-line overrides. Only the first call
    /// counts.
    pub fn set_cli(self) {
        let _ = CLI_OVERRIDES.set(self);
    }

    /// The overrides given on this process's command line.
    pub fn cli() -> HistoryOverrides {
        CLI_OVERRIDES.get().cloned().unwrap_or_default()
    }
}

/// Split a comma-separated app list, as the CLI flag and the environment
/// variable take it.
pub fn parse_app_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Wrap {
        #[serde(default)]
        history: HistoryConfig,
    }

    #[test]
    fn a_missing_table_is_off_with_the_shipped_defaults() {
        let w: Wrap = toml::from_str("").unwrap();
        assert_eq!(w.history, HistoryConfig::default());
        assert!(!w.history.enabled);
        assert!(!w.history.recording());
        assert_eq!(w.history.retention_days, 30);
        assert!(w.history.record_app);
        assert!(!w.history.record_window_title);
        assert_eq!(w.history.exclude_apps.len(), DEFAULT_EXCLUDE_APPS.len());
    }

    #[test]
    fn an_empty_table_is_still_off() {
        let w: Wrap = toml::from_str("[history]\n").unwrap();
        assert!(!w.history.enabled);
    }

    #[test]
    fn paused_stops_recording_without_turning_history_off() {
        let w: Wrap = toml::from_str("[history]\nenabled = true\npaused = true\n").unwrap();
        assert!(w.history.enabled);
        assert!(!w.history.recording());
    }

    #[test]
    fn exclude_apps_takes_an_array_or_a_comma_string() {
        let w: Wrap = toml::from_str("[history]\nexclude_apps = \"Foo, Bar ,\"\n").unwrap();
        assert_eq!(w.history.exclude_apps, vec!["Foo", "Bar"]);
        let w: Wrap = toml::from_str("[history]\nexclude_apps = [\"Foo\"]\n").unwrap();
        assert_eq!(w.history.exclude_apps, vec!["Foo"]);
        let w: Wrap = toml::from_str("[history]\nexclude_apps = []\n").unwrap();
        assert!(w.history.exclude_apps.is_empty());
    }

    #[test]
    fn the_default_list_catches_the_password_managers_by_class() {
        let h = HistoryConfig::default();
        for class in [
            "org.keepassxc.KeePassXC",
            "KeePassXC",
            "Bitwarden",
            "bitwarden",
            "1Password",
            "1password",
            "Proton Pass",
            "proton-pass",
        ] {
            assert!(h.excluded_by(class).is_some(), "{class} should be excluded");
        }
        for class in ["kitty", "firefox", "org.gnome.Nautilus", ""] {
            assert_eq!(h.excluded_by(class), None, "{class} should be recorded");
        }
    }

    #[test]
    fn a_blank_entry_never_excludes_everything() {
        let h = HistoryConfig {
            exclude_apps: vec![" ".into(), "-".into()],
            ..Default::default()
        };
        assert_eq!(h.excluded_by("kitty"), None);
    }

    #[test]
    fn overrides_win_field_by_field() {
        let mut h = HistoryConfig::default();
        HistoryOverrides {
            enabled: Some(true),
            retention_days: Some(0),
            exclude_apps: Some(vec!["Foo".into()]),
            ..Default::default()
        }
        .apply(&mut h);
        assert!(h.enabled);
        assert!(!h.paused);
        assert_eq!(h.retention_days, 0);
        assert_eq!(h.exclude_apps, vec!["Foo"]);
        assert!(h.record_app);
    }

    #[test]
    fn app_lists_split_on_commas() {
        assert_eq!(parse_app_list(" a, b ,,c"), vec!["a", "b", "c"]);
        assert!(parse_app_list("").is_empty());
    }
}
