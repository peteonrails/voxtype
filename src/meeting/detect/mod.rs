//! Meeting detection: notice a call starting and offer to record it.
//!
//! A call shows up as a call app opening a microphone capture stream in
//! PipeWire. Native clients (Zoom, Teams) count on that alone; a browser
//! opens the mic for many things, so a browser stream only counts when a
//! window confirms a call: by its title ("Meet - ...", "| Microsoft
//! Teams") or by its class, when the call has a window of its own (the Zoom
//! web client, `chrome-app.zoom.us__wc_join_...`). Windows come from
//! Hyprland and are skipped on other compositors.
//!
//! Both signals come and go during a call. A browser's window title is its
//! front tab's, so it stops matching when another tab is brought forward.
//! A call app may also let go of the mic for a while. So once a call has
//! settled it goes on while either signal is left (see [`Evidence`]), and
//! it is over when the mic is released and no window shows the call.
//!
//! The Zoom web client's window is titled with the meeting's name, so no
//! title finds it: only the "Join from Zoom Workplace app" tab that
//! launched it has a title that says Zoom, and that tab is neither in
//! front for long nor closed when the call ends. Its class finds the
//! window itself. Once a call's own window has been seen, a title no
//! longer speaks for that call: when the window closes and the mic is
//! released, the call is over, whatever tab is in front.
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
    /// A window title confirmed it (a browser call); `false` for a native
    /// client, which counts on its mic stream alone.
    pub by_window: bool,
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
                let quoted = |list: &[String]| -> Vec<String> {
                    list.iter().map(|t| format!("\"{}\"", t)).collect()
                };
                let titles: Vec<String> = apps.iter().flat_map(|a| quoted(&a.titles)).collect();
                let classes: Vec<String> = apps.iter().flat_map(|a| quoted(&a.classes)).collect();
                let mut why = Vec::new();
                if !titles.is_empty() {
                    why.push(format!("no window title contains {}", titles.join(" or ")));
                }
                if !classes.is_empty() {
                    why.push(format!("no window class contains {}", classes.join(" or ")));
                }
                why.join(" and ")
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

/// The microphone streams of the calls in a snapshot, each with its call:
/// every stream not set aside by [`MicStream::ignore_reason`] whose app is a
/// call [`find_calls`] confirms. The recording announcement plays into these.
pub fn call_streams(snapshot: &Snapshot, config: &MeetingDetectConfig) -> Vec<(Call, MicStream)> {
    let (calls, _) = find_calls(snapshot, config);
    let mut out = Vec::new();
    for stream in &snapshot.streams {
        if stream.ignore_reason().is_some() {
            continue;
        }
        let call = calls.iter().find(|call| {
            config
                .apps
                .iter()
                .any(|a| a.name == call.app && stream.matches_any(&a.streams))
        });
        if let Some(call) = call {
            out.push((call.clone(), stream.clone()));
        }
    }
    out
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
    if !app.needs_window() {
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
        let title = meeting_title(&app.name, window.map(|w| w.title.as_str()));
        return Some(Call {
            app: app.name.clone(),
            title,
            reason: format!("{} has the mic (stream {})", stream.label(), stream.id),
            auto_record,
            by_window: false,
        });
    }
    // The call's own window first: its title is the meeting's, where a
    // title match may only be the tab that launched the call.
    if let Some((window, class)) = windows
        .iter()
        .find_map(|w| class_confirms(app, w).map(|class| (w, class)))
    {
        return Some(Call {
            app: app.name.clone(),
            title: meeting_title(&app.name, Some(&window.title)),
            // The pattern, not the class: the class holds the meeting's
            // number, which has no place in the journal.
            reason: format!(
                "{} has the mic (stream {}) and its call window \"{}\" is open (class contains \"{}\")",
                stream.label(),
                stream.id,
                window.title,
                class
            ),
            auto_record,
            by_window: true,
        });
    }
    let window = windows.iter().find(|w| title_confirms(app, w))?;
    Some(Call {
        app: app.name.clone(),
        title: meeting_title(&app.name, Some(&window.title)),
        reason: format!(
            "{} has the mic (stream {}) and window \"{}\" matches",
            stream.label(),
            stream.id,
            window.title
        ),
        auto_record,
        by_window: true,
    })
}

/// Whether a window title confirms a call of `app`.
fn title_confirms(app: &CallAppPattern, window: &Window) -> bool {
    let t = window.title.to_lowercase();
    app.titles.iter().any(|p| t.contains(&p.to_lowercase()))
}

/// The class pattern of `app` that makes `window` the call's own window,
/// if one does.
fn class_confirms<'a>(app: &'a CallAppPattern, window: &Window) -> Option<&'a str> {
    let class = window.class.to_lowercase();
    app.classes
        .iter()
        .find(|p| class.contains(&p.to_lowercase()))
        .map(String::as_str)
}

/// Whether the snapshot shows a window of `app`'s call itself, found by
/// its class.
pub fn has_own_window(app: &CallAppPattern, snapshot: &Snapshot) -> bool {
    snapshot
        .windows
        .as_deref()
        .is_some_and(|windows| windows.iter().any(|w| class_confirms(app, w).is_some()))
}

/// What a snapshot shows of a call that has already settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// Everything that made it a call: the mic and, for a browser call, a
    /// window.
    Full,
    /// The app let go of the mic, but a window still shows the call.
    WindowOnly,
    /// The app still has the mic, but no window shows the call: its tab is
    /// behind another one, or was renamed or closed.
    MicOnly,
    /// A browser call whose windows cannot be read right now. Not knowing
    /// is not the window being gone, so the call goes on. `mic` is whether
    /// the app still has the mic.
    WindowsUnknown { mic: bool },
    /// The mic is released and no window shows the call.
    Gone,
}

/// The [`Evidence`] for a call of `app` in `snapshot`.
///
/// `own_window_only` is for a call whose own window has been seen
/// ([`has_own_window`]): only that window shows it then, not a title. The
/// tab that launched a Zoom call keeps its "Join from Zoom Workplace app"
/// title long after the call's window has closed.
pub fn evidence(app: &CallAppPattern, snapshot: &Snapshot, own_window_only: bool) -> Evidence {
    let mic = snapshot
        .streams
        .iter()
        .any(|s| s.ignore_reason().is_none() && s.matches_any(&app.streams));
    if !app.needs_window() {
        // A native client has no second signal: its mic is all there is.
        return if mic { Evidence::Full } else { Evidence::Gone };
    }
    let window = snapshot.windows.as_deref().map(|windows| {
        windows.iter().any(|w| {
            class_confirms(app, w).is_some() || (!own_window_only && title_confirms(app, w))
        })
    });
    match (mic, window) {
        (true, Some(true)) => Evidence::Full,
        (true, Some(false)) => Evidence::MicOnly,
        (false, Some(true)) => Evidence::WindowOnly,
        (false, Some(false)) => Evidence::Gone,
        (mic, None) => Evidence::WindowsUnknown { mic },
    }
}

/// Which service a call app is, for its titles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Service {
    Zoom,
    Meet,
    Teams,
    Other,
}

impl Service {
    fn of(app: &str) -> Self {
        let a = app.to_lowercase();
        if a.contains("zoom") {
            Self::Zoom
        } else if a.contains("meet") {
            Self::Meet
        } else if a.contains("teams") {
            Self::Teams
        } else {
            Self::Other
        }
    }
}

/// Window titles that name the app, not the meeting: "Zoom Meeting",
/// "Join from Zoom Workplace app - Zoom", "Calling | Microsoft Teams".
const ZOOM_GENERIC: &[&str] = &[
    "",
    "zoom",
    "zoom meeting",
    "zoom workplace",
    "zoom cloud meetings",
    "zoom webinar",
    "meeting",
    "webinar",
    "join meeting",
    "launch meeting",
];
const MEET_GENERIC: &[&str] = &["", "meet", "google meet"];
const TEAMS_GENERIC: &[&str] = &[
    "",
    "teams",
    "microsoft teams",
    "microsoft teams (work or school)",
    "new microsoft teams",
    "meeting",
    "meeting in progress",
    "meeting compact view",
    "call",
    "calling",
    "chat",
    "activity",
    "calendar",
    "teams and channels",
];

/// Cut a case-insensitive suffix such as " - Zoom" off `t`.
fn without_suffix<'a>(t: &'a str, suffixes: &[&str]) -> &'a str {
    let lower = t.to_lowercase();
    for s in suffixes {
        if lower.ends_with(s) && t.is_char_boundary(t.len() - s.len()) {
            return t[..t.len() - s.len()].trim_end();
        }
    }
    t
}

/// "abc-defg-hij": a Meet meeting code, which says nothing to a person.
fn is_meet_code(t: &str) -> bool {
    let parts: Vec<&str> = t.split('-').collect();
    parts.len() == 3
        && [3, 4, 3]
            .iter()
            .zip(&parts)
            .all(|(n, p)| p.len() == *n && p.chars().all(|c| c.is_ascii_lowercase()))
}

/// The meeting title for a call: the meeting's own name when the window
/// title holds one ("Meet - Weekly sync", "Weekly sync | Microsoft Teams"),
/// otherwise "Zoom call", "Google Meet call" or "Teams call", never a
/// placeholder such as "Join from Zoom Workplace app - Zoom".
pub fn meeting_title(app: &str, window_title: Option<&str>) -> String {
    let service = Service::of(app);
    let cleaned = window_title.map(clean_title).unwrap_or_default();
    let real = match service {
        Service::Zoom => {
            let t = without_suffix(&cleaned, &[" - zoom", " | zoom", " – zoom"]).trim();
            let lower = t.to_lowercase();
            // "app.zoom.us_/wc/join/...": the web client's window before
            // its page has a title.
            (!ZOOM_GENERIC.contains(&lower.as_str())
                && !lower.contains("join from zoom")
                && !lower.contains("zoom workplace")
                && !lower.contains("zoom.us"))
            .then(|| t.to_string())
        }
        Service::Meet => {
            let t = without_suffix(&cleaned, &[" - google meet", " | google meet"]).trim();
            let t = ["Meet - ", "Meet – ", "Meet: ", "Meet "]
                .iter()
                .find_map(|p| t.strip_prefix(p))
                .unwrap_or(t)
                .trim();
            let lower = t.to_lowercase();
            (!MEET_GENERIC.contains(&lower.as_str()) && !is_meet_code(&lower))
                .then(|| t.to_string())
        }
        Service::Teams => {
            let lower_all = cleaned.to_lowercase();
            let cut = [" | microsoft teams", " - microsoft teams"]
                .iter()
                .filter_map(|s| lower_all.find(s))
                .min()
                .unwrap_or(cleaned.len());
            let t = cleaned[..cut].trim();
            let lower = t.to_lowercase();
            (!TEAMS_GENERIC.contains(&lower.as_str())).then(|| t.to_string())
        }
        Service::Other => (!cleaned.is_empty()).then_some(cleaned.clone()),
    };
    real.unwrap_or_else(|| match service {
        Service::Zoom => "Zoom call".to_string(),
        Service::Meet => "Google Meet call".to_string(),
        Service::Teams => "Teams call".to_string(),
        Service::Other => format!("{} call", app.trim()),
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
    /// A settled call lost part of its evidence, or got it back, and goes
    /// on. Sent when what is seen of it changes.
    Continues { call: Call, evidence: Evidence },
    /// The call is over: its app released the mic and, for a browser call,
    /// no window shows it any more. `offer_stop` is set when a meeting is
    /// recording, so `on_call_end` can stop it or offer to.
    Ended { call: Call, offer_stop: bool },
    /// A call that had just ended is back within the grace period: the
    /// same call, so nothing is offered again and a waiting stop is off.
    Returned { call: Call },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting out the debounce.
    Settling,
    /// Settled, but dictation holds the prompt back until it ends.
    WaitingForDictation,
    /// Prompted, recorded, or deliberately skipped. Never prompts again.
    Done,
    /// Over since `at`, and remembered for the grace period: if its app
    /// takes the mic again or its window comes back in that time, it is the
    /// same call, not a new one to prompt for.
    Ended { at: Instant },
}

#[derive(Debug, Clone)]
struct Track {
    call: Call,
    first_seen: Instant,
    /// When there was last any evidence of the call
    last_seen: Instant,
    phase: Phase,
    /// What was last seen of it, so a change is reported once
    evidence: Evidence,
    /// The call's own window has been seen (by its class), so titles no
    /// longer count as its window
    own_window: bool,
}

/// Prompts once per call, after the call has held the mic for the debounce.
pub struct Detector {
    config: MeetingDetectConfig,
    debounce: Duration,
    /// How long an ended call is remembered (`stop_grace_secs`)
    grace: Duration,
    tracks: BTreeMap<String, Track>,
}

impl Detector {
    pub fn new(config: MeetingDetectConfig) -> Self {
        let debounce = Duration::from_secs(u64::from(config.debounce_secs));
        Self::with_debounce(config, debounce)
    }

    pub fn with_debounce(config: MeetingDetectConfig, debounce: Duration) -> Self {
        let grace = Duration::from_secs(u64::from(config.stop_grace_secs));
        Self {
            config,
            debounce,
            grace,
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
                    if matches!(track.phase, Phase::Settling | Phase::WaitingForDictation) {
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
                            evidence: Evidence::Full,
                            own_window: false,
                        },
                    );
                }
            }
        }

        let mut ended = Vec::new();
        for (app, track) in self.tracks.iter_mut() {
            // Every track is of an app in the config.
            let pattern = self.config.apps.iter().find(|a| a.name == *app);
            if pattern.is_some_and(|a| has_own_window(a, snapshot)) {
                track.own_window = true;
            }
            let seen = pattern.map_or(Evidence::Gone, |a| evidence(a, snapshot, track.own_window));
            match track.phase {
                // Until it has settled, a call needs all of its evidence:
                // a browser with the mic and no call window is no call yet.
                Phase::Settling | Phase::WaitingForDictation => {
                    if seen != Evidence::Full {
                        if track.phase == Phase::Settling {
                            events.push(Event::Brief {
                                call: track.call.clone(),
                            });
                        }
                        ended.push(app.clone());
                        continue;
                    }
                    track.last_seen = now;
                    if now.duration_since(track.first_seen) < self.debounce {
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
                                why: "dictation is using the mic; will ask when it ends"
                                    .to_string(),
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
                Phase::Done if seen == Evidence::Gone => {
                    // A short gap (device switch, reconnect) is not the end
                    // of the call; wait the debounce before saying it is.
                    if now.duration_since(track.last_seen) < self.debounce {
                        continue;
                    }
                    events.push(Event::Ended {
                        call: track.call.clone(),
                        offer_stop: activity.meeting_active,
                    });
                    if self.grace.is_zero() {
                        ended.push(app.clone());
                    } else {
                        track.phase = Phase::Ended { at: now };
                    }
                }
                Phase::Done => {
                    track.last_seen = now;
                    if seen != track.evidence {
                        track.evidence = seen;
                        events.push(Event::Continues {
                            call: track.call.clone(),
                            evidence: seen,
                        });
                    }
                }
                Phase::Ended { at } => {
                    if seen != Evidence::Gone {
                        track.phase = Phase::Done;
                        track.last_seen = now;
                        track.evidence = seen;
                        events.push(Event::Returned {
                            call: track.call.clone(),
                        });
                    } else if now.duration_since(at) >= self.grace {
                        ended.push(app.clone());
                    }
                }
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
        Event::Continues { call, evidence } => {
            let saw = match evidence {
                Evidence::Full if call.by_window => "has the mic and its call window again",
                Evidence::Full => "has the mic again",
                Evidence::WindowOnly => {
                    "released the mic, but its call window is still open (muted, or reconnecting)"
                }
                Evidence::MicOnly => {
                    "still has the mic, but no window shows the call any more \
                     (another tab is in front, or its window was closed)"
                }
                Evidence::WindowsUnknown { mic: true } => {
                    "still has the mic; window titles cannot be read right now"
                }
                Evidence::WindowsUnknown { mic: false } => {
                    "released the mic, and window titles cannot be read right now, \
                     so the call is not taken to be over"
                }
                Evidence::Gone => "left no sign",
            };
            format!("call continues: {} {}", call.app, saw)
        }
        Event::Ended { call, offer_stop } => {
            let gone = if call.by_window {
                "released the mic and no window shows the call any more"
            } else {
                "released the mic"
            };
            if *offer_stop {
                format!(
                    "call ended: {} {} while a meeting records ([meeting.detect] on_call_end decides)",
                    call.app, gone
                )
            } else {
                format!("call ended: {} {}", call.app, gone)
            }
        }
        Event::Returned { call } => format!(
            "call is back: {} is seen again within the grace period; the same call, so nothing is offered again",
            call.app
        ),
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
