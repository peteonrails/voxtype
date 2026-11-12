//! The meeting status notifications (started, paused, resumed, ended) and
//! when to show them.
//!
//! A meeting view on screen already shows all of this, so by default
//! (`[meeting] status_notifications = "auto"`) they are skipped while one is
//! visible. Error notifications never go through here: they always show.

use std::time::Duration;

/// `[meeting] status_notifications`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusMode {
    /// Skip while a meeting view is on screen.
    Auto,
    Always,
    Never,
}

pub const STATUS_MODES: &[&str] = &["auto", "always", "never"];

impl StatusMode {
    /// Unknown values read as `auto`, the default.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "always" => Self::Always,
            "never" => Self::Never,
            _ => Self::Auto,
        }
    }
}

/// Whether to post a status notification. `enabled` is the existing
/// `[output.notification] on_recording_start/stop` switch, which still
/// governs these; `view_visible` is only consulted in auto mode.
pub fn should_notify(mode: StatusMode, enabled: bool, view_visible: impl FnOnce() -> bool) -> bool {
    enabled
        && match mode {
            StatusMode::Always => true,
            StatusMode::Never => false,
            StatusMode::Auto => !view_visible(),
        }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingNotice {
    Started { title: String },
    Paused { title: String },
    Resumed { title: String },
    Ended { title: String, elapsed: Duration },
}

impl MeetingNotice {
    /// Summary and body.
    pub fn text(&self) -> (&'static str, String) {
        match self {
            Self::Started { title } => ("Meeting Started", title.clone()),
            Self::Paused { title } => ("Meeting Paused", title.clone()),
            Self::Resumed { title } => ("Meeting Resumed", title.clone()),
            Self::Ended { title, elapsed } => (
                "Meeting Ended",
                format!("{} ({})", title, format_elapsed(*elapsed)),
            ),
        }
    }
}

/// "under a minute", "42 min", "1 h 05 min".
pub fn format_elapsed(d: Duration) -> String {
    let mins = d.as_secs() / 60;
    match mins {
        0 => "under a minute".to_string(),
        m if m < 60 => format!("{m} min"),
        m => format!("{} h {:02} min", m / 60, m % 60),
    }
}

/// The meeting's title for a notification: the one it was started with,
/// else the same date-based name the meeting list uses.
pub fn display_title(
    title: Option<&str>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
) -> String {
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => started_at
            .unwrap_or_else(chrono::Utc::now)
            .format("Meeting %Y-%m-%d %H:%M")
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_for_every_mode_and_view_state() {
        use StatusMode::*;
        for (mode, view, want) in [
            (Auto, false, true),
            (Auto, true, false),
            (Always, false, true),
            (Always, true, true),
            (Never, false, false),
            (Never, true, false),
        ] {
            assert_eq!(
                should_notify(mode, true, || view),
                want,
                "{mode:?} view={view}"
            );
            assert!(
                !should_notify(mode, false, || view),
                "the on_recording switch still gates"
            );
        }
    }

    #[test]
    fn only_auto_looks_at_the_view() {
        assert!(should_notify(StatusMode::Always, true, || panic!(
            "not consulted"
        )));
        assert!(!should_notify(StatusMode::Never, true, || panic!(
            "not consulted"
        )));
    }

    #[test]
    fn decision_follows_the_presence_file() {
        use super::super::view_presence::{view_visible_at, STALE_SECS, STATE_FILE};
        let now = 1_800_000_000u64;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATE_FILE);
        let write = |visible: bool, minimized: bool, age: u64| {
            std::fs::write(
                &path,
                format!(
                    r#"{{"visible":{visible},"minimized":{minimized},"host":"standalone","pid":7,"updated_at":{}}}"#,
                    now - age
                ),
            )
            .unwrap()
        };
        let auto = |alive: bool| {
            should_notify(StatusMode::Auto, true, || {
                view_visible_at(&path, now, |_| alive)
            })
        };

        assert!(auto(true), "absent file: notify");
        write(true, false, 1);
        assert!(!auto(true), "visible view: skip");
        write(true, true, 1);
        assert!(!auto(true), "minimized pill: skip");
        write(true, false, STALE_SECS + 5);
        assert!(auto(true), "stale file: notify");
        write(true, false, 1);
        assert!(auto(false), "dead host: notify");
        write(false, false, 1);
        assert!(auto(true), "hidden view: notify");
    }

    #[test]
    fn modes_parse_with_auto_as_the_fallback() {
        assert_eq!(StatusMode::parse("auto"), StatusMode::Auto);
        assert_eq!(StatusMode::parse("Always"), StatusMode::Always);
        assert_eq!(StatusMode::parse(" never "), StatusMode::Never);
        assert_eq!(StatusMode::parse("bogus"), StatusMode::Auto);
    }

    #[test]
    fn notices_carry_the_title_and_the_ended_one_the_length() {
        let t = || "Weekly sync".to_string();
        assert_eq!(
            MeetingNotice::Started { title: t() }.text(),
            ("Meeting Started", "Weekly sync".to_string())
        );
        assert_eq!(MeetingNotice::Paused { title: t() }.text().1, "Weekly sync");
        assert_eq!(
            MeetingNotice::Resumed { title: t() }.text().1,
            "Weekly sync"
        );
        assert_eq!(
            MeetingNotice::Ended {
                title: t(),
                elapsed: Duration::from_secs(62 * 60 + 30)
            }
            .text(),
            ("Meeting Ended", "Weekly sync (1 h 02 min)".to_string())
        );
        for (_, body) in [
            MeetingNotice::Started { title: t() }.text(),
            MeetingNotice::Ended {
                title: t(),
                elapsed: Duration::ZERO,
            }
            .text(),
        ] {
            assert!(!body.contains("ID:"), "no UUIDs: {body}");
        }
    }

    #[test]
    fn elapsed_reads_naturally() {
        assert_eq!(format_elapsed(Duration::from_secs(20)), "under a minute");
        assert_eq!(format_elapsed(Duration::from_secs(42 * 60 + 59)), "42 min");
        assert_eq!(format_elapsed(Duration::from_secs(3600)), "1 h 00 min");
    }

    #[test]
    fn untitled_meetings_use_the_date_name() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-11T14:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(display_title(Some("Standup"), Some(at)), "Standup");
        assert_eq!(
            display_title(Some("  "), Some(at)),
            "Meeting 2026-11-11 14:30"
        );
        assert_eq!(display_title(None, Some(at)), "Meeting 2026-11-11 14:30");
    }
}
