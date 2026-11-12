//! Omarchy reminders: `omarchy-reminder <minutes> "<text>"`, a desktop
//! notification after a delay. It has no lists or dates, so it suits
//! "remind me about this later today"; Todoist is the place to keep tasks.

use chrono::{NaiveDateTime, NaiveTime};
use std::path::PathBuf;
use std::time::Duration;

pub const PROGRAM: &str = "omarchy-reminder";

/// The `omarchy-reminder` to run.
#[derive(Debug, Clone)]
pub struct Reminders {
    pub program: PathBuf,
    /// Pause between two reminders for the same delay. omarchy-reminder
    /// names its timer after the delay and the second it was set, so two in
    /// one second would collide.
    pub spacing: Duration,
}

impl Default for Reminders {
    fn default() -> Self {
        Self {
            program: PROGRAM.into(),
            spacing: Duration::from_millis(1100),
        }
    }
}

impl Reminders {
    /// Whether omarchy-reminder is installed here.
    pub fn available(&self) -> bool {
        if self.program.components().count() > 1 {
            self.program.is_file()
        } else {
            which::which(&self.program).is_ok()
        }
    }

    /// Set a reminder `minutes` from now saying `text`.
    pub fn set(&self, minutes: u32, text: &str) -> Result<(), String> {
        let output = std::process::Command::new(&self.program)
            .arg(minutes.max(1).to_string())
            .arg(text)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("{} could not start: {e}", self.program.display()))?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(if stderr.is_empty() {
                format!("{} failed ({})", self.program.display(), output.status)
            } else {
                stderr
            })
        }
    }
}

/// Minutes until a reminder for an item due at `due_at` on `due_on`, from
/// `now`: until that time when it is later today, else `default`.
pub fn minutes_for(
    due_on: Option<chrono::NaiveDate>,
    due_at: Option<NaiveTime>,
    now: NaiveDateTime,
    default: u32,
) -> u32 {
    if let (Some(on), Some(at)) = (due_on, due_at) {
        if on == now.date() {
            let secs = (on.and_time(at) - now).num_seconds();
            if secs > 0 {
                return (secs as u64).div_ceil(60).min(u32::MAX as u64) as u32;
            }
        }
    }
    default.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 12)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap()
    }

    #[test]
    fn a_time_later_today_sets_the_delay() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 12);
        let three = NaiveTime::from_hms_opt(15, 0, 0);
        assert_eq!(minutes_for(today, three, at(14, 20), 60), 40);
        assert_eq!(minutes_for(today, three, at(14, 59), 60), 1);
        // Past, another day, or no time: the default delay.
        assert_eq!(minutes_for(today, three, at(16, 0), 60), 60);
        assert_eq!(
            minutes_for(NaiveDate::from_ymd_opt(2026, 11, 13), three, at(14, 0), 30),
            30
        );
        assert_eq!(minutes_for(today, None, at(14, 0), 30), 30);
        assert_eq!(minutes_for(None, None, at(14, 0), 0), 1);
    }

    #[test]
    fn a_reminder_runs_omarchy_reminder_with_minutes_and_text() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("omarchy-reminder");
        crate::test_support::write_script(
            &program,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}/calls\n",
                dir.path().display()
            ),
        );
        let reminders = Reminders {
            program,
            spacing: Duration::ZERO,
        };
        assert!(reminders.available());
        reminders.set(40, "Tag the release").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("calls")).unwrap(),
            "40\nTag the release\n"
        );
        let missing = Reminders {
            program: dir.path().join("nope"),
            spacing: Duration::ZERO,
        };
        assert!(!missing.available());
        assert!(missing.set(1, "x").is_err());
    }
}
