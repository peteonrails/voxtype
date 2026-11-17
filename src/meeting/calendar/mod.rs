//! A meeting's title from the calendar.
//!
//! When a meeting starts without a real title (none given, the date-only
//! default, or a call detection placeholder such as "Zoom call"), voxtype
//! asks a calendar command what is on now and takes that event's title,
//! and its link when the meeting has none.
//!
//! The command speaks the JSON contract of the Omarchy Next Meeting plugin
//! (peteonrails.next-meeting): one object with `title`, `start_iso`,
//! `minutes_until`, `ongoing`, `url`, `next` (the event after it), `empty`
//! and `error`. It is `[meeting.calendar] command`, or else the plugin's
//! own `agendaCommand` when that plugin is on the Omarchy bar.

pub mod ics;
pub mod source;

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The Omarchy plugin whose contract and command are reused.
pub const PLUGIN_ID: &str = "peteonrails.next-meeting";

/// The plugin's own default command.
pub const PLUGIN_DEFAULT_COMMAND: &str = "agenda --next-json";

/// Titles call detection gives a call it could not name.
const PLACEHOLDERS: &[&str] = &["Zoom call", "Google Meet call", "Teams call"];

/// Whether `title` is one the calendar may replace: none, the date-only
/// default, or a call detection placeholder. A title someone chose is not.
pub fn is_replaceable(title: Option<&str>) -> bool {
    let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) else {
        return true;
    };
    if PLACEHOLDERS.iter().any(|p| p.eq_ignore_ascii_case(title)) {
        return true;
    }
    // "Meeting 2026-11-12 14:00", what an untitled meeting is shown as.
    title
        .strip_prefix("Meeting ")
        .is_some_and(|rest| chrono::NaiveDateTime::parse_from_str(rest, "%Y-%m-%d %H:%M").is_ok())
}

/// One event, as the contract gives it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Event {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub start_iso: Option<String>,
    #[serde(default)]
    pub minutes_until: Option<f64>,
    #[serde(default)]
    pub ongoing: Option<bool>,
    #[serde(default)]
    pub url: Option<String>,
}

/// What the command prints.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Agenda {
    #[serde(flatten)]
    pub event: Event,
    #[serde(default)]
    pub next: Option<Event>,
    #[serde(default)]
    pub empty: bool,
    /// `true`, or, from some providers, the error message itself
    #[serde(default, deserialize_with = "flag_or_message")]
    pub error: bool,
}

/// A boolean, or a string that counts as true when it says anything.
fn flag_or_message<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::String(s) => !s.trim().is_empty(),
        serde_json::Value::Null => false,
        _ => true,
    })
}

/// The calendar event a meeting starting now is.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub title: String,
    pub url: Option<String>,
}

impl Event {
    fn title(&self) -> Option<String> {
        self.title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    }

    /// Starts within `window` minutes either side of now.
    fn near(&self, window: f64) -> bool {
        self.minutes_until.is_some_and(|m| m.abs() <= window)
    }

    /// Running now: said so, or started already (the contract treats a
    /// `minutes_until` at or below zero as running).
    fn running(&self) -> bool {
        self.ongoing == Some(true) || self.minutes_until.is_some_and(|m| m <= 0.0)
    }

    fn found(&self) -> Option<Found> {
        Some(Found {
            title: self.title()?,
            url: self
                .url
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_string),
        })
    }
}

/// The event a meeting starting now is: the first event when it is
/// running or starts within `window_mins` either way, else `next` on the
/// same terms. When the first event is running but started long before
/// and `next` starts within the window, the call is taken to be `next`,
/// as the Next Meeting widget hands over to it.
pub fn pick(agenda: &Agenda, window_mins: u32) -> Option<Found> {
    if agenda.empty || agenda.error {
        return None;
    }
    let window = window_mins as f64;
    let first = &agenda.event;
    let next = agenda.next.as_ref().filter(|n| n.title().is_some());
    if first.title().is_some() && first.near(window) {
        return first.found();
    }
    if let Some(next) = next.filter(|n| n.near(window)) {
        return next.found();
    }
    if first.title().is_some() && first.running() {
        return first.found();
    }
    next.filter(|n| n.running()).and_then(Event::found)
}

/// A command to ask, and a directory to add to its PATH.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub command: String,
    /// The plugin's bundled `bin/`, appended to PATH as the plugin does
    pub extra_path: Option<PathBuf>,
    /// Where it came from, for the log
    pub source: &'static str,
}

/// The command to ask: `configured` when set, else the Next Meeting
/// plugin's `agendaCommand` when the plugin is on the Omarchy bar (read
/// from `<config_dir>/omarchy/shell.json`, never written), else none.
pub fn resolve(configured: &str, config_dir: &Path) -> Option<Command> {
    if !configured.trim().is_empty() {
        return Some(Command {
            command: configured.trim().to_string(),
            extra_path: None,
            source: "[meeting.calendar] command",
        });
    }
    let omarchy = config_dir.join("omarchy");
    let plugin_dir = omarchy.join("plugins").join(PLUGIN_ID);
    if !plugin_dir.is_dir() {
        return None;
    }
    let shell: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(omarchy.join("shell.json")).ok()?).ok()?;
    let widget = find_widget(&shell)?;
    let command = widget
        .get("agendaCommand")
        .and_then(|c| c.as_str())
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .or_else(|| plugin_default(&plugin_dir))
        .unwrap_or_else(|| PLUGIN_DEFAULT_COMMAND.to_string());
    let bin = plugin_dir.join("bin");
    Some(Command {
        command,
        extra_path: bin.is_dir().then_some(bin),
        source: "the Next Meeting plugin",
    })
}

/// The plugin's entry on the bar, wherever the layout puts it.
fn find_widget(v: &serde_json::Value) -> Option<&serde_json::Value> {
    match v {
        serde_json::Value::Object(map) => {
            if map.get("id").and_then(|i| i.as_str()) == Some(PLUGIN_ID) {
                return Some(v);
            }
            map.values().find_map(find_widget)
        }
        serde_json::Value::Array(items) => items.iter().find_map(find_widget),
        _ => None,
    }
}

/// `agendaCommand` from the plugin manifest's defaults.
fn plugin_default(plugin_dir: &Path) -> Option<String> {
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(plugin_dir.join("manifest.json")).ok()?)
            .ok()?;
    manifest
        .pointer("/barWidget/defaults/agendaCommand")
        .and_then(|c| c.as_str())
        .map(str::to_string)
}

/// Why asking the calendar gave nothing. Each is logged at debug and
/// leaves the title alone.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CalendarError {
    #[error("the calendar command could not start: {0}")]
    Start(String),
    #[error("the calendar command ran past {0} s")]
    TimedOut(u64),
    #[error("the calendar command failed ({0})")]
    Failed(String),
    #[error("the calendar command printed something other than the calendar JSON: {0}")]
    BadJson(String),
}

/// Run `command` through `bash -lc`, as the Next Meeting widget does, with
/// ~/.local/bin and ~/bin first on PATH and the plugin's `bin/` last, and
/// read its agenda. Killed, with what it started, after `timeout`.
pub async fn ask(command: &Command, timeout: Duration) -> Result<Agenda, CalendarError> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut path = format!(
        "{home}/.local/bin:{home}/bin:{}",
        std::env::var("PATH").unwrap_or_default()
    );
    if let Some(extra) = &command.extra_path {
        path.push(':');
        path.push_str(&extra.display().to_string());
    }
    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg("-lc")
        .arg(&command.command)
        .env("PATH", path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| CalendarError::Start(e.to_string()))?;
    let pid = child.id();
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return Err(CalendarError::Start(e.to_string())),
        Err(_) => {
            if let Some(pid) = pid {
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(pid as i32),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            return Err(CalendarError::TimedOut(timeout.as_secs()));
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CalendarError::Failed(if stderr.is_empty() {
            output.status.to_string()
        } else {
            stderr.chars().take(200).collect()
        }));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<Agenda>(stdout.trim()).map_err(|e| CalendarError::BadJson(e.to_string()))
}

/// Ask the calendar and pick the event a meeting starting now is.
pub async fn lookup(
    command: &Command,
    timeout: Duration,
    window_mins: u32,
) -> Result<Option<Found>, CalendarError> {
    Ok(pick(&ask(command, timeout).await?, window_mins))
}

/// A title for a meeting starting now, and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Titled {
    pub title: String,
    pub url: Option<String>,
    /// "the calendar link", "[meeting.calendar] command" or "the Next
    /// Meeting plugin"
    pub source: &'static str,
}

/// Seconds a fetch of the calendar link may take.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything a lookup reads, so tests can point it elsewhere.
#[derive(Debug, Clone)]
pub struct Lookup {
    pub settings: crate::config::MeetingCalendarConfig,
    pub store: source::Store,
    /// The XDG config directory (`~/.config`), where Omarchy's plugins are
    pub config_dir: Option<std::path::PathBuf>,
    pub now: chrono::DateTime<chrono::Utc>,
}

impl Lookup {
    /// Today's settings and places.
    pub fn current(settings: &crate::config::MeetingCalendarConfig) -> Self {
        Self {
            settings: settings.clone(),
            store: source::Store::new(source::Store::default_dir()),
            config_dir: crate::config::Config::config_dir()
                .and_then(|voxtype| voxtype.parent().map(Path::to_path_buf)),
            now: chrono::Utc::now(),
        }
    }

    /// The command to ask: configured, else the plugin's.
    pub fn command(&self) -> Option<Command> {
        resolve(&self.settings.command, self.config_dir.as_deref()?)
    }

    /// The calendar's name for a meeting starting now: from the calendar
    /// link, then the command, then the Next Meeting plugin, each tried
    /// only when the one before has nothing. Every miss and failure is
    /// logged at debug, never with the link.
    pub async fn title(&self) -> Option<Titled> {
        if self.store.settings().is_some() {
            match self.title_from_link().await {
                Ok(Some(t)) => return Some(t),
                Ok(None) => tracing::debug!(
                    "No event in the calendar link within {} min of the meeting's start",
                    self.settings.window_mins
                ),
                Err(e) => tracing::debug!("No title from the calendar link: {}", e),
            }
        }
        let command = self.command()?;
        let timeout = Duration::from_secs(self.settings.timeout_secs.max(1));
        match lookup(&command, timeout, self.settings.window_mins).await {
            Ok(Some(found)) => Some(Titled {
                title: found.title,
                url: found.url,
                source: command.source,
            }),
            Ok(None) => {
                tracing::debug!(
                    "No calendar event within {} min of the meeting's start ({})",
                    self.settings.window_mins,
                    command.source
                );
                None
            }
            Err(e) => {
                tracing::debug!("No title from the calendar ({}): {}", command.source, e);
                None
            }
        }
    }

    /// From the events cached from the link, fetched now when there are
    /// none yet.
    async fn title_from_link(&self) -> Result<Option<Titled>, source::IcsError> {
        let cached = match self.store.cached() {
            Some(c) => c,
            None => {
                let store = self.store.clone();
                let now = self.now;
                tokio::task::spawn_blocking(move || store.refresh(now, FETCH_TIMEOUT))
                    .await
                    .map_err(|e| source::IcsError::Io(e.to_string()))??;
                self.store.cached().unwrap_or_default()
            }
        };
        Ok(
            ics::pick(&cached.events, self.now, self.settings.window_mins).map(|e| Titled {
                title: e.title.clone(),
                url: e.url.clone(),
                source: "the calendar link",
            }),
        )
    }
}

/// Keep the calendar link's events fresh: fetch when the daemon starts,
/// then every [`source::REFRESH_EVERY`], and soon after a new link is set.
/// Runs for the daemon's life; does nothing while no link is set.
pub async fn refresh_forever(store: source::Store) {
    let mut last_error: Option<String> = None;
    let mut retry_at = chrono::Utc::now();
    loop {
        let now = chrono::Utc::now();
        if store.settings().is_some() && store.stale(now) && now >= retry_at {
            let s = store.clone();
            let result = tokio::task::spawn_blocking(move || s.refresh(now, FETCH_TIMEOUT)).await;
            match result {
                Ok(Ok(n)) => {
                    if last_error.take().is_some() {
                        tracing::info!("Calendar link reachable again");
                    }
                    tracing::debug!("Calendar link fetched: {} events in the next 48 hours", n);
                }
                Ok(Err(e)) => {
                    // The message names the host, never the link.
                    let text = e.to_string();
                    if last_error.as_deref() != Some(text.as_str()) {
                        tracing::info!("Calendar link not fetched: {}", text);
                    }
                    last_error = Some(text);
                    retry_at = now + chrono::Duration::minutes(5);
                }
                Err(e) => tracing::debug!("Calendar fetch task failed: {}", e),
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agenda(json: &str) -> Agenda {
        serde_json::from_str(json).unwrap()
    }

    fn found(title: &str, url: Option<&str>) -> Option<Found> {
        Some(Found {
            title: title.into(),
            url: url.map(Into::into),
        })
    }

    #[test]
    fn what_the_calendar_may_replace() {
        assert!(is_replaceable(None));
        assert!(is_replaceable(Some("  ")));
        assert!(is_replaceable(Some("Zoom call")));
        assert!(is_replaceable(Some("Google Meet call")));
        assert!(is_replaceable(Some("teams call")));
        assert!(is_replaceable(Some("Meeting 2026-11-12 14:00")));
        assert!(!is_replaceable(Some("Pete <> Chad")));
        assert!(!is_replaceable(Some("Meeting about the roadmap")));
        assert!(!is_replaceable(Some("Slack call")));
    }

    #[test]
    fn an_ongoing_event_is_the_meeting() {
        let a = agenda(
            r#"{"title":"Design review","ongoing":true,"minutes_until":-35,"url":"https://meet/x"}"#,
        );
        assert_eq!(pick(&a, 10), found("Design review", Some("https://meet/x")));
        // The contract treats a start already passed as running.
        let a = agenda(r#"{"title":"Standup","minutes_until":-3}"#);
        assert_eq!(pick(&a, 10), found("Standup", None));
    }

    #[test]
    fn an_event_within_the_window_either_way_is_the_meeting() {
        let a = agenda(r#"{"title":"Standup","minutes_until":8,"ongoing":false}"#);
        assert_eq!(pick(&a, 10), found("Standup", None));
        let a = agenda(r#"{"title":"Standup","minutes_until":10}"#);
        assert_eq!(pick(&a, 10), found("Standup", None));
        let a = agenda(r#"{"title":"Standup","minutes_until":11}"#);
        assert_eq!(pick(&a, 10), None, "outside the window");
        assert_eq!(pick(&a, 15), found("Standup", None));
    }

    #[test]
    fn next_counts_on_the_same_terms() {
        // Running long, with the next one about to start: the call is next.
        let a = agenda(
            r#"{"title":"All day offsite","ongoing":true,"minutes_until":-240,
                "next":{"title":"Pete <> Chad Bi-Weekly","minutes_until":4}}"#,
        );
        assert_eq!(pick(&a, 10), found("Pete <> Chad Bi-Weekly", None));
        // Just started, with the next further out: still the first.
        let a = agenda(
            r#"{"title":"Design review","ongoing":true,"minutes_until":-2,
                "next":{"title":"Retro","minutes_until":48}}"#,
        );
        assert_eq!(pick(&a, 10), found("Design review", None));
        // First too far away, next within the window.
        let a = agenda(
            r#"{"title":"Lunch","minutes_until":-90,"ongoing":false,
                "next":{"title":"Standup","minutes_until":-1}}"#,
        );
        assert_eq!(pick(&a, 10), found("Standup", None));
        // Neither near nor running.
        let a = agenda(
            r#"{"title":"Retro","minutes_until":120,"next":{"title":"Later","minutes_until":300}}"#,
        );
        assert_eq!(pick(&a, 10), None);
    }

    #[test]
    fn empty_error_and_untitled_agendas_give_nothing() {
        assert_eq!(pick(&agenda(r#"{"empty":true}"#), 10), None);
        assert_eq!(
            pick(&agenda(r#"{"error":true,"title":"x","ongoing":true}"#), 10),
            None
        );
        assert_eq!(pick(&agenda(r#"{"title":"  ","ongoing":true}"#), 10), None);
        assert_eq!(pick(&agenda(r#"{}"#), 10), None);
        // A provider that puts its error message in `error`.
        let a = agenda(r#"{"error":"Traceback (most recent call last): ...","title":"x"}"#);
        assert!(a.error);
        assert_eq!(pick(&a, 10), None);
        assert!(!agenda(r#"{"error":"","title":"x"}"#).error);
    }

    fn plugin(config: &Path, shell: &str, manifest_default: Option<&str>) {
        let dir = config.join("omarchy/plugins").join(PLUGIN_ID);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        if let Some(cmd) = manifest_default {
            std::fs::write(
                dir.join("manifest.json"),
                serde_json::json!({"barWidget": {"defaults": {"agendaCommand": cmd}}}).to_string(),
            )
            .unwrap();
        }
        std::fs::write(config.join("omarchy/shell.json"), shell).unwrap();
    }

    #[test]
    fn the_plugins_command_is_found_on_the_bar() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path();
        assert_eq!(resolve("", config), None, "no plugin");

        plugin(
            config,
            r#"{"bar":{"layout":{"right":[{"id":"clock"},
                {"id":"peteonrails.next-meeting","agendaCommand":"$HOME/bin/cal --json"}]}}}"#,
            Some("agenda --next-json"),
        );
        let found = resolve("", config).unwrap();
        assert_eq!(found.command, "$HOME/bin/cal --json");
        assert_eq!(
            found.extra_path,
            Some(config.join("omarchy/plugins").join(PLUGIN_ID).join("bin"))
        );

        // On the bar with nothing set: the plugin's default.
        plugin(
            config,
            r#"{"bar":{"layout":{"center":[{"id":"peteonrails.next-meeting","joinCommand":"x"}]}}}"#,
            Some("agenda --next-json --custom"),
        );
        assert_eq!(
            resolve("", config).unwrap().command,
            "agenda --next-json --custom"
        );

        // Installed but not on the bar: off.
        plugin(
            config,
            r#"{"bar":{"layout":{"right":[{"id":"clock"}]}}}"#,
            None,
        );
        assert_eq!(resolve("", config), None);

        // Configured wins over everything.
        let own = resolve("my-cal --json", config).unwrap();
        assert_eq!(
            (own.command.as_str(), own.extra_path),
            ("my-cal --json", None)
        );
    }

    fn command(script: &str) -> Command {
        Command {
            command: script.into(),
            extra_path: None,
            source: "test",
        }
    }

    #[tokio::test]
    async fn the_command_is_asked_and_its_failures_named() {
        let t = Duration::from_secs(3);
        let got = lookup(
            &command(r#"echo '{"title":"Standup","ongoing":true,"url":"https://z/1"}'"#),
            t,
            10,
        )
        .await
        .unwrap();
        assert_eq!(got, found("Standup", Some("https://z/1")));

        let bad = ask(&command("echo not json"), t).await.unwrap_err();
        assert!(matches!(bad, CalendarError::BadJson(_)), "{bad}");
        let failed = ask(&command("echo boom >&2; exit 2"), t).await.unwrap_err();
        assert_eq!(failed, CalendarError::Failed("boom".into()));
        let missing = ask(&command("no-such-calendar-command-xyz"), t)
            .await
            .unwrap_err();
        assert!(matches!(missing, CalendarError::Failed(_)), "{missing}");
        let slow = ask(&command("sleep 5"), Duration::from_millis(300))
            .await
            .unwrap_err();
        assert_eq!(slow, CalendarError::TimedOut(0));
        let empty = lookup(&command(r#"echo '{"empty":true}'"#), t, 10)
            .await
            .unwrap();
        assert_eq!(empty, None);
    }

    #[tokio::test]
    async fn the_plugins_bin_is_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // A name nothing else on this machine's PATH can have, since the
        // plugin's bin comes last.
        crate::test_support::write_script(
            &bin.join("voxtype-test-agenda-8c1f"),
            "#!/bin/sh\necho '{\"title\":\"From the plugin\",\"ongoing\":true}'\n",
        );
        let cmd = Command {
            command: "voxtype-test-agenda-8c1f --next-json".into(),
            extra_path: Some(bin),
            source: "test",
        };
        assert_eq!(
            lookup(&cmd, Duration::from_secs(3), 10).await.unwrap(),
            found("From the plugin", None)
        );
    }

    fn lookup_in(dir: &Path, command: &str, now: &str) -> Lookup {
        Lookup {
            settings: crate::config::MeetingCalendarConfig {
                command: command.into(),
                window_mins: 10,
                timeout_secs: 3,
            },
            store: source::Store::new(dir.join("state")),
            config_dir: Some(dir.join("config")),
            now: chrono::DateTime::parse_from_rfc3339(now)
                .unwrap()
                .with_timezone(&chrono::Utc),
        }
    }

    /// A link set and its events cached, as the daemon's refresh leaves it.
    fn cache_fixture(store: &source::Store, fetched: &str) {
        let at = chrono::DateTime::parse_from_rfc3339(fetched)
            .unwrap()
            .with_timezone(&chrono::Utc);
        store
            .set_link("https://calendar.example.com/private-secret/basic.ics")
            .unwrap();
        let events = ics::events_between(
            ics::tests::FIXTURE,
            at - chrono::Duration::days(1),
            at + source::AHEAD,
        )
        .unwrap();
        std::fs::write(
            store.dir.join(source::EVENTS_FILE),
            serde_json::to_string(&source::Cached {
                fetched_at: at.timestamp(),
                events,
            })
            .unwrap(),
        )
        .unwrap();
    }

    const COMMAND: &str = r#"echo '{"title":"From the command","ongoing":true}'"#;

    #[tokio::test]
    async fn the_calendar_link_comes_first_then_the_command_then_the_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let at = "2026-11-12T13:55:00Z";

        // Nothing set up: no title.
        assert_eq!(lookup_in(dir.path(), "", at).title().await, None);

        // The plugin on the bar.
        plugin(
            &dir.path().join("config"),
            r#"{"bar":{"layout":{"right":[{"id":"peteonrails.next-meeting","agendaCommand":"echo '{\"title\":\"From the plugin\",\"ongoing\":true}'"}]}}}"#,
            None,
        );
        let t = lookup_in(dir.path(), "", at).title().await.unwrap();
        assert_eq!(
            (t.title.as_str(), t.source),
            ("From the plugin", "the Next Meeting plugin")
        );

        // A configured command beats the plugin.
        let t = lookup_in(dir.path(), COMMAND, at).title().await.unwrap();
        assert_eq!(
            (t.title.as_str(), t.source),
            ("From the command", "[meeting.calendar] command")
        );

        // The calendar link beats both, with the event's meeting link.
        let l = lookup_in(dir.path(), COMMAND, at);
        cache_fixture(&l.store, at);
        let t = l.title().await.unwrap();
        assert_eq!(t.title, "Design review, with NY");
        assert_eq!(t.source, "the calendar link");
        assert_eq!(
            t.url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );

        // No event near the start in the link: the command answers.
        let l = lookup_in(dir.path(), COMMAND, "2026-11-12T16:00:00Z");
        let t = l.title().await.unwrap();
        assert_eq!(t.source, "[meeting.calendar] command");
    }

    #[tokio::test]
    async fn an_unreachable_link_falls_through_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let l = lookup_in(dir.path(), COMMAND, "2026-11-12T13:55:00Z");
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        l.store
            .set_link(&format!("http://127.0.0.1:{port}/private-secret/basic.ics"))
            .unwrap();
        let t = l.title().await.unwrap();
        assert_eq!(t.source, "[meeting.calendar] command");
        let recorded = l.store.settings().unwrap().last_error.unwrap();
        assert!(
            recorded.contains("127.0.0.1") && !recorded.contains("private-secret"),
            "{recorded}"
        );
    }
}
