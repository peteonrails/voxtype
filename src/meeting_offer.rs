//! The offer to turn meeting mode on, for installs that predate it.
//!
//! New installs get `[meeting] enabled = true` from the config template.
//! Existing configs keep meeting mode off, so instead the tray shows a
//! "Turn on meetings..." entry and, at most once a day on daemon start, a
//! notification with Turn on, Not now and "No, don't ask again". Users who
//! already record meetings with Meeting Recorder get the notification on the
//! first start, worded for them. Nothing is ever enabled without a click.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where the user's answers are kept, under `$XDG_STATE_HOME/voxtype`.
pub const STATE_FILE: &str = "meeting_offer.json";

/// The notification is shown at most this often.
pub const REOFFER_SECS: u64 = 24 * 60 * 60;

/// The binary and package-name prefix of omarchy-meeting-recorder. The
/// package may be `omarchy-meeting-recorder` or `-bin`; both start with
/// this, followed by `-<version>` in pacman's local database.
const MEETING_RECORDER: &str = "omarchy-meeting-recorder";

/// What the user has said so far.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferState {
    /// "No, don't ask again". The offer is gone for good.
    #[serde(default)]
    pub never: bool,
    /// When the offer first applied (unix seconds).
    #[serde(default)]
    pub first_seen: Option<u64>,
    /// When the notification was last shown (unix seconds).
    #[serde(default)]
    pub last_notified: Option<u64>,
    /// The Meeting Recorder notification has been shown once.
    #[serde(default)]
    pub recorder_notified: bool,
}

impl OfferState {
    pub fn path() -> PathBuf {
        crate::config::Config::state_dir().join(STATE_FILE)
    }

    /// The saved state, or a fresh one if there is none or it is unreadable.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)
    }
}

/// Which wording the notification uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferKind {
    Plain,
    /// The user has omarchy-meeting-recorder installed.
    MeetingRecorder,
}

/// What to show on this daemon start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Decision {
    /// Show "Turn on meetings..." in the tray.
    pub show_entry: bool,
    /// Post the notification now.
    pub notify: Option<OfferKind>,
}

/// Decide on daemon start, and record a notification in `state` when one is
/// due. The caller saves `state` afterwards.
///
/// The tray entry shows whenever the offer applies. The notification comes
/// on the first start for Meeting Recorder users; for everyone else it
/// waits a day after the offer first applied, so an upgrade does not greet
/// every user with a prompt. After that it comes back at most once a day
/// until the user turns meetings on or says not to ask again.
pub fn decide(
    meeting_enabled: bool,
    offer_enabled: bool,
    recorder_installed: bool,
    state: &mut OfferState,
    now: u64,
) -> Decision {
    if meeting_enabled || !offer_enabled || state.never {
        return Decision::default();
    }
    let first_seen = *state.first_seen.get_or_insert(now);
    let notify = if recorder_installed && !state.recorder_notified {
        Some(OfferKind::MeetingRecorder)
    } else {
        let waited = now.saturating_sub(first_seen) >= REOFFER_SECS;
        let rested = state
            .last_notified
            .is_none_or(|t| now.saturating_sub(t) >= REOFFER_SECS);
        (waited && rested).then_some(if recorder_installed {
            OfferKind::MeetingRecorder
        } else {
            OfferKind::Plain
        })
    };
    if let Some(kind) = notify {
        state.last_notified = Some(now);
        if kind == OfferKind::MeetingRecorder {
            state.recorder_notified = true;
        }
    }
    Decision {
        show_entry: true,
        notify,
    }
}

/// The user's answer to the notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    TurnOn,
    NotNow,
    Never,
}

pub const ACTION_TURN_ON: &str = "turn-on";
pub const ACTION_NOT_NOW: &str = "not-now";
pub const ACTION_NEVER: &str = "never";

impl Answer {
    /// Map a notification action key. A click on the body means Turn on;
    /// closing the notification without choosing means Not now.
    pub fn from_action(key: &str) -> Self {
        match key {
            ACTION_TURN_ON | "default" => Self::TurnOn,
            ACTION_NEVER => Self::Never,
            _ => Self::NotNow,
        }
    }
}

/// Record an answer. Not now needs nothing: the notification time is
/// already saved, and the next start re-offers once a day has passed.
pub fn apply_answer(state: &mut OfferState, answer: Answer) {
    if answer == Answer::Never {
        state.never = true;
    }
}

/// Load, decide and save in one step, on daemon start.
pub fn on_daemon_start(
    path: &Path,
    meeting_enabled: bool,
    offer_enabled: bool,
    recorder_installed: bool,
    now: u64,
) -> Decision {
    let mut state = OfferState::load(path);
    let before = state.clone();
    let decision = decide(
        meeting_enabled,
        offer_enabled,
        recorder_installed,
        &mut state,
        now,
    );
    if state != before {
        if let Err(e) = state.save(path) {
            tracing::warn!("Could not save the meeting offer state: {}", e);
        }
    }
    decision
}

/// Load, record an answer and save.
pub fn record_answer(path: &Path, answer: Answer) {
    let mut state = OfferState::load(path);
    apply_answer(&mut state, answer);
    if let Err(e) = state.save(path) {
        tracing::warn!("Could not save the meeting offer answer: {}", e);
    }
}

/// The notification text: summary and body.
pub fn wording(kind: OfferKind) -> (&'static str, &'static str) {
    match kind {
        OfferKind::Plain => (
            "Turn on Voxtype meetings?",
            "Voxtype can transcribe your meetings live, with your side and the \
             other side of the call labelled. Turn on meeting mode?",
        ),
        OfferKind::MeetingRecorder => (
            "Turn on Voxtype meetings?",
            "You record meetings with Meeting Recorder. Voxtype can transcribe \
             meetings live too, with your side and the other side of the call \
             labelled. Turn on meeting mode?",
        ),
    }
}

/// The notification's actions, `(key, label)`.
pub fn actions() -> [(&'static str, &'static str); 3] {
    [
        (ACTION_TURN_ON, "Turn on"),
        (ACTION_NOT_NOW, "Not now"),
        (ACTION_NEVER, "No, don't ask again"),
    ]
}

/// Whether omarchy-meeting-recorder is installed: its binary on `path_var`,
/// or its package in pacman's local database. Both are a directory read; no
/// process is run.
pub fn recorder_installed(path_var: Option<&std::ffi::OsStr>, pacman_local: &Path) -> bool {
    let on_path = path_var
        .map(|p| std::env::split_paths(p).any(|dir| dir.join(MEETING_RECORDER).is_file()))
        .unwrap_or(false);
    on_path
        || std::fs::read_dir(pacman_local)
            .map(|entries| {
                entries.filter_map(Result::ok).any(|e| {
                    e.file_name()
                        .to_str()
                        .and_then(|n| n.strip_prefix(MEETING_RECORDER))
                        .is_some_and(|rest| rest.starts_with('-'))
                })
            })
            .unwrap_or(false)
}

/// Probe the real system.
pub fn recorder_installed_here() -> bool {
    recorder_installed(
        std::env::var_os("PATH").as_deref(),
        Path::new("/var/lib/pacman/local"),
    )
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// How turning meetings on ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Enabled {
    /// Saved, and the daemon is restarting to pick it up.
    Restarting,
    /// Saved, but the daemon could not be restarted here (not run by
    /// systemd, or busy). The user is told to restart it.
    RestartNeeded(String),
}

/// Turns meeting mode on the way a user would: `voxtype config set
/// meeting.enabled true`, then the restart every "restart required" key
/// asks for, since the daemon reads meeting.enabled only at startup.
pub struct Enabler {
    /// The voxtype binary.
    pub exe: PathBuf,
    /// The config the daemon runs on, when given with --config.
    pub config_path: Option<PathBuf>,
    /// The user's config path, and the system config it falls back to.
    pub user_config: Option<PathBuf>,
    pub system_config: PathBuf,
    pub systemctl: PathBuf,
}

impl Enabler {
    pub fn for_daemon(config_path: Option<PathBuf>) -> anyhow::Result<Self> {
        Ok(Self {
            exe: std::env::current_exe()?,
            config_path,
            user_config: crate::config::Config::default_path(),
            system_config: crate::config::Config::system_path(),
            systemctl: PathBuf::from("systemctl"),
        })
    }

    /// The file to write. A user running on the system config gets a copy
    /// of it as their own first, so turning meetings on changes only that
    /// one setting; the system file is not theirs to write.
    fn target(&self) -> anyhow::Result<PathBuf> {
        if let Some(path) = &self.config_path {
            return Ok(path.clone());
        }
        let user = self
            .user_config
            .clone()
            .ok_or_else(|| anyhow::anyhow!("cannot find the user config directory"))?;
        if !user.exists() && self.system_config.is_file() {
            if let Some(dir) = user.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::copy(&self.system_config, &user)?;
        }
        Ok(user)
    }

    pub async fn enable(&self) -> anyhow::Result<Enabled> {
        let target = self.target()?;
        let out = tokio::process::Command::new(&self.exe)
            .arg("--config")
            .arg(&target)
            .args(["config", "set", "meeting.enabled", "true"])
            .stdin(std::process::Stdio::null())
            .output()
            .await?;
        if !out.status.success() {
            anyhow::bail!(
                "voxtype config set meeting.enabled true failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let active = tokio::process::Command::new(&self.systemctl)
            .args(["--user", "is-active", "voxtype"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .await
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
            .unwrap_or(false);
        if !active {
            return Ok(Enabled::RestartNeeded(
                "Voxtype is not running as the voxtype systemd service".into(),
            ));
        }
        // --no-block: the restart stops this very process, so queue the job
        // and return instead of waiting on our own shutdown.
        let restarted = tokio::process::Command::new(&self.systemctl)
            .args(["--user", "--no-block", "restart", "voxtype"])
            .stdin(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);
        Ok(if restarted {
            Enabled::Restarting
        } else {
            Enabled::RestartNeeded("systemctl --user restart voxtype failed".into())
        })
    }
}

/// The notification for a saved setting the daemon could not pick up.
pub fn restart_needed_wording() -> (&'static str, &'static str) {
    (
        "Meetings are on",
        "Restart Voxtype to start using them: systemctl --user restart voxtype",
    )
}

#[cfg(target_os = "linux")]
pub mod notify;

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = REOFFER_SECS;
    const T0: u64 = 1_800_000_000;

    #[test]
    fn nothing_when_meetings_are_on_or_the_offer_is_off_or_refused() {
        let mut s = OfferState::default();
        assert_eq!(decide(true, true, true, &mut s, T0), Decision::default());
        assert_eq!(decide(false, false, true, &mut s, T0), Decision::default());
        let mut never = OfferState {
            never: true,
            ..Default::default()
        };
        assert_eq!(
            decide(false, true, true, &mut never, T0),
            Decision::default()
        );
        assert_eq!(s, OfferState::default(), "nothing recorded");
    }

    #[test]
    fn plain_offer_shows_the_entry_first_and_notifies_a_day_later() {
        let mut s = OfferState::default();
        let first = decide(false, true, false, &mut s, T0);
        assert_eq!(
            first,
            Decision {
                show_entry: true,
                notify: None
            }
        );
        assert_eq!(s.first_seen, Some(T0));
        // A restart the same day: still only the entry.
        assert_eq!(decide(false, true, false, &mut s, T0 + 3600).notify, None);
        // A day later: the notification.
        assert_eq!(
            decide(false, true, false, &mut s, T0 + DAY).notify,
            Some(OfferKind::Plain)
        );
        assert_eq!(s.last_notified, Some(T0 + DAY));
    }

    #[test]
    fn not_now_reoffers_at_most_once_a_day() {
        let mut s = OfferState::default();
        decide(false, true, false, &mut s, T0);
        decide(false, true, false, &mut s, T0 + DAY);
        apply_answer(&mut s, Answer::NotNow);
        assert!(!s.never);
        // Restarts within the day stay quiet.
        assert_eq!(
            decide(false, true, false, &mut s, T0 + DAY + 600).notify,
            None
        );
        assert_eq!(
            decide(false, true, false, &mut s, T0 + 2 * DAY - 1).notify,
            None
        );
        // The next start after a day re-offers.
        assert_eq!(
            decide(false, true, false, &mut s, T0 + 2 * DAY).notify,
            Some(OfferKind::Plain)
        );
    }

    #[test]
    fn dont_ask_again_hides_everything_for_good() {
        let mut s = OfferState::default();
        decide(false, true, true, &mut s, T0);
        apply_answer(&mut s, Answer::Never);
        for later in [T0 + 1, T0 + DAY, T0 + 30 * DAY] {
            assert_eq!(
                decide(false, true, true, &mut s, later),
                Decision::default()
            );
        }
    }

    #[test]
    fn accepted_offer_goes_away_once_meetings_are_on() {
        let mut s = OfferState::default();
        decide(false, true, false, &mut s, T0);
        apply_answer(&mut s, Answer::TurnOn);
        assert!(!s.never);
        assert_eq!(
            decide(true, true, false, &mut s, T0 + DAY),
            Decision::default()
        );
    }

    #[test]
    fn meeting_recorder_users_are_asked_on_the_first_start() {
        let mut s = OfferState::default();
        assert_eq!(
            decide(false, true, true, &mut s, T0),
            Decision {
                show_entry: true,
                notify: Some(OfferKind::MeetingRecorder)
            }
        );
        assert!(s.recorder_notified);
        // Not now: back to the daily rule, same wording.
        apply_answer(&mut s, Answer::NotNow);
        assert_eq!(decide(false, true, true, &mut s, T0 + 60).notify, None);
        assert_eq!(
            decide(false, true, true, &mut s, T0 + DAY).notify,
            Some(OfferKind::MeetingRecorder)
        );
    }

    #[test]
    fn recorder_installed_later_gets_its_own_prompt_at_once() {
        let mut s = OfferState::default();
        decide(false, true, false, &mut s, T0);
        assert_eq!(
            decide(false, true, true, &mut s, T0 + 60).notify,
            Some(OfferKind::MeetingRecorder)
        );
    }

    #[test]
    fn daemon_start_persists_between_runs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("voxtype").join(STATE_FILE);
        assert_eq!(
            on_daemon_start(&path, false, true, true, T0).notify,
            Some(OfferKind::MeetingRecorder)
        );
        // The next start remembers it asked.
        assert_eq!(
            on_daemon_start(&path, false, true, true, T0 + 5).notify,
            None
        );
        record_answer(&path, Answer::Never);
        assert_eq!(
            on_daemon_start(&path, false, true, true, T0 + 9 * DAY),
            Decision::default()
        );
        assert!(OfferState::load(&path).never);
    }

    #[test]
    fn answers_map_from_notification_actions() {
        assert_eq!(Answer::from_action("turn-on"), Answer::TurnOn);
        assert_eq!(Answer::from_action("default"), Answer::TurnOn);
        assert_eq!(Answer::from_action("not-now"), Answer::NotNow);
        assert_eq!(Answer::from_action("closed"), Answer::NotNow);
        assert_eq!(Answer::from_action("never"), Answer::Never);
    }

    #[test]
    fn state_round_trips_and_survives_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("voxtype").join(STATE_FILE);
        assert_eq!(OfferState::load(&path), OfferState::default());
        let s = OfferState {
            never: true,
            first_seen: Some(T0),
            last_notified: Some(T0 + 5),
            recorder_notified: true,
        };
        s.save(&path).unwrap();
        assert_eq!(OfferState::load(&path), s);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(OfferState::load(&path), OfferState::default());
    }

    #[test]
    fn recorder_probe_reads_path_and_pacman_db() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let db = dir.path().join("local");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&db).unwrap();
        std::fs::create_dir(db.join("omarchy-meeting-recorderx-1.0-1")).unwrap();
        let path = std::ffi::OsString::from(bin.as_os_str());
        assert!(!recorder_installed(Some(&path), &db));
        std::fs::create_dir(db.join("omarchy-meeting-recorder-bin-0.4.2-1")).unwrap();
        assert!(recorder_installed(Some(&path), &db));
        std::fs::remove_dir(db.join("omarchy-meeting-recorder-bin-0.4.2-1")).unwrap();
        std::fs::write(bin.join("omarchy-meeting-recorder"), "").unwrap();
        assert!(recorder_installed(Some(&path), &db));
        assert!(!recorder_installed(None, &dir.path().join("missing")));
    }

    #[test]
    fn wording_is_neutral_and_names_the_recorder_only_when_installed() {
        let (_, plain) = wording(OfferKind::Plain);
        assert!(!plain.contains("Meeting Recorder"));
        let (_, mr) = wording(OfferKind::MeetingRecorder);
        assert!(mr.starts_with("You record meetings with Meeting Recorder. "));
        assert_eq!(
            actions().map(|(_, l)| l),
            ["Turn on", "Not now", "No, don't ask again"]
        );
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        crate::test_support::write_script(&p, &format!("#!/bin/sh\n{body}\n"));
        p
    }

    fn enabler(dir: &Path, systemctl_active: bool) -> (Enabler, PathBuf) {
        let log = dir.join("log");
        let exe = script(
            dir,
            "voxtype",
            &format!("echo \"voxtype $*\" >> '{}'", log.display()),
        );
        let systemctl = script(
            dir,
            "systemctl",
            &format!(
                "echo \"systemctl $*\" >> '{}'\n[ \"$2\" = is-active ] && echo {}\nexit 0",
                log.display(),
                if systemctl_active {
                    "active"
                } else {
                    "inactive"
                }
            ),
        );
        (
            Enabler {
                exe,
                config_path: None,
                user_config: Some(dir.join("xc/voxtype/config.toml")),
                system_config: dir.join("etc/config.toml"),
                systemctl,
            },
            log,
        )
    }

    #[tokio::test]
    async fn turning_on_sets_the_key_and_restarts_the_service() {
        let dir = tempfile::tempdir().unwrap();
        let (e, log) = enabler(dir.path(), true);
        std::fs::create_dir_all(dir.path().join("xc/voxtype")).unwrap();
        std::fs::write(dir.path().join("xc/voxtype/config.toml"), "").unwrap();
        assert_eq!(e.enable().await.unwrap(), Enabled::Restarting);
        let user = dir.path().join("xc/voxtype/config.toml");
        assert_eq!(
            std::fs::read_to_string(log).unwrap(),
            format!(
                "voxtype --config {} config set meeting.enabled true\n\
                 systemctl --user is-active voxtype\n\
                 systemctl --user --no-block restart voxtype\n",
                user.display()
            )
        );
    }

    #[tokio::test]
    async fn without_the_service_the_user_is_told_to_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (e, log) = enabler(dir.path(), false);
        assert!(matches!(
            e.enable().await.unwrap(),
            Enabled::RestartNeeded(_)
        ));
        assert!(!std::fs::read_to_string(log)
            .unwrap()
            .contains("restart voxtype"));
    }

    #[tokio::test]
    async fn a_user_on_the_system_config_gets_their_own_copy_first() {
        let dir = tempfile::tempdir().unwrap();
        let (e, _) = enabler(dir.path(), false);
        std::fs::create_dir_all(dir.path().join("etc")).unwrap();
        std::fs::write(
            dir.path().join("etc/config.toml"),
            "[vad]\nenabled = true\n",
        )
        .unwrap();
        e.enable().await.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("xc/voxtype/config.toml")).unwrap(),
            "[vad]\nenabled = true\n"
        );
    }

    #[tokio::test]
    async fn an_explicit_config_path_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, log) = enabler(dir.path(), false);
        e.config_path = Some(PathBuf::from("/etc/custom.toml"));
        e.enable().await.unwrap();
        assert!(std::fs::read_to_string(log)
            .unwrap()
            .starts_with("voxtype --config /etc/custom.toml config set"));
    }

    #[tokio::test]
    async fn a_failed_config_set_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, _) = enabler(dir.path(), true);
        e.exe = script(dir.path(), "failing", "echo nope >&2; exit 2");
        let err = e.enable().await.unwrap_err().to_string();
        assert!(err.contains("nope"), "{err}");
    }
}
