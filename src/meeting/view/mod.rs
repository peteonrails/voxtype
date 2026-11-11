//! A toolkit-free model of one meeting as a viewer shows it.
//!
//! Fed by `meeting.sock` events (see [`crate::meeting::events`]) or loaded
//! from storage, it owns everything a meeting view needs that is not
//! drawing: transcript lines in start-time order, speaker chips merged by
//! the name they display, the meeting's state, and what to do after a
//! reconnect. The terminal view (`voxtype meeting watch`) draws it today; a
//! GTK window can reuse it unchanged.

pub mod client;
pub mod commands;
pub mod frontend;

use crate::meeting::data::{MeetingData, MeetingSummary, TranscriptSegment};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

/// Where the meeting is, as a viewer shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewState {
    /// No meeting: the daemon is idle and nothing is loaded.
    Idle,
    Recording,
    Paused,
    /// Stopping: the last chunk is being transcribed.
    Finalizing,
    /// Stopped, and the summary is being written.
    Summarizing,
    /// Stopped (or loaded from storage).
    Ended,
}

impl ViewState {
    /// A short label for a header.
    pub fn label(self) -> &'static str {
        match self {
            ViewState::Idle => "Idle",
            ViewState::Recording => "Recording",
            ViewState::Paused => "Paused",
            ViewState::Finalizing => "Finalizing",
            ViewState::Summarizing => "Summarizing",
            ViewState::Ended => "Ended",
        }
    }

    /// Is a meeting being recorded right now (including paused and the
    /// final chunk)?
    pub fn is_live(self) -> bool {
        matches!(
            self,
            ViewState::Recording | ViewState::Paused | ViewState::Finalizing
        )
    }

    fn from_status(status: &str) -> Option<Self> {
        match status {
            "recording" => Some(ViewState::Recording),
            "paused" => Some(ViewState::Paused),
            "finalizing" => Some(ViewState::Finalizing),
            "summarizing" => Some(ViewState::Summarizing),
            "idle" => Some(ViewState::Idle),
            _ => None,
        }
    }
}

/// One `meeting.sock` line, as a client reads it. Unknown events and fields
/// are ignored, so a newer daemon never breaks an older viewer.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ViewEvent {
    Hello {
        status: String,
        #[serde(default)]
        meeting_id: Option<String>,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        started_at: Option<DateTime<Utc>>,
        #[serde(default)]
        segments: Vec<TranscriptSegment>,
    },
    Segments {
        #[serde(default)]
        segments: Vec<TranscriptSegment>,
    },
    SegmentUpdated {
        segment: TranscriptSegment,
    },
    SpeakerRenamed {
        from: String,
        to: String,
    },
    NotesUpdated {
        meeting_id: String,
        #[serde(default)]
        updated_at: Option<DateTime<Utc>>,
    },
    Status {
        status: String,
        #[serde(default)]
        meeting_id: Option<String>,
    },
    SummaryReady {
        meeting_id: String,
        #[serde(default)]
        agent_name: String,
        #[serde(default)]
        path: Option<PathBuf>,
        summary: MeetingSummary,
    },
    Error {
        message: String,
        #[serde(default)]
        meeting_id: Option<String>,
        #[serde(default)]
        code: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

impl ViewEvent {
    /// Parse one NDJSON line. `None` for a line that is not an event.
    pub fn parse(line: &str) -> Option<Self> {
        serde_json::from_str(line).ok()
    }
}

/// What an event changed, so a viewer redraws only what it must.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Nothing,
    /// A different meeting, or the same one from a fresh snapshot.
    Reset,
    Lines,
    State,
    Notes,
    Summary,
    Error,
    /// The socket's live events no longer describe a meeting this model
    /// knows (one started while it watched); reconnect for a fresh `hello`.
    Resync,
}

/// A saved summary, with who wrote it.
#[derive(Debug, Clone)]
pub struct SummaryInfo {
    pub agent_name: String,
    pub path: Option<PathBuf>,
    pub summary: MeetingSummary,
}

/// One speaker as the view shows it: every diarizer speaker displayed under
/// the same name is one chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakerChip {
    pub name: String,
    /// The diarizer IDs behind the name (`You`, `Remote 1`, ...)
    pub keys: Vec<String>,
    pub lines: usize,
    /// When the speaker first spoke
    pub first_ms: u64,
}

/// The view model.
#[derive(Debug, Clone)]
pub struct MeetingView {
    pub meeting_id: Option<String>,
    pub title: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<u64>,
    pub state: ViewState,
    /// Connected to the daemon's socket. Always false for a stored meeting.
    pub connected: bool,
    /// Follows the live socket. False for a meeting loaded from storage,
    /// which only takes events about its own summary.
    pub live: bool,
    lines: Vec<TranscriptSegment>,
    pub summary: Option<SummaryInfo>,
    pub last_error: Option<String>,
    /// When notes.md last changed, as the daemon reported it
    pub notes_updated_at: Option<DateTime<Utc>>,
    /// Bumped on every change, for viewers that cache rendering
    pub revision: u64,
}

impl Default for MeetingView {
    fn default() -> Self {
        Self::live()
    }
}

impl MeetingView {
    /// A model that follows the live socket, empty until its first `hello`.
    pub fn live() -> Self {
        Self {
            meeting_id: None,
            title: None,
            started_at: None,
            ended_at: None,
            duration_secs: None,
            state: ViewState::Idle,
            connected: false,
            live: true,
            lines: Vec::new(),
            summary: None,
            last_error: None,
            notes_updated_at: None,
            revision: 0,
        }
    }

    /// A finished meeting from storage, shown as Ended.
    pub fn stored(meeting: &MeetingData) -> Self {
        let m = &meeting.metadata;
        let mut view = Self {
            meeting_id: Some(m.id.to_string()),
            title: Some(m.display_title()),
            started_at: Some(m.started_at),
            ended_at: m.ended_at,
            duration_secs: m.duration_secs,
            state: ViewState::Ended,
            connected: false,
            live: false,
            lines: Vec::new(),
            summary: m.summary.clone().map(|summary| SummaryInfo {
                agent_name: summary.model.clone().unwrap_or_default(),
                path: m.storage_path.as_ref().map(|p| p.join("summary.md")),
                summary,
            }),
            last_error: None,
            notes_updated_at: None,
            revision: 0,
        };
        view.set_lines(meeting.transcript.segments.clone());
        view
    }

    /// Transcript lines, ordered by start time.
    pub fn lines(&self) -> &[TranscriptSegment] {
        &self.lines
    }

    /// The line with segment id `id`.
    pub fn line(&self, id: u32) -> Option<&TranscriptSegment> {
        self.lines.iter().find(|s| s.id == id)
    }

    fn set_lines(&mut self, mut lines: Vec<TranscriptSegment>) {
        lines.sort_by_key(|s| (s.start_ms, s.id));
        lines.dedup_by_key(|s| s.id);
        self.lines = lines;
    }

    /// Insert or replace one line, keeping start-time order.
    fn upsert(&mut self, segment: TranscriptSegment) {
        self.lines.retain(|s| s.id != segment.id);
        let at = self
            .lines
            .partition_point(|s| (s.start_ms, s.id) <= (segment.start_ms, segment.id));
        self.lines.insert(at, segment);
    }

    fn is_ours(&self, meeting_id: Option<&str>) -> bool {
        match (meeting_id, self.meeting_id.as_deref()) {
            (Some(theirs), Some(ours)) => theirs == ours,
            // An event without an ID is about whatever is live.
            (None, _) => self.live,
            (Some(_), None) => false,
        }
    }

    fn reset_to(&mut self, meeting_id: Option<String>) {
        *self = Self {
            meeting_id,
            connected: self.connected,
            live: self.live,
            revision: self.revision,
            ..Self::live()
        };
    }

    fn end(&mut self) {
        self.state = ViewState::Ended;
        if self.ended_at.is_none() {
            self.ended_at = Some(Utc::now());
        }
    }

    /// Apply one event.
    pub fn apply(&mut self, event: ViewEvent) -> Change {
        let change = self.apply_inner(event);
        if change != Change::Nothing {
            self.revision += 1;
        }
        change
    }

    /// Parse and apply one NDJSON line.
    pub fn apply_line(&mut self, line: &str) -> Change {
        match ViewEvent::parse(line) {
            Some(event) => self.apply(event),
            None => Change::Nothing,
        }
    }

    fn apply_inner(&mut self, event: ViewEvent) -> Change {
        if !self.live {
            return self.apply_stored(event);
        }
        match event {
            ViewEvent::Hello {
                status,
                meeting_id,
                title,
                started_at,
                segments,
            } => {
                self.connected = true;
                let state = ViewState::from_status(&status).unwrap_or(ViewState::Idle);
                if state.is_live() {
                    if meeting_id != self.meeting_id || !self.state.is_live() {
                        self.reset_to(meeting_id);
                    }
                    self.title = title;
                    self.started_at = started_at;
                    self.state = state;
                    self.set_lines(segments);
                    return Change::Reset;
                }
                // Idle: a meeting we were showing ended while we were away
                // (or the daemon restarted under it).
                if self.meeting_id.is_some() || !self.lines.is_empty() {
                    if self.state != ViewState::Ended {
                        self.end();
                        return Change::State;
                    }
                    return Change::Nothing;
                }
                self.state = ViewState::Idle;
                Change::State
            }
            ViewEvent::Segments { segments } => {
                if !self.state.is_live() {
                    // Lines for a meeting this model has no hello for.
                    return Change::Resync;
                }
                for s in segments {
                    self.upsert(s);
                }
                Change::Lines
            }
            ViewEvent::SegmentUpdated { segment } => {
                self.upsert(segment);
                Change::Lines
            }
            ViewEvent::SpeakerRenamed { from, to } => {
                for s in &mut self.lines {
                    if s.speaker_id.as_deref() == Some(from.as_str()) {
                        s.speaker_id = Some(to.clone());
                    }
                    if s.original_speaker.as_deref() == Some(from.as_str()) {
                        s.original_speaker = Some(to.clone());
                    }
                }
                Change::Lines
            }
            ViewEvent::NotesUpdated {
                meeting_id,
                updated_at,
            } => {
                if !self.is_ours(Some(&meeting_id)) {
                    return Change::Nothing;
                }
                self.notes_updated_at = updated_at.or_else(|| Some(Utc::now()));
                Change::Notes
            }
            ViewEvent::Status { status, meeting_id } => {
                let Some(state) = ViewState::from_status(&status) else {
                    return Change::Nothing;
                };
                match state {
                    ViewState::Summarizing => {
                        if self.is_ours(meeting_id.as_deref()) && self.meeting_id.is_some() {
                            self.state = ViewState::Summarizing;
                            Change::State
                        } else {
                            Change::Nothing
                        }
                    }
                    ViewState::Idle => {
                        if self.state.is_live() {
                            self.end();
                            Change::State
                        } else {
                            Change::Nothing
                        }
                    }
                    live => {
                        if self.state.is_live() {
                            self.state = live;
                            Change::State
                        } else {
                            // A meeting started while we watched: its title
                            // and ID only come with a fresh hello.
                            Change::Resync
                        }
                    }
                }
            }
            ViewEvent::SummaryReady {
                meeting_id,
                agent_name,
                path,
                summary,
            } => self.summary_ready(&meeting_id, agent_name, path, summary),
            ViewEvent::Error {
                message,
                meeting_id,
                ..
            } => self.error(meeting_id.as_deref(), message),
            ViewEvent::Unknown => Change::Nothing,
        }
    }

    /// A stored meeting only listens for its own summary.
    fn apply_stored(&mut self, event: ViewEvent) -> Change {
        match event {
            ViewEvent::Status { status, meeting_id }
                if status == "summarizing" && self.is_ours(meeting_id.as_deref()) =>
            {
                self.state = ViewState::Summarizing;
                Change::State
            }
            ViewEvent::SummaryReady {
                meeting_id,
                agent_name,
                path,
                summary,
            } => self.summary_ready(&meeting_id, agent_name, path, summary),
            ViewEvent::Error {
                message,
                meeting_id: Some(id),
                ..
            } if self.is_ours(Some(&id)) => self.error(Some(&id), message),
            _ => Change::Nothing,
        }
    }

    fn summary_ready(
        &mut self,
        meeting_id: &str,
        agent_name: String,
        path: Option<PathBuf>,
        summary: MeetingSummary,
    ) -> Change {
        if !self.is_ours(Some(meeting_id)) {
            return Change::Nothing;
        }
        self.summary = Some(SummaryInfo {
            agent_name,
            path,
            summary,
        });
        if self.state == ViewState::Summarizing {
            self.state = ViewState::Ended;
        }
        Change::Summary
    }

    fn error(&mut self, meeting_id: Option<&str>, message: String) -> Change {
        if meeting_id.is_some() && !self.is_ours(meeting_id) {
            return Change::Nothing;
        }
        self.last_error = Some(message);
        if self.state == ViewState::Summarizing {
            self.state = ViewState::Ended;
        }
        Change::Error
    }

    /// The socket dropped. Lines and state stay as they were until the
    /// reconnect's `hello` says otherwise.
    pub fn disconnected(&mut self) {
        if self.connected {
            self.connected = false;
            self.revision += 1;
        }
    }

    /// Time since the meeting started (live), or its length (ended).
    pub fn elapsed(&self, now: DateTime<Utc>) -> Option<Duration> {
        let secs = if self.state.is_live() {
            (now - self.started_at?).num_seconds().max(0) as u64
        } else if let Some(d) = self.duration_secs {
            d
        } else if let (Some(a), Some(b)) = (self.started_at, self.ended_at) {
            (b - a).num_seconds().max(0) as u64
        } else {
            self.lines.iter().map(|s| s.end_ms).max()? / 1000
        };
        Some(Duration::from_secs(secs))
    }

    /// Speakers in order of first line, merged by displayed name.
    pub fn speakers(&self) -> Vec<SpeakerChip> {
        let mut chips: Vec<SpeakerChip> = Vec::new();
        for s in &self.lines {
            if s.text.trim().is_empty() {
                continue;
            }
            let name = s.speaker_display();
            let key = s.speaker_key();
            match chips.iter_mut().find(|c| c.name == name) {
                Some(chip) => {
                    chip.lines += 1;
                    if !chip.keys.contains(&key) {
                        chip.keys.push(key);
                    }
                }
                None => chips.push(SpeakerChip {
                    name,
                    keys: vec![key],
                    lines: 1,
                    first_ms: s.start_ms,
                }),
            }
        }
        chips
    }

    /// Index into [`MeetingView::speakers`] of the chip a line belongs to,
    /// for picking a stable color.
    pub fn speaker_index(&self, segment: &TranscriptSegment) -> Option<usize> {
        let name = segment.speaker_display();
        self.speakers().iter().position(|c| c.name == name)
    }
}

/// `mm:ss`, or `h:mm:ss` from an hour on.
pub fn clock(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::data::AudioSource;

    fn seg(id: u32, secs: u64, speaker: &str, text: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "start_ms": secs * 1000, "end_ms": secs * 1000 + 900, "text": text,
            "source": if speaker == "You" { "microphone" } else { "loopback" },
            "speaker_id": speaker, "chunk_id": 0
        })
    }

    fn hello(status: &str, id: &str, segments: Vec<serde_json::Value>) -> String {
        serde_json::json!({
            "event": "hello", "status": status, "meeting_id": id, "title": "Standup",
            "started_at": "2026-11-11T14:00:00Z", "chunk_count": 1,
            "diarization": {"enabled": true, "backend": "simple"}, "segments": segments
        })
        .to_string()
    }

    fn ev(v: serde_json::Value) -> String {
        v.to_string()
    }

    fn texts(v: &MeetingView) -> Vec<String> {
        v.lines()
            .iter()
            .map(|s| format!("{} {}", s.speaker_display(), s.text))
            .collect()
    }

    #[test]
    fn hello_loads_a_live_meeting_in_start_order() {
        let mut v = MeetingView::live();
        let c = v.apply_line(&hello(
            "recording",
            "m1",
            vec![seg(1, 9, "Remote", "late"), seg(0, 2, "You", "early")],
        ));
        assert_eq!(c, Change::Reset);
        assert!(v.connected);
        assert_eq!(v.state, ViewState::Recording);
        assert_eq!(v.title.as_deref(), Some("Standup"));
        assert_eq!(texts(&v), vec!["You early", "Remote late"]);
    }

    #[test]
    fn segments_interleave_by_start_time_and_dedupe_by_id() {
        let mut v = MeetingView::live();
        v.apply_line(&hello("recording", "m1", vec![seg(0, 1, "You", "a")]));
        v.apply_line(&ev(
            serde_json::json!({"event":"segments","chunk_id":1,"segments":[seg(2, 30, "You", "c")]}),
        ));
        // The other track's chunk for the same stretch arrives after.
        v.apply_line(&ev(
            serde_json::json!({"event":"segments","chunk_id":1,"segments":[seg(1, 20, "Remote", "b"), seg(2, 30, "You", "c")]}),
        ));
        assert_eq!(texts(&v), vec!["You a", "Remote b", "You c"]);
    }

    #[test]
    fn a_segment_update_replaces_the_line_in_place() {
        let mut v = MeetingView::live();
        v.apply_line(&hello(
            "recording",
            "m1",
            vec![seg(0, 1, "Remote 1", "x"), seg(1, 5, "Remote 1", "y")],
        ));
        let mut moved = seg(1, 5, "Remote 2", "y");
        moved["original_speaker"] = "Remote 1".into();
        moved["annotation"] = serde_json::json!({"text": "check", "highlight": true});
        assert_eq!(
            v.apply_line(&ev(
                serde_json::json!({"event":"segment_updated","segment": moved})
            )),
            Change::Lines
        );
        assert_eq!(texts(&v), vec!["Remote 1 x", "Remote 2 y"]);
        assert_eq!(v.lines()[1].annotation.as_ref().unwrap().text, "check");
        assert_eq!(v.lines().len(), 2);
    }

    #[test]
    fn a_rename_reaches_every_line_and_moved_lines() {
        let mut v = MeetingView::live();
        let mut moved = seg(1, 5, "Person 1", "y");
        moved["original_speaker"] = "Remote".into();
        v.apply_line(&hello(
            "recording",
            "m1",
            vec![seg(0, 1, "Remote", "x"), moved],
        ));
        v.apply_line(r#"{"event":"speaker_renamed","from":"Remote","to":"Remote 1"}"#);
        assert_eq!(v.lines()[0].speaker_key(), "Remote 1");
        assert_eq!(v.lines()[1].original_speaker.as_deref(), Some("Remote 1"));
    }

    #[test]
    fn chips_merge_speakers_shown_under_one_name() {
        let mut v = MeetingView::live();
        let mut a = seg(0, 1, "Remote 1", "x");
        a["speaker_label"] = "Dana".into();
        let mut b = seg(1, 3, "Remote 2", "y");
        b["speaker_label"] = "Dana".into();
        v.apply_line(&hello(
            "recording",
            "m1",
            vec![a, seg(2, 2, "You", "z"), b, seg(3, 4, "Remote 3", " ")],
        ));
        let chips = v.speakers();
        assert_eq!(
            chips,
            vec![
                SpeakerChip {
                    name: "Dana".into(),
                    keys: vec!["Remote 1".into(), "Remote 2".into()],
                    lines: 2,
                    first_ms: 1000
                },
                SpeakerChip {
                    name: "You".into(),
                    keys: vec!["You".into()],
                    lines: 1,
                    first_ms: 2000
                },
            ]
        );
        assert_eq!(v.speaker_index(&v.lines()[2]), Some(0));
    }

    #[test]
    fn the_state_machine_runs_record_pause_stop_summarize_end() {
        let mut v = MeetingView::live();
        v.apply_line(&hello("recording", "m1", vec![seg(0, 1, "You", "a")]));
        assert_eq!(
            v.apply_line(r#"{"event":"status","status":"paused"}"#),
            Change::State
        );
        assert_eq!(v.state, ViewState::Paused);
        v.apply_line(r#"{"event":"status","status":"recording"}"#);
        v.apply_line(r#"{"event":"status","status":"finalizing"}"#);
        assert_eq!(v.state, ViewState::Finalizing);
        v.apply_line(r#"{"event":"status","status":"idle"}"#);
        assert_eq!(v.state, ViewState::Ended);
        assert!(v.ended_at.is_some());
        // Summarizing names the meeting it is about.
        assert_eq!(
            v.apply_line(r#"{"event":"status","status":"summarizing","meeting_id":"other"}"#),
            Change::Nothing
        );
        v.apply_line(r#"{"event":"status","status":"summarizing","meeting_id":"m1"}"#);
        assert_eq!(v.state, ViewState::Summarizing);
        let ready = serde_json::json!({
            "event": "summary_ready", "meeting_id": "m1", "agent": "claude",
            "agent_name": "Claude", "path": "/m/summary.md",
            "summary": {"summary": "Done.", "generated_at": "2026-11-11T15:00:00Z"}
        });
        assert_eq!(v.apply_line(&ev(ready)), Change::Summary);
        assert_eq!(v.state, ViewState::Ended);
        let s = v.summary.as_ref().unwrap();
        assert_eq!(
            (s.agent_name.as_str(), s.summary.summary.as_str()),
            ("Claude", "Done.")
        );
        // Lines survive the whole way.
        assert_eq!(texts(&v), vec!["You a"]);
    }

    #[test]
    fn a_failed_summary_ends_with_the_error() {
        let mut v = MeetingView::live();
        v.apply_line(&hello("recording", "m1", vec![]));
        v.apply_line(r#"{"event":"status","status":"idle"}"#);
        v.apply_line(r#"{"event":"status","status":"summarizing","meeting_id":"m1"}"#);
        v.apply_line(r#"{"event":"error","message":"Summary failed: x","meeting_id":"other"}"#);
        assert_eq!(v.state, ViewState::Summarizing);
        v.apply_line(r#"{"event":"error","message":"Summary failed: x","meeting_id":"m1","code":"agent_failed"}"#);
        assert_eq!(v.state, ViewState::Ended);
        assert_eq!(v.last_error.as_deref(), Some("Summary failed: x"));
    }

    #[test]
    fn idle_hello_with_nothing_loaded_is_idle() {
        let mut v = MeetingView::live();
        assert_eq!(v.apply_line(&hello("idle", "", vec![])), Change::State);
        // hello idle carries no meeting id in practice
        let mut v = MeetingView::live();
        v.apply_line(r#"{"event":"hello","status":"idle","chunk_count":0,"segments":[]}"#);
        assert_eq!(v.state, ViewState::Idle);
        assert!(v.connected);
    }

    #[test]
    fn reconnecting_to_the_same_meeting_refreshes_from_the_snapshot() {
        let mut v = MeetingView::live();
        v.apply_line(&hello("recording", "m1", vec![seg(0, 1, "You", "a")]));
        v.disconnected();
        assert!(!v.connected);
        assert_eq!(texts(&v), vec!["You a"], "lines kept while away");
        // Missed a chunk and a pause while disconnected.
        v.apply_line(&hello(
            "paused",
            "m1",
            vec![seg(0, 1, "You", "a"), seg(1, 3, "Remote", "b")],
        ));
        assert!(v.connected);
        assert_eq!(v.state, ViewState::Paused);
        assert_eq!(texts(&v), vec!["You a", "Remote b"]);
    }

    #[test]
    fn a_meeting_that_ended_while_disconnected_shows_as_ended() {
        let mut v = MeetingView::live();
        v.apply_line(&hello("recording", "m1", vec![seg(0, 1, "You", "a")]));
        v.disconnected();
        v.apply_line(r#"{"event":"hello","status":"idle","chunk_count":0,"segments":[]}"#);
        assert_eq!(v.state, ViewState::Ended);
        assert_eq!(texts(&v), vec!["You a"]);
    }

    #[test]
    fn a_new_meeting_replaces_the_old_one() {
        let mut v = MeetingView::live();
        v.apply_line(&hello("recording", "m1", vec![seg(0, 1, "You", "old")]));
        v.apply_line(r#"{"event":"status","status":"idle"}"#);
        v.summary = None;
        // Live events for a meeting we have no hello for ask for a resync.
        assert_eq!(
            v.apply_line(r#"{"event":"status","status":"recording"}"#),
            Change::Resync
        );
        assert_eq!(
            v.apply_line(&ev(
                serde_json::json!({"event":"segments","chunk_id":0,"segments":[seg(0, 1, "You", "new")]})
            )),
            Change::Resync
        );
        assert_eq!(texts(&v), vec!["You old"]);
        // The resync's hello brings the new meeting.
        assert_eq!(
            v.apply_line(&hello("recording", "m2", vec![seg(0, 1, "You", "new")])),
            Change::Reset
        );
        assert_eq!(v.meeting_id.as_deref(), Some("m2"));
        assert_eq!(texts(&v), vec!["You new"]);
        assert!(v.ended_at.is_none());
    }

    #[test]
    fn notes_updates_are_tracked_for_this_meeting_only() {
        let mut v = MeetingView::live();
        v.apply_line(&hello("recording", "m1", vec![]));
        assert_eq!(
            v.apply_line(r#"{"event":"notes_updated","meeting_id":"m2"}"#),
            Change::Nothing
        );
        assert_eq!(
            v.apply_line(
                r#"{"event":"notes_updated","meeting_id":"m1","updated_at":"2026-11-11T14:05:00Z"}"#
            ),
            Change::Notes
        );
        assert!(v.notes_updated_at.is_some());
    }

    #[test]
    fn unknown_events_and_garbage_change_nothing() {
        let mut v = MeetingView::live();
        assert_eq!(
            v.apply_line(r#"{"event":"levels","rms":0.2}"#),
            Change::Nothing
        );
        assert_eq!(v.apply_line("not json"), Change::Nothing);
        assert_eq!(v.revision, 0);
    }

    #[test]
    fn a_stored_meeting_ignores_live_traffic_but_takes_its_summary() {
        let mut data = MeetingData::new(Some("Retro".into()));
        let mut s = TranscriptSegment::new(0, 1000, 2000, "hi".into(), 0);
        s.source = AudioSource::Microphone;
        data.transcript.add_segment(s);
        data.metadata.duration_secs = Some(65);
        let id = data.metadata.id.to_string();
        let mut v = MeetingView::stored(&data);
        assert_eq!(v.state, ViewState::Ended);
        assert!(!v.live);
        assert_eq!(v.elapsed(Utc::now()), Some(Duration::from_secs(65)));

        assert_eq!(
            v.apply_line(&hello("recording", "live", vec![seg(5, 1, "You", "live")])),
            Change::Nothing
        );
        assert_eq!(
            v.apply_line(r#"{"event":"status","status":"recording"}"#),
            Change::Nothing
        );
        assert_eq!(texts(&v), vec!["You hi"]);

        v.apply_line(&format!(
            r#"{{"event":"status","status":"summarizing","meeting_id":"{id}"}}"#
        ));
        assert_eq!(v.state, ViewState::Summarizing);
        let ready = serde_json::json!({
            "event": "summary_ready", "meeting_id": id, "agent": "claude", "agent_name": "Claude",
            "summary": {"summary": "S", "generated_at": "2026-11-11T15:00:00Z"}
        });
        assert_eq!(v.apply_line(&ev(ready)), Change::Summary);
        assert_eq!(v.state, ViewState::Ended);
    }

    #[test]
    fn elapsed_counts_from_the_start_while_live() {
        let mut v = MeetingView::live();
        v.apply_line(&hello("recording", "m1", vec![]));
        let now: DateTime<Utc> = "2026-11-11T14:02:05Z".parse().unwrap();
        assert_eq!(v.elapsed(now), Some(Duration::from_secs(125)));
        assert_eq!(clock(Duration::from_secs(125)), "02:05");
        assert_eq!(clock(Duration::from_secs(3725)), "1:02:05");
    }
}

/// Speaker colors, by chip index, for views that color names. Picked to
/// read on light and dark backgrounds.
pub const SPEAKER_COLORS: [&str; 8] = [
    "#1c71d8", "#c061cb", "#e5a50a", "#2ec27e", "#3584e4", "#e01b24", "#26a269", "#9141ac",
];

/// Escape text for Pango markup.
pub fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// One transcript line as Pango markup: dimmed time, the speaker's name in
/// their color, the text, and the annotation on a second line.
pub fn line_markup(view: &MeetingView, segment: &TranscriptSegment) -> String {
    let color = view
        .speaker_index(segment)
        .map(|i| SPEAKER_COLORS[i % SPEAKER_COLORS.len()])
        .unwrap_or("#808080");
    let mut m = format!(
        "<span alpha=\"60%\">[{}]</span> <b><span foreground=\"{}\">{}</span></b>: {}",
        clock(Duration::from_millis(segment.start_ms)),
        color,
        escape_markup(&segment.speaker_display()),
        escape_markup(segment.text.trim())
    );
    if let Some(a) = &segment.annotation {
        m.push_str(&format!(
            "\n<i><span foreground=\"#c64600\">✎ {}</span></i>",
            escape_markup(&crate::meeting::export::annotation_line(a))
        ));
    }
    m
}

#[cfg(test)]
mod markup_tests {
    use super::*;

    #[test]
    fn markup_escapes_and_colors_by_speaker() {
        let mut v = MeetingView::live();
        v.apply_line(
            &serde_json::json!({
                "event": "hello", "status": "recording", "meeting_id": "m", "segments": [
                    {"id": 0, "start_ms": 61000, "end_ms": 62000, "text": "a < b & c", "source": "loopback",
                     "speaker_id": "Remote", "speaker_label": "Dana <PM>", "chunk_id": 0,
                     "annotation": {"text": "check", "highlight": true}}
                ]
            })
            .to_string(),
        );
        let m = line_markup(&v, &v.lines()[0]);
        assert_eq!(
            m,
            "<span alpha=\"60%\">[01:01]</span> <b><span foreground=\"#1c71d8\">Dana &lt;PM&gt;</span></b>: a &lt; b &amp; c\n<i><span foreground=\"#c64600\">✎ Highlight: check</span></i>"
        );
    }
}
