//! Meeting detection: notice a call starting and offer to record it.
//!
//! A call shows up as a call app opening a microphone capture stream in
//! PipeWire. Native clients (Zoom, Teams) count on that alone; a browser
//! opens the mic for many things, so a browser stream only counts when a
//! window title confirms a call ("Meet - ...", "| Microsoft Teams"). Window
//! titles come from Hyprland and are skipped on other compositors.
//!
//! [`Detector`] is the pure decision logic: it takes [`Snapshot`]s of the
//! system and emits [`Event`]s, prompting once per call after a debounce.
//! The live loop around it (PipeWire watching, notifications, starting the
//! meeting) is in [`runner`].

pub mod hyprland;
#[cfg(target_os = "linux")]
pub mod notify;
pub mod pipewire;
#[cfg(target_os = "linux")]
pub mod runner;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::config::{CallAppPattern, MeetingDetectConfig};

/// An open microphone capture stream (`Stream/Input/Audio` node).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MicStream {
    pub id: u32,
    pub app_name: String,
    pub binary: String,
    pub node_name: String,
    pub media_name: String,
    pub pid: Option<u32>,
}

impl MicStream {
    /// The name shown in explanations: the app name, else the binary.
    pub fn label(&self) -> &str {
        [&self.app_name, &self.binary, &self.node_name]
            .into_iter()
            .find(|s| !s.is_empty())
            .map(String::as_str)
            .unwrap_or("unknown")
    }

    fn matches_any(&self, needles: &[String]) -> bool {
        let fields = [
            self.app_name.to_lowercase(),
            self.binary.to_lowercase(),
            self.node_name.to_lowercase(),
        ];
        needles.iter().any(|n| {
            let n = n.to_lowercase();
            fields.iter().any(|f| !f.is_empty() && f.contains(&n))
        })
    }

    /// Why this stream must never count as a call, if it must not: voxtype's
    /// own capture (dictation or a meeting), other dictation tools, and the
    /// camera-effects virtual mic that is always open.
    pub fn ignore_reason(&self) -> Option<&'static str> {
        if self.pid.is_some_and(|p| p == std::process::id()) {
            return Some("voxtype's own capture");
        }
        let fields = [
            self.app_name.to_lowercase(),
            self.binary.to_lowercase(),
            self.node_name.to_lowercase(),
        ];
        let has = |n: &str| fields.iter().any(|f| f.contains(n));
        if has("voxtype") {
            Some("voxtype's own capture")
        } else if has("superwhisper") || has("[hex]") || self.binary == "hex" {
            Some("another dictation app")
        } else if has("camera-effects") {
            Some("camera-effects virtual mic")
        } else {
            None
        }
    }
}

/// A toplevel window (from Hyprland).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Window {
    pub class: String,
    pub title: String,
    pub pid: Option<u32>,
}

/// What the detector sees at one moment.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub streams: Vec<MicStream>,
    /// `None` when window titles are unavailable (not on Hyprland). Browser
    /// calls then cannot be confirmed; native clients still are.
    pub windows: Option<Vec<Window>>,
}

/// voxtype's own activity, which can hold a prompt back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activity {
    /// A meeting is recording, paused, finalizing or about to start.
    pub meeting_active: bool,
    /// Push-to-talk dictation is using the mic.
    pub dictating: bool,
}

/// A call the detector believes is in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// The app pattern's `name` ("Zoom", "Google Meet").
    pub app: String,
    /// The title for the meeting: the call window's title without the
    /// browser's suffix, else the app name.
    pub title: String,
    /// Human-readable evidence, for `meeting suggest`.
    pub reason: String,
    /// Recorded without asking (`auto_record_apps`).
    pub auto_record: bool,
}

/// One line of explanation about a stream that did not become a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note(pub String);

/// The calls in a snapshot, plus notes on every stream that was set aside.
pub fn find_calls(snapshot: &Snapshot, config: &MeetingDetectConfig) -> (Vec<Call>, Vec<Note>) {
    let mut calls: Vec<Call> = Vec::new();
    let mut notes = Vec::new();
    for stream in &snapshot.streams {
        if let Some(why) = stream.ignore_reason() {
            notes.push(Note(format!(
                "ignored mic stream {} ({}): {}",
                stream.id,
                stream.label(),
                why
            )));
            continue;
        }
        let apps: Vec<&CallAppPattern> = config
            .apps
            .iter()
            .filter(|a| stream.matches_any(&a.streams))
            .collect();
        if apps.is_empty() {
            notes.push(Note(format!(
                "mic stream {} ({}) is not a call app",
                stream.id,
                stream.label()
            )));
            continue;
        }
        let mut confirmed = false;
        for app in &apps {
            if calls.iter().any(|c| c.app == app.name) {
                confirmed = true;
                continue;
            }
            if let Some(call) = confirm(stream, app, snapshot.windows.as_deref(), config) {
                calls.push(call);
                confirmed = true;
            }
        }
        if !confirmed {
            let why = if snapshot.windows.is_none() {
                "window titles are unavailable (not on Hyprland)".to_string()
            } else {
                let wanted: Vec<String> = apps
                    .iter()
                    .flat_map(|a| a.titles.iter().map(|t| format!("\"{}\"", t)))
                    .collect();
                format!("no window title contains {}", wanted.join(" or "))
            };
            notes.push(Note(format!(
                "mic stream {} ({}) has the mic, but {}",
                stream.id,
                stream.label(),
                why
            )));
        }
    }
    (calls, notes)
}

/// Turn a stream matched to `app` into a call, if the app's window evidence
/// (when it needs any) is present.
fn confirm(
    stream: &MicStream,
    app: &CallAppPattern,
    windows: Option<&[Window]>,
    config: &MeetingDetectConfig,
) -> Option<Call> {
    let auto_record = config
        .auto_record_apps
        .iter()
        .any(|a| a.trim().eq_ignore_ascii_case(&app.name));
    let windows = windows.unwrap_or(&[]);
    if app.titles.is_empty() {
        // Native client: the mic is enough. Title from its own window,
        // preferring one that looks like a meeting.
        let own: Vec<&Window> = windows
            .iter()
            .filter(|w| {
                (stream.pid.is_some() && w.pid == stream.pid)
                    || app
                        .streams
                        .iter()
                        .any(|s| w.class.to_lowercase().contains(&s.to_lowercase()))
            })
            .filter(|w| !w.title.trim().is_empty())
            .collect();
        let window = own
            .iter()
            .find(|w| w.title.to_lowercase().contains("meeting"))
            .or(own.first());
        let title = window
            .map(|w| clean_title(&w.title))
            .unwrap_or_else(|| app.name.clone());
        return Some(Call {
            app: app.name.clone(),
            title,
            reason: format!("{} has the mic (stream {})", stream.label(), stream.id),
            auto_record,
        });
    }
    let window = windows.iter().find(|w| {
        let t = w.title.to_lowercase();
        app.titles.iter().any(|p| t.contains(&p.to_lowercase()))
    })?;
    Some(Call {
        app: app.name.clone(),
        title: clean_title(&window.title),
        reason: format!(
            "{} has the mic (stream {}) and window \"{}\" matches",
            stream.label(),
            stream.id,
            window.title
        ),
        auto_record,
    })
}

const BROWSER_SUFFIXES: &[&str] = &[
    "Chromium",
    "Google Chrome",
    "Mozilla Firefox",
    "Firefox",
    "Brave",
    "Vivaldi",
    "Microsoft Edge",
    "Zen Browser",
    "LibreWolf",
];

/// A window title as a meeting title: drop the browser's " - Chromium"
/// suffix and an unread-count prefix like "(2) ".
pub fn clean_title(title: &str) -> String {
    let mut t = title.trim();
    for suffix in BROWSER_SUFFIXES {
        for sep in [" - ", " — ", " – "] {
            if let Some(rest) = t.strip_suffix(&format!("{}{}", sep, suffix)) {
                t = rest.trim_end();
            }
        }
    }
    if let Some(rest) = t.strip_prefix('(') {
        if let Some((count, after)) = rest.split_once(") ") {
            if !count.is_empty() && count.chars().all(|c| c.is_ascii_digit()) {
                t = after;
            }
        }
    }
    t.trim().to_string()
}

/// Something the detector decided. The live loop acts on these; `meeting
/// suggest` prints them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A call appeared; it must last the debounce before anything happens.
    Seen { call: Call, settle: Duration },
    /// Ask the user whether to record.
    Prompt(Call),
    /// Start recording without asking (`auto_record_apps`).
    AutoRecord(Call),
    /// The call settled, but voxtype will not prompt (yet).
    Held { call: Call, why: String },
    /// The mic closed again before the debounce: a permission check or a
    /// device test, not a call.
    Brief { call: Call },
    /// The call app released the mic. `offer_stop` is set when a meeting is
    /// recording, so the user can be offered to stop it (never automatic).
    Ended { call: Call, offer_stop: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting out the debounce.
    Settling,
    /// Settled, but dictation holds the prompt back until it ends.
    WaitingForDictation,
    /// Prompted, recorded, or deliberately skipped. Never prompts again.
    Done,
}

#[derive(Debug, Clone)]
struct Track {
    call: Call,
    first_seen: Instant,
    last_seen: Instant,
    phase: Phase,
}

/// Prompts once per call, after the call has held the mic for the debounce.
pub struct Detector {
    config: MeetingDetectConfig,
    debounce: Duration,
    tracks: BTreeMap<String, Track>,
}

impl Detector {
    pub fn new(config: MeetingDetectConfig) -> Self {
        let debounce = Duration::from_secs(u64::from(config.debounce_secs));
        Self::with_debounce(config, debounce)
    }

    pub fn with_debounce(config: MeetingDetectConfig, debounce: Duration) -> Self {
        Self {
            config,
            debounce,
            tracks: BTreeMap::new(),
        }
    }

    pub fn config(&self) -> &MeetingDetectConfig {
        &self.config
    }

    /// True while a call is being tracked, so the caller keeps re-checking
    /// (debounce timers, title changes) instead of waiting for a PipeWire
    /// change.
    pub fn is_tracking(&self) -> bool {
        !self.tracks.is_empty()
    }

    /// Feed one snapshot; returns what changed.
    pub fn evaluate(
        &mut self,
        snapshot: &Snapshot,
        activity: Activity,
        now: Instant,
    ) -> Vec<Event> {
        let (calls, _) = find_calls(snapshot, &self.config);
        let mut events = Vec::new();

        for call in calls {
            match self.tracks.get_mut(&call.app) {
                Some(track) => {
                    track.last_seen = now;
                    if track.phase != Phase::Done {
                        // Titles settle as the call page loads.
                        track.call = call;
                    }
                }
                None => {
                    events.push(Event::Seen {
                        call: call.clone(),
                        settle: self.debounce,
                    });
                    self.tracks.insert(
                        call.app.clone(),
                        Track {
                            call,
                            first_seen: now,
                            last_seen: now,
                            phase: Phase::Settling,
                        },
                    );
                }
            }
        }

        let mut ended = Vec::new();
        for (app, track) in self.tracks.iter_mut() {
            let present = track.last_seen == now;
            if !present {
                match track.phase {
                    Phase::Settling | Phase::WaitingForDictation => {
                        if track.phase == Phase::Settling {
                            events.push(Event::Brief {
                                call: track.call.clone(),
                            });
                        }
                        ended.push(app.clone());
                    }
                    // A short gap (device switch, reconnect) is not the end
                    // of the call; wait the debounce before saying it is.
                    Phase::Done if now.duration_since(track.last_seen) >= self.debounce => {
                        events.push(Event::Ended {
                            call: track.call.clone(),
                            offer_stop: activity.meeting_active,
                        });
                        ended.push(app.clone());
                    }
                    Phase::Done => {}
                }
                continue;
            }
            if track.phase == Phase::Done || now.duration_since(track.first_seen) < self.debounce {
                continue;
            }
            if activity.meeting_active {
                events.push(Event::Held {
                    call: track.call.clone(),
                    why: "a meeting is already recording".to_string(),
                });
                track.phase = Phase::Done;
            } else if activity.dictating {
                if track.phase != Phase::WaitingForDictation {
                    events.push(Event::Held {
                        call: track.call.clone(),
                        why: "dictation is using the mic; will ask when it ends".to_string(),
                    });
                    track.phase = Phase::WaitingForDictation;
                }
            } else {
                track.phase = Phase::Done;
                events.push(if track.call.auto_record {
                    Event::AutoRecord(track.call.clone())
                } else {
                    Event::Prompt(track.call.clone())
                });
            }
        }
        for app in ended {
            self.tracks.remove(&app);
        }
        events
    }
}

/// Describe an event as one line, for `meeting suggest`.
pub fn describe(event: &Event) -> String {
    match event {
        Event::Seen { call, settle } => format!(
            "call seen: {} - {}; prompting if it lasts {}s",
            call.app,
            call.reason,
            settle.as_secs()
        ),
        Event::Prompt(call) => format!("would prompt: Record \"{}\"? ({})", call.title, call.app),
        Event::AutoRecord(call) => format!(
            "would start recording \"{}\" without asking ({} is in auto_record_apps)",
            call.title, call.app
        ),
        Event::Held { call, why } => format!("not prompting for {}: {}", call.app, why),
        Event::Brief { call } => format!(
            "ignored a brief mic open by {} (shorter than the debounce)",
            call.app
        ),
        Event::Ended { call, offer_stop } => {
            if *offer_stop {
                format!(
                    "call ended: {} released the mic; would offer \"Stop meeting?\"",
                    call.app
                )
            } else {
                format!("call ended: {} released the mic", call.app)
            }
        }
    }
}

/// What `meeting suggest --dry-run` prints for one snapshot: each stream
/// set aside and why, then the decision, as if every call had already
/// lasted the debounce.
pub fn report(
    snapshot: &Snapshot,
    activity: Activity,
    config: &MeetingDetectConfig,
) -> Vec<String> {
    let mut lines = Vec::new();
    match &snapshot.windows {
        Some(w) => lines.push(format!("windows: {} from Hyprland", w.len())),
        None => lines.push(
            "windows: unavailable (not on Hyprland); browser calls cannot be confirmed".into(),
        ),
    }
    lines.push(format!("mic streams: {}", snapshot.streams.len()));
    let (_, notes) = find_calls(snapshot, config);
    lines.extend(notes.into_iter().map(|n| format!("  {}", n.0)));
    let mut detector = Detector::with_debounce(config.clone(), Duration::ZERO);
    let events: Vec<Event> = detector
        .evaluate(snapshot, activity, Instant::now())
        .into_iter()
        .filter(|e| !matches!(e, Event::Seen { .. }))
        .collect();
    if events.is_empty() {
        lines.push("no prompt: no call detected".into());
    }
    for event in &events {
        if let Event::Prompt(call) | Event::AutoRecord(call) | Event::Held { call, .. } = event {
            lines.push(format!("evidence: {}", call.reason));
        }
        lines.push(describe(event));
    }
    lines
}

/// Read voxtype's own activity from the runtime files the daemon writes.
pub fn read_activity(
    runtime_dir: &std::path::Path,
    state_file: Option<&std::path::Path>,
) -> Activity {
    let meeting = std::fs::read_to_string(runtime_dir.join("meeting_state")).unwrap_or_default();
    let meeting_active = matches!(
        meeting.lines().next().map(str::trim),
        Some("recording" | "paused" | "finalizing")
    ) || runtime_dir.join("meeting_start").exists();
    let dictating = state_file
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|s| matches!(s.trim(), "recording" | "streaming"));
    Activity {
        meeting_active,
        dictating,
    }
}

#[cfg(test)]
mod tests;
