//! `[meeting.detect]`: notice when a call starts and offer to record it.

use serde::{Deserialize, Deserializer, Serialize};

/// Meeting detection configuration.
///
/// Detection only suggests: it posts a notification with a Record button and
/// never starts a meeting on its own, except for apps listed in
/// `auto_record_apps`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingDetectConfig {
    /// Watch for calls and offer to record them. Unset means "on for
    /// Omarchy, off elsewhere", so an upgrade on other desktops changes
    /// nothing. Only takes effect while `[meeting] enabled = true`.
    #[serde(default)]
    pub enabled: Option<bool>,

    /// Apps (by `name` from `apps`) whose calls are recorded without asking.
    /// Accepts a TOML array or a comma-separated string.
    #[serde(default, deserialize_with = "string_list")]
    pub auto_record_apps: Vec<String>,

    /// Seconds a call app must hold the mic before voxtype prompts, and a
    /// call must show no sign of itself before it counts as over. Filters
    /// out brief mic opens such as a permission check.
    #[serde(default = "default_debounce_secs")]
    pub debounce_secs: u32,

    /// The call apps to watch. Replaces the built-in list when set.
    #[serde(default = "default_apps")]
    pub apps: Vec<CallAppPattern>,

    /// What happens when a detected call ends while a meeting records:
    /// "stop" stops it after `stop_grace_secs` (with a Keep recording
    /// button), "ask" offers to stop it, "keep" leaves it running. A meeting
    /// started by hand with no detected call is never stopped.
    ///
    /// A browser call (an app with `titles`) has ended when the browser has
    /// released the mic and no window title shows the call; a native
    /// client's when it releases the mic.
    #[serde(default = "default_on_call_end")]
    pub on_call_end: String,

    /// Seconds between "Call ended" and the automatic stop, for Keep
    /// recording. A call seen again in that time is the same call: the stop
    /// is called off and it is not offered a second time.
    #[serde(default = "default_stop_grace_secs")]
    pub stop_grace_secs: u32,
}

/// `on_call_end`, parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallEnd {
    Stop,
    Ask,
    Keep,
}

pub const CALL_END_CHOICES: &[&str] = &["stop", "ask", "keep"];

impl CallEnd {
    /// Unknown values read as the default, stop.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "ask" => Self::Ask,
            "keep" => Self::Keep,
            _ => Self::Stop,
        }
    }
}

fn default_on_call_end() -> String {
    "stop".to_string()
}

fn default_stop_grace_secs() -> u32 {
    20
}

/// One call app: which mic streams belong to it and, for apps that run in a
/// browser, which window titles confirm that a call is in progress.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CallAppPattern {
    /// Display name, used in the prompt and in `auto_record_apps`.
    pub name: String,

    /// Case-insensitive substrings matched against a PipeWire capture
    /// stream's `application.name`, `application.process.binary` and
    /// `node.name`.
    #[serde(default, deserialize_with = "string_list")]
    pub streams: Vec<String>,

    /// Case-insensitive substrings of a window title that confirm the call.
    /// Empty means the mic stream alone is enough. Browsers need this,
    /// since a browser opens the mic for many things besides calls.
    #[serde(default, deserialize_with = "string_list")]
    pub titles: Vec<String>,
}

fn default_debounce_secs() -> u32 {
    5
}

const BROWSERS: &[&str] = &[
    "chromium",
    "chrome",
    "firefox",
    "brave",
    "vivaldi",
    "microsoft-edge",
    "msedge",
    "zen",
    "librewolf",
];

fn pattern(name: &str, streams: &[&str], titles: &[&str]) -> CallAppPattern {
    CallAppPattern {
        name: name.to_string(),
        streams: streams.iter().map(|s| s.to_string()).collect(),
        titles: titles.iter().map(|s| s.to_string()).collect(),
    }
}

/// The built-in call apps. Native clients count on the mic alone; browser
/// calls need a matching window title as well.
pub fn default_apps() -> Vec<CallAppPattern> {
    vec![
        // Zoom's capture stream is "ZOOM VoiceEngine" from the "zoom" binary.
        pattern("Zoom", &["zoom"], &[]),
        pattern("Microsoft Teams", &["teams-for-linux", "teams"], &[]),
        pattern("Google Meet", BROWSERS, &["Meet - ", "Google Meet"]),
        pattern("Microsoft Teams (web)", BROWSERS, &["| Microsoft Teams"]),
        pattern("Zoom (web)", BROWSERS, &["Zoom Meeting", "Zoom Workplace"]),
    ]
}

impl Default for MeetingDetectConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            auto_record_apps: Vec::new(),
            debounce_secs: default_debounce_secs(),
            apps: default_apps(),
            on_call_end: default_on_call_end(),
            stop_grace_secs: default_stop_grace_secs(),
        }
    }
}

impl MeetingDetectConfig {
    /// Whether detection is on, resolving an unset `enabled` against the
    /// desktop: on for Omarchy, off everywhere else.
    pub fn call_end(&self) -> CallEnd {
        CallEnd::parse(&self.on_call_end)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
            .unwrap_or_else(|| crate::setup::omarchy::Context::from_env().is_omarchy())
    }
}

/// A list given either as a TOML array or as one comma-separated string. The
/// string form is what `voxtype config set` writes.
pub(super) fn string_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Form {
        One(String),
        Many(Vec<String>),
    }
    // Only the comma form is trimmed: a pattern in an array keeps its exact
    // spacing ("Meet - " must not match "Meet -ing notes").
    Ok(match Form::deserialize(d)? {
        Form::One(s) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Form::Many(v) => v.into_iter().filter(|s| !s.trim().is_empty()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Wrap {
        #[serde(default)]
        detect: MeetingDetectConfig,
    }

    #[test]
    fn defaults_leave_enabled_unset_and_ship_builtin_apps() {
        let c = MeetingDetectConfig::default();
        assert_eq!(c.enabled, None);
        assert!(c.auto_record_apps.is_empty());
        assert_eq!(c.debounce_secs, 5);
        let names: Vec<_> = c.apps.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"Zoom"));
        assert!(names.contains(&"Google Meet"));
        assert!(names.contains(&"Microsoft Teams"));
    }

    #[test]
    fn auto_record_apps_accepts_array_or_comma_string() {
        let w: Wrap = toml::from_str("[detect]\nauto_record_apps = [\"Zoom\", \"\"]").unwrap();
        assert_eq!(w.detect.auto_record_apps, vec!["Zoom"]);
        let w: Wrap =
            toml::from_str("[detect]\nauto_record_apps = \"Zoom, Google Meet,\"").unwrap();
        assert_eq!(w.detect.auto_record_apps, vec!["Zoom", "Google Meet"]);
    }

    #[test]
    fn user_apps_replace_the_builtins() {
        let w: Wrap = toml::from_str(
            "[[detect.apps]]\nname = \"Jitsi\"\nstreams = [\"firefox\"]\ntitles = \"Jitsi Meet\"\n",
        )
        .unwrap();
        assert_eq!(
            w.detect.apps,
            vec![pattern("Jitsi", &["firefox"], &["Jitsi Meet"])]
        );
        assert_eq!(w.detect.enabled, None);
    }

    #[test]
    fn explicit_enabled_wins_over_the_desktop() {
        let w: Wrap = toml::from_str("[detect]\nenabled = false").unwrap();
        assert!(!w.detect.is_enabled());
        let w: Wrap = toml::from_str("[detect]\nenabled = true").unwrap();
        assert!(w.detect.is_enabled());
    }

    #[test]
    fn call_end_defaults_to_stop_after_20_seconds() {
        let w: Wrap = toml::from_str("").unwrap();
        assert_eq!(w.detect.call_end(), CallEnd::Stop);
        assert_eq!(w.detect.stop_grace_secs, 20);
        let w: Wrap =
            toml::from_str("[detect]\non_call_end = \"ask\"\nstop_grace_secs = 5").unwrap();
        assert_eq!(w.detect.call_end(), CallEnd::Ask);
        assert_eq!(w.detect.stop_grace_secs, 5);
        assert_eq!(CallEnd::parse("KEEP"), CallEnd::Keep);
        assert_eq!(CallEnd::parse("bogus"), CallEnd::Stop);
    }
}
