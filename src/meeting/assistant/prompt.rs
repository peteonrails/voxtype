//! The prompt shared by `meeting ask` and summaries.
//!
//! ```text
//! Meeting: Weekly sync, 2026-11-11 14:00, 32m. Speakers: Pete (You), Steve (Remote 1).
//! [00:41] Steve: ...
//! [00:52] Pete: ...
//! User notes: ...
//! Annotations: [01:58] "check this number"
//! Question: what did Steve just say?
//! Answer briefly. Cite the [mm:ss] of the lines you rely on.
//! ```
//!
//! Speaker names are what the transcript shows on screen: labels and
//! per-line reassignments are applied by the storage layer when the
//! transcript is loaded, before it gets here.

use crate::meeting::data::{MeetingData, TranscriptSegment};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A note attached to one transcript line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Annotation {
    pub start_ms: u64,
    pub text: String,
}

/// A `[mm:ss]` an answer cites.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Citation {
    /// As written, without brackets: `01:58` or `1:02:03`.
    pub ts: String,
    pub ms: u64,
}

/// One rendered transcript line.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub start_ms: u64,
    pub text: String,
}

/// `mm:ss`, or `h:mm:ss` from an hour on.
pub fn ts(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Parse `mm:ss` or `h:mm:ss` into milliseconds.
pub fn parse_ts(s: &str) -> Option<u64> {
    let parts: Vec<u64> = s
        .trim()
        .split(':')
        .map(|p| p.parse::<u64>().ok())
        .collect::<Option<_>>()?;
    let secs = match parts.as_slice() {
        [m, s] if *s < 60 => m * 60 + s,
        [h, m, s] if *m < 60 && *s < 60 => h * 3600 + m * 60 + s,
        _ => return None,
    };
    Some(secs * 1000)
}

/// Every `[mm:ss]` / `[h:mm:ss]` in `answer`, in order, without repeats.
/// Ranges like `[01:58-02:10]` cite both ends.
pub fn citations(answer: &str) -> Vec<Citation> {
    let mut out: Vec<Citation> = Vec::new();
    let mut rest = answer;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        for part in after[..close].split([',', '-', '–']) {
            let part = part.trim();
            if let Some(ms) = parse_ts(part) {
                if !out.iter().any(|c| c.ts == part) {
                    out.push(Citation {
                        ts: part.to_string(),
                        ms,
                    });
                }
            }
        }
        rest = &after[close + 1..];
    }
    out
}

/// The end of the transcript so far, in meeting time.
pub fn transcript_end_ms(meeting: &MeetingData) -> u64 {
    meeting
        .transcript
        .segments
        .iter()
        .map(|s| s.end_ms)
        .max()
        .unwrap_or(0)
}

fn duration_text(meeting: &MeetingData, now: DateTime<Utc>) -> String {
    let secs = meeting.metadata.duration_secs.unwrap_or_else(|| {
        (now - meeting.metadata.started_at)
            .num_seconds()
            .max((transcript_end_ms(meeting) / 1000) as i64) as u64
    });
    let mins = secs.div_ceil(60);
    let live = if meeting.metadata.ended_at.is_none() {
        " so far"
    } else {
        ""
    };
    if mins >= 60 {
        format!("{}h {}m{live}", mins / 60, mins % 60)
    } else {
        format!("{mins}m{live}")
    }
}

/// `Pete (You), Steve (Remote 1), Remote 2`: each speaker's display name,
/// with the diarizer's ID when a name was given, in order of first line.
pub fn speaker_list(segments: &[TranscriptSegment]) -> String {
    let mut seen: Vec<(String, String)> = Vec::new();
    for s in segments {
        let key = s.speaker_key();
        if !seen.iter().any(|(k, _)| *k == key) {
            seen.push((key, s.speaker_display()));
        }
    }
    seen.iter()
        .map(|(key, name)| {
            if key == name {
                name.clone()
            } else {
                format!("{name} ({key})")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The first line of the prompt.
pub fn header(meeting: &MeetingData, now: DateTime<Utc>) -> String {
    let speakers = speaker_list(&meeting.transcript.segments);
    format!(
        "Meeting: {}, {}, {}. Speakers: {}.",
        meeting.metadata.display_title(),
        meeting
            .metadata
            .started_at
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M"),
        duration_text(meeting, now),
        if speakers.is_empty() {
            "none yet".to_string()
        } else {
            speakers
        }
    )
}

/// `[mm:ss] Name: text` for every segment with text.
pub fn lines(segments: &[TranscriptSegment]) -> Vec<Line> {
    segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| Line {
            start_ms: s.start_ms,
            text: format!(
                "[{}] {}: {}",
                ts(s.start_ms),
                s.speaker_display(),
                s.text.trim()
            ),
        })
        .collect()
}

/// Lines from the last `window` of the transcript (ending at `end_ms`).
pub fn last_window(all: &[Line], window: Duration, end_ms: u64) -> Vec<Line> {
    let from = end_ms.saturating_sub(window.as_millis() as u64);
    all.iter().filter(|l| l.start_ms >= from).cloned().collect()
}

fn size(lines: &[Line]) -> usize {
    lines.iter().map(|l| l.text.len() + 1).sum()
}

/// How the transcript fits the character budget.
#[derive(Debug, Clone, PartialEq)]
pub enum Fit {
    /// Everything, verbatim.
    Whole(Vec<Line>),
    /// Too long: `recent` verbatim, `earlier` to be summarized.
    Split {
        earlier: Vec<Line>,
        recent: Vec<Line>,
    },
}

/// Fit `all` into `budget` characters. When it does not fit, the most
/// recent lines that fit in three quarters of the budget stay verbatim and
/// the rest is summarized into the remaining quarter.
pub fn fit(all: Vec<Line>, budget: usize) -> Fit {
    if size(&all) <= budget {
        return Fit::Whole(all);
    }
    let keep = budget * 3 / 4;
    let mut used = 0;
    let mut cut = all.len();
    while cut > 0 {
        let n = all[cut - 1].text.len() + 1;
        if used + n > keep {
            break;
        }
        used += n;
        cut -= 1;
    }
    // Always keep at least the last line, truncated if it alone is huge.
    if cut == all.len() {
        cut -= 1;
    }
    let mut earlier = all;
    let mut recent = earlier.split_off(cut);
    for l in &mut recent {
        if l.text.len() > keep {
            let mut end = keep;
            while !l.text.is_char_boundary(end) {
                end -= 1;
            }
            l.text.truncate(end);
            l.text.push('…');
        }
    }
    Fit::Split { earlier, recent }
}

/// Split `lines` into pieces of at most `budget` characters each.
pub fn pieces(lines: &[Line], budget: usize) -> Vec<Vec<Line>> {
    let mut out: Vec<Vec<Line>> = Vec::new();
    let mut cur: Vec<Line> = Vec::new();
    let mut used = 0;
    for l in lines {
        let n = l.text.len() + 1;
        if !cur.is_empty() && used + n > budget {
            out.push(std::mem::take(&mut cur));
            used = 0;
        }
        used += n;
        cur.push(l.clone());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Everything the agent is told about the meeting.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub header: String,
    /// Summary of the part of the meeting before `recent`, and where it ends.
    pub earlier_summary: Option<(String, u64)>,
    /// Said when only part of the meeting is included on purpose (`--last`).
    pub excerpt: Option<String>,
    pub lines: Vec<Line>,
    pub notes: Option<String>,
    pub annotations: Vec<Annotation>,
}

impl Context {
    fn render(&self) -> String {
        let mut p = String::new();
        p.push_str(&self.header);
        p.push('\n');
        if let Some(excerpt) = &self.excerpt {
            p.push_str(excerpt);
            p.push('\n');
        }
        if let Some((summary, through)) = &self.earlier_summary {
            p.push_str(&format!(
                "Summary of the meeting up to [{}]:\n{}\nTranscript from [{}] on:\n",
                ts(*through),
                summary.trim(),
                ts(*through)
            ));
        }
        if self.lines.is_empty() {
            p.push_str("(No transcript lines yet.)\n");
        }
        for l in &self.lines {
            p.push_str(&l.text);
            p.push('\n');
        }
        if let Some(notes) = self
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            p.push_str("User notes:\n");
            p.push_str(notes);
            p.push('\n');
        }
        if !self.annotations.is_empty() {
            p.push_str("Annotations:");
            for a in &self.annotations {
                p.push_str(&format!(" [{}] \"{}\"", ts(a.start_ms), a.text.trim()));
            }
            p.push('\n');
        }
        p
    }

    /// The prompt for one question.
    pub fn ask(&self, question: &str) -> String {
        let mut p = self.render();
        p.push_str(&format!("Question: {}\n", question.trim()));
        p.push_str("Answer briefly. Cite the [mm:ss] of the lines you rely on.\n");
        p
    }

    /// The prompt for a full summary, answered as JSON.
    pub fn summary(&self) -> String {
        let mut p = self.render();
        p.push_str(SUMMARY_INSTRUCTIONS);
        p
    }
}

const SUMMARY_INSTRUCTIONS: &str = r#"Summarize this meeting. Where the user's notes say what mattered, expand on them from the transcript rather than ignoring them.
Use speaker names exactly as written above. Reply with only a JSON object, no other text:
{
  "overview": "3-5 sentences on what the meeting covered and concluded",
  "key_points": ["..."],
  "decisions": ["..."],
  "action_items": [{"description": "...", "owner": "speaker name or null", "due": "date or null"}],
  "chapters": [{"start": "mm:ss", "title": "..."}]
}
Chapters follow the order of the meeting; each start is the [mm:ss] of the line where that topic begins.
"#;

/// Prompt that folds `lines` into a running summary of the meeting so far.
pub fn rolling_summary_prompt(header: &str, previous: Option<&str>, lines: &[Line]) -> String {
    let mut p = String::new();
    p.push_str(header);
    p.push('\n');
    if let Some(prev) = previous {
        p.push_str("Summary so far:\n");
        p.push_str(prev.trim());
        p.push_str("\nTranscript that follows:\n");
    }
    for l in lines {
        p.push_str(&l.text);
        p.push('\n');
    }
    p.push_str(
        "Write a running summary of the meeting so far that someone could use to answer \
         questions about it: topics in order with their [mm:ss], what each speaker said that \
         matters, numbers, decisions and action items with owners. Plain text, at most 400 words.\n",
    );
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::data::AudioSource;

    fn seg(id: u32, secs: u64, key: &str, label: Option<&str>, text: &str) -> TranscriptSegment {
        let mut s = TranscriptSegment::new(id, secs * 1000, secs * 1000 + 2000, text.into(), 0);
        s.source = if key == "You" {
            AudioSource::Microphone
        } else {
            AudioSource::Loopback
        };
        s.speaker_id = Some(key.to_string());
        s.speaker_label = label.map(String::from);
        s
    }

    #[test]
    fn timestamps_round_trip() {
        assert_eq!(ts(41_000), "00:41");
        assert_eq!(ts(3_723_000), "1:02:03");
        assert_eq!(parse_ts("00:41"), Some(41_000));
        assert_eq!(parse_ts("1:02:03"), Some(3_723_000));
        assert_eq!(parse_ts("1:99"), None);
        assert_eq!(parse_ts("abc"), None);
    }

    #[test]
    fn citations_are_found_once_each() {
        let c = citations("Steve said so [00:41], again [00:41] and at [01:58-02:10]. [note]");
        let ts: Vec<&str> = c.iter().map(|c| c.ts.as_str()).collect();
        assert_eq!(ts, vec!["00:41", "01:58", "02:10"]);
        assert_eq!(c[1].ms, 118_000);
    }

    #[test]
    fn speakers_show_names_and_ids() {
        let segs = vec![
            seg(0, 1, "You", Some("Pete"), "hi"),
            seg(1, 5, "Remote 1", Some("Steve"), "hello"),
            seg(2, 9, "Remote 2", None, "hey"),
            seg(3, 12, "You", Some("Pete"), "ok"),
        ];
        assert_eq!(
            speaker_list(&segs),
            "Pete (You), Steve (Remote 1), Remote 2"
        );
    }

    fn mk(n: usize, len: usize) -> Vec<Line> {
        (0..n)
            .map(|i| Line {
                start_ms: i as u64 * 1000,
                text: "x".repeat(len - 1),
            })
            .collect()
    }

    #[test]
    fn a_short_transcript_fits_whole() {
        assert!(matches!(fit(mk(10, 10), 100), Fit::Whole(l) if l.len() == 10));
    }

    #[test]
    fn a_long_transcript_keeps_the_recent_lines() {
        match fit(mk(100, 10), 200) {
            Fit::Split { earlier, recent } => {
                assert_eq!(recent.len(), 15); // 150 chars of 200 * 3/4
                assert_eq!(earlier.len(), 85);
                assert_eq!(recent[0].start_ms, 85_000);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn one_huge_line_is_truncated_not_dropped() {
        match fit(mk(2, 1000), 100) {
            Fit::Split { recent, .. } => {
                assert_eq!(recent.len(), 1);
                assert!(recent[0].text.len() <= 75 + '…'.len_utf8());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pieces_respect_the_budget() {
        let p = pieces(&mk(10, 10), 35);
        assert_eq!(p.iter().map(Vec::len).collect::<Vec<_>>(), vec![3, 3, 3, 1]);
    }

    #[test]
    fn last_window_is_a_time_window() {
        let all = mk(600, 10); // one line a second for ten minutes
        let w = last_window(&all, Duration::from_secs(120), 600_000);
        assert_eq!(w.first().unwrap().start_ms, 480_000);
        assert_eq!(w.len(), 120);
    }

    #[test]
    fn prompt_has_every_section() {
        let ctx = Context {
            header: "Meeting: X".into(),
            earlier_summary: Some(("They planned.".into(), 60_000)),
            excerpt: None,
            lines: vec![Line {
                start_ms: 61_000,
                text: "[01:01] Steve: ship it".into(),
            }],
            notes: Some("check budget".into()),
            annotations: vec![Annotation {
                start_ms: 118_000,
                text: "check this number".into(),
            }],
        };
        let p = ctx.ask("what did Steve say?");
        assert!(p.contains("Summary of the meeting up to [01:00]:\nThey planned."));
        assert!(p.contains("[01:01] Steve: ship it\n"));
        assert!(p.contains("User notes:\ncheck budget\n"));
        assert!(p.contains("Annotations: [01:58] \"check this number\""));
        assert!(p.ends_with(
            "Question: what did Steve say?\nAnswer briefly. Cite the [mm:ss] of the lines you rely on.\n"
        ));
        assert!(ctx.summary().contains("\"chapters\""));
    }
}
