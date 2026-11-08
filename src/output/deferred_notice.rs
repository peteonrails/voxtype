//! Notices about a step of the output pipeline that failed without stopping
//! the dictation.
//!
//! A failed post-process command or output hook is not fatal: voxtype uses
//! the original text and carries on. That is the right behaviour, but done
//! silently it leaves the user believing the LLM cleanup ran. These notices
//! tell them.
//!
//! They are queued rather than sent on the spot because every voxtype
//! notification shares one slot: the "Transcribed" notification posted right
//! after output would replace a notice sent during post-processing before
//! anyone could read it. The daemon calls [`flush`] once the dictation is
//! delivered, so the notice is what stays on screen.
//!
//! Each failure has a key such as `post_process:timeout`. The first failure
//! for a key always notifies; repeats of the same key are suppressed for
//! [`REPEAT_AFTER`], and a success clears the keys of its source so the next
//! failure after a recovery is reported again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a repeat of the same failure stays quiet.
pub const REPEAT_AFTER: Duration = Duration::from_secs(300);

/// How long a notice stays on screen. Longer than a status notification:
/// it carries a fix to read.
#[cfg(target_os = "linux")]
const NOTICE_MS: u32 = 8000;

/// Longest detail (an error line, a command) quoted in a notice, so the fix
/// that follows it is not pushed off screen.
const MAX_DETAIL_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// Rate limiter keyed by failure kind. Pure, so it is tested without a clock.
#[derive(Debug, Default)]
struct Gate {
    last: HashMap<String, Instant>,
}

impl Gate {
    fn should_notify(&mut self, key: &str, now: Instant) -> bool {
        if let Some(at) = self.last.get(key) {
            if now.duration_since(*at) < REPEAT_AFTER {
                return false;
            }
        }
        self.last.insert(key.to_string(), now);
        true
    }

    /// Forget every key that starts with `source`, after it succeeded.
    fn clear(&mut self, source: &str) {
        self.last.retain(|key, _| !key.starts_with(source));
    }
}

#[derive(Default)]
struct State {
    gate: Gate,
    pending: Vec<Notice>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// Queue `notice` for the next [`flush`], unless the same `key` was reported
/// within [`REPEAT_AFTER`].
pub fn report(key: &str, notice: Notice) {
    with_state(|state| {
        if state.gate.should_notify(key, Instant::now()) && !state.pending.contains(&notice) {
            state.pending.push(notice);
        }
    });
}

/// `source` worked again: its next failure notifies even if it is a repeat.
pub fn succeeded(source: &str) {
    with_state(|state| state.gate.clear(source));
}

/// Post the queued notices, merged into one notification so they don't
/// replace each other in the shared slot.
pub async fn flush() {
    let pending = with_state(|state| std::mem::take(&mut state.pending));
    let Some(notice) = merge(pending) else {
        return;
    };

    #[cfg(target_os = "linux")]
    crate::notification::send_status(
        &notice.title,
        &notice.body,
        "normal",
        crate::notification::Lifetime::Millis(NOTICE_MS),
    )
    .await;

    #[cfg(not(target_os = "linux"))]
    crate::notification::send(&notice.title, &notice.body).await;
}

fn merge(mut notices: Vec<Notice>) -> Option<Notice> {
    match notices.len() {
        0 => None,
        1 => notices.pop(),
        _ => Some(Notice {
            title: "Voxtype: some dictation steps failed".to_string(),
            body: notices
                .iter()
                .map(|n| format!("{}: {}", n.title.trim_start_matches("Voxtype: "), n.body))
                .collect::<Vec<_>>()
                .join("\n\n"),
        }),
    }
}

/// First non-empty line of `text`, clipped to [`MAX_DETAIL_CHARS`].
pub fn detail(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first.chars().count() <= MAX_DETAIL_CHARS {
        first.to_string()
    } else {
        let clipped: String = first.chars().take(MAX_DETAIL_CHARS).collect();
        format!("{}...", clipped.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_failure_notifies_and_repeats_are_quiet() {
        let mut gate = Gate::default();
        let t0 = Instant::now();
        assert!(gate.should_notify("post_process:timeout", t0));
        assert!(!gate.should_notify("post_process:timeout", t0 + Duration::from_secs(60)));
        assert!(gate.should_notify("post_process:timeout", t0 + REPEAT_AFTER));
    }

    #[test]
    fn a_different_failure_still_notifies() {
        let mut gate = Gate::default();
        let t0 = Instant::now();
        assert!(gate.should_notify("post_process:timeout", t0));
        assert!(gate.should_notify("post_process:exit", t0));
        assert!(gate.should_notify("hook:post_output", t0));
    }

    #[test]
    fn success_resets_only_its_source() {
        let mut gate = Gate::default();
        let t0 = Instant::now();
        gate.should_notify("post_process:timeout", t0);
        gate.should_notify("hook:post_output", t0);
        gate.clear("post_process:");
        assert!(gate.should_notify("post_process:timeout", t0));
        assert!(!gate.should_notify("hook:post_output", t0));
    }

    #[test]
    fn merge_keeps_a_single_notice_and_joins_several() {
        let a = Notice {
            title: "Voxtype: A".into(),
            body: "a".into(),
        };
        let b = Notice {
            title: "Voxtype: B".into(),
            body: "b".into(),
        };
        assert_eq!(merge(vec![]), None);
        assert_eq!(merge(vec![a.clone()]), Some(a.clone()));
        let both = merge(vec![a, b]).unwrap();
        assert_eq!(both.body, "A: a\n\nB: b");
    }

    #[test]
    fn detail_takes_first_line_and_clips() {
        assert_eq!(detail("\n  boom  \nmore"), "boom");
        let long = "x".repeat(200);
        assert_eq!(detail(&long).chars().count(), MAX_DETAIL_CHARS + 3);
    }
}
