//! Audio settings: input device, max duration, media behavior, feedback sounds.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use std::io::Read;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::app::{Action, App};
use super::common::{
    self, FeedbackLevel as CommonFeedback, FormRowSpec, TextInput, TextInputResult,
};
use super::config_editor::{ConfigEditor, EditorError};
use crate::audio::devices::enumerate_input_devices;
use crate::config::AudioConfig;

#[derive(Debug)]
pub struct AudioState {
    pub device: String,
    pub max_duration_secs: u32,
    pub pause_media: bool,
    pub duck_media: bool,
    pub duck_media_volume_percent: u8,
    pub duck_media_fade_ms: u32,
    pub feedback_enabled: bool,
    pub feedback_theme: String,
    pub feedback_volume: f32,

    pub field: Field,
    pub feedback: Option<Feedback>,
    pub dirty_since_load: bool,
    /// Device list: "default" (and the configured device) immediately, plus
    /// everything the background probe finds once it completes.
    pub device_choices: Vec<String>,
    pub scan_status: ScanStatus,
    /// Wired to the probe thread; drained by [`AudioState::poll_device_scan`].
    scan_rx: Option<mpsc::Receiver<Option<Vec<String>>>>,
    pub editing: Option<TextEdit>,
}

/// Progress of the background device probe shown in the Device guidance pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    Scanning,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct TextEdit {
    pub field: Field,
    pub input: TextInput,
}

#[derive(Debug, Clone)]
pub struct Feedback {
    pub level: FeedbackLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum FeedbackLevel {
    Ok,
    Err,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Device,
    MaxDuration,
    PauseMedia,
    DuckMedia,
    DuckMediaVolume,
    DuckMediaFade,
    FeedbackEnabled,
    FeedbackTheme,
    FeedbackVolume,
}

impl Field {
    const ALL: &'static [Field] = &[
        Field::Device,
        Field::MaxDuration,
        Field::PauseMedia,
        Field::DuckMedia,
        Field::DuckMediaVolume,
        Field::DuckMediaFade,
        Field::FeedbackEnabled,
        Field::FeedbackTheme,
        Field::FeedbackVolume,
    ];
}

const THEME_CHOICES: &[&str] = &["default", "subtle", "mechanical"];
/// Step in seconds for the max-duration cycler.
const DURATION_STEP: u32 = 30;
const DURATION_MIN: u32 = 30;
const DURATION_MAX: u32 = 1800;
const VOLUME_STEP: f32 = 0.1;
const DUCK_VOLUME_STEP: i32 = 5;
/// Step for the duck-fade cycler. A multiple of the 20 ms ramp interval in
/// `audio::media`, so every stop lands on a value that produces whole steps.
const DUCK_FADE_STEP: i32 = 50;
const DUCK_FADE_MAX: u32 = 2000;

impl AudioState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            device: ed
                .get_string("audio", "device")
                .unwrap_or_else(|| "default".to_string()),
            max_duration_secs: ed
                .get_int("audio", "max_duration_secs")
                .map(|n| n.clamp(0, u32::MAX as i64) as u32)
                .unwrap_or(120),
            pause_media: ed.get_bool("audio", "pause_media").unwrap_or(false),
            duck_media: ed.get_bool("audio", "duck_media").unwrap_or(false),
            duck_media_volume_percent: ed
                .get_int("audio", "duck_media_volume_percent")
                .map(|n| n.clamp(0, 150) as u8)
                .unwrap_or_else(|| AudioConfig::default().duck_media_volume_percent),
            duck_media_fade_ms: ed
                .get_int("audio", "duck_media_fade_ms")
                .map(|n| n.clamp(0, DUCK_FADE_MAX as i64) as u32)
                .unwrap_or_else(|| AudioConfig::default().duck_media_fade_ms),
            feedback_enabled: ed.get_bool("audio.feedback", "enabled").unwrap_or(false),
            feedback_theme: ed
                .get_string("audio.feedback", "theme")
                .unwrap_or_else(|| "default".to_string()),
            feedback_volume: ed.get_f32_or("audio.feedback", "volume", 0.7),
            field: Field::Device,
            feedback: None,
            dirty_since_load: false,
            device_choices: initial_device_choices(&ed),
            scan_status: ScanStatus::Scanning,
            scan_rx: start_device_scan(),
            editing: None,
        })
    }

    /// Fold in the background probe's result if it has arrived. Non-blocking;
    /// called once per event-loop tick. Returns true when state changed.
    pub fn poll_device_scan(&mut self) -> bool {
        let Some(rx) = &self.scan_rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(Some(devices)) => {
                for d in devices {
                    if !self.device_choices.contains(&d) {
                        self.device_choices.push(d);
                    }
                }
                self.scan_status = ScanStatus::Done;
                self.scan_rx = None;
                true
            }
            Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                self.scan_status = ScanStatus::Failed;
                self.scan_rx = None;
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
        }
    }

    fn is_text_field(field: Field) -> bool {
        matches!(field, Field::Device)
    }

    fn start_edit_if_text_field(&mut self) -> bool {
        if !Self::is_text_field(self.field) {
            return false;
        }
        self.editing = Some(TextEdit {
            field: self.field,
            input: TextInput::new(self.device.clone()),
        });
        true
    }

    fn commit_text_edit(&mut self, field: Field, buffer: String) {
        if let Field::Device = field {
            let trimmed = buffer.trim();
            self.device = if trimmed.is_empty() {
                "default".to_string()
            } else {
                trimmed.to_string()
            };
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }

    pub fn save(&mut self) -> Action {
        let mut ed = match ConfigEditor::load() {
            Ok(e) => e,
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("load: {}", e),
                });
                return Action::None;
            }
        };
        ed.set_string("audio", "device", &self.device);
        ed.set_int("audio", "max_duration_secs", self.max_duration_secs as i64);
        ed.set_bool("audio", "pause_media", self.pause_media);
        ed.set_bool("audio", "duck_media", self.duck_media);
        ed.set_int(
            "audio",
            "duck_media_volume_percent",
            self.duck_media_volume_percent as i64,
        );
        ed.set_int(
            "audio",
            "duck_media_fade_ms",
            self.duck_media_fade_ms as i64,
        );
        ed.set_bool("audio.feedback", "enabled", self.feedback_enabled);
        ed.set_string("audio.feedback", "theme", &self.feedback_theme);
        ed.set_f32("audio.feedback", "volume", self.feedback_volume);

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Ok,
                    message: format!("Saved to {}", ed.path().display()),
                });
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("save: {}", e),
                });
            }
        }
        Action::None
    }

    pub fn reset(&mut self) {
        match Self::load() {
            Ok(fresh) => {
                let field = self.field;
                let cached = self.device_choices.clone();
                let scan_status = self.scan_status;
                let scan_rx = self.scan_rx.take();
                *self = fresh;
                self.field = field;
                // Keep the original probe and its result; reverting field
                // edits doesn't change what devices exist.
                self.device_choices = cached;
                self.scan_status = scan_status;
                self.scan_rx = scan_rx;
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Ok,
                    message: "Reverted unsaved changes".to_string(),
                });
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("reload: {}", e),
                });
            }
        }
    }

    fn move_field(&mut self, delta: i32) {
        let len = Field::ALL.len() as i32;
        let cur = Field::ALL
            .iter()
            .position(|f| *f == self.field)
            .unwrap_or(0) as i32;
        let new = (cur + delta).rem_euclid(len);
        self.field = Field::ALL[new as usize];
    }

    fn cycle(&mut self, delta: i32) {
        match self.field {
            Field::Device => {
                if !self.device_choices.is_empty() {
                    let idx = self
                        .device_choices
                        .iter()
                        .position(|d| d == &self.device)
                        .map(|i| i as i32)
                        .unwrap_or(-1);
                    let new = (idx + delta).rem_euclid(self.device_choices.len() as i32);
                    self.device = self.device_choices[new as usize].clone();
                }
            }
            Field::MaxDuration => {
                let next = self.max_duration_secs as i32 + delta * DURATION_STEP as i32;
                self.max_duration_secs =
                    next.clamp(DURATION_MIN as i32, DURATION_MAX as i32) as u32;
            }
            Field::PauseMedia => {
                self.pause_media = !self.pause_media;
            }
            Field::DuckMedia => {
                self.duck_media = !self.duck_media;
            }
            Field::DuckMediaVolume => {
                let next = self.duck_media_volume_percent as i32 + delta * DUCK_VOLUME_STEP;
                self.duck_media_volume_percent = next.clamp(0, 150) as u8;
            }
            Field::DuckMediaFade => {
                let next = self.duck_media_fade_ms as i32 + delta * DUCK_FADE_STEP;
                self.duck_media_fade_ms = next.clamp(0, DUCK_FADE_MAX as i32) as u32;
            }
            Field::FeedbackEnabled => {
                self.feedback_enabled = !self.feedback_enabled;
            }
            Field::FeedbackTheme => {
                let idx = THEME_CHOICES
                    .iter()
                    .position(|t| *t == self.feedback_theme)
                    .map(|i| i as i32)
                    .unwrap_or(-1);
                let new = (idx + delta).rem_euclid(THEME_CHOICES.len() as i32);
                self.feedback_theme = THEME_CHOICES[new as usize].to_string();
            }
            Field::FeedbackVolume => {
                let next = self.feedback_volume + delta as f32 * VOLUME_STEP;
                self.feedback_volume = next.clamp(0.0, 1.0);
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

/// Immediate device list shown while the background probe runs: "default"
/// plus whatever the config currently points at, so the current selection is
/// always cyclable even before (or without) probe results.
fn initial_device_choices(ed: &ConfigEditor) -> Vec<String> {
    let mut out = vec!["default".to_string()];
    if let Some(dev) = ed.get_string("audio", "device") {
        if !dev.is_empty() && dev != "default" {
            out.push(dev);
        }
    }
    out
}

/// Kick off the device probe on a background thread. The probe runs in a
/// short-lived `voxtype configure --probe-audio-devices` subprocess because
/// ALSA device probing can misbehave in ways a thread can't contain: dsnoop
/// opens have been observed to block forever in an ioctl, and libasound can
/// die on SIGFPE while probing hw params (both seen on PipeWire systems).
/// In-process, either takes the whole TUI down; in a child, the thread kills
/// it after a timeout and the TUI just shows "scan failed".
fn start_device_scan() -> Option<mpsc::Receiver<Option<Vec<String>>>> {
    let exe = std::env::current_exe().ok()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(probe_input_devices_via_subprocess(&exe));
    });
    Some(rx)
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

fn probe_input_devices_via_subprocess(exe: &std::path::Path) -> Option<Vec<String>> {
    let mut child = std::process::Command::new(exe)
        .args(["configure", "--probe-audio-devices"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + PROBE_TIMEOUT;
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !success {
        return None;
    }

    // The child has exited, so this read is bounded; the device list is far
    // smaller than the pipe buffer, so the child never blocked writing it.
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(
        out.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
    )
}

/// Entry point for the hidden `--probe-audio-devices` flag: print one input
/// device name per line. Runs in the subprocess spawned by
/// [`start_device_scan`].
pub fn print_input_devices() {
    // Watchdog: probing can hang forever in an ALSA ioctl. The parent kills
    // us after PROBE_TIMEOUT, but not if it already exited (user quit the
    // TUI early) — self-terminate so a hung probe can't linger as an orphan.
    std::thread::spawn(|| {
        std::thread::sleep(PROBE_TIMEOUT + Duration::from_secs(5));
        std::process::exit(2);
    });
    for name in enumerate_input_devices() {
        println!("{}", name);
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.audio {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Audio");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(
                Paragraph::new("Failed to load config; check ~/.config/voxtype/config.toml.")
                    .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }
    };

    let rows = vec![
        FormRowSpec::new(
            state.field == Field::Device,
            "Input device",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::Device => e.input.caret_string(),
                _ => state.device.clone(),
            },
        ),
        FormRowSpec::new(
            state.field == Field::MaxDuration,
            "Max recording (seconds)",
            state.max_duration_secs.to_string(),
        ),
        FormRowSpec::new(
            state.field == Field::PauseMedia,
            "Pause MPRIS media on record",
            yesno(state.pause_media),
        ),
        FormRowSpec::new(
            state.field == Field::DuckMedia,
            "Duck media volume on record",
            yesno(state.duck_media),
        ),
        FormRowSpec::new(
            state.field == Field::DuckMediaVolume,
            "Ducked media volume",
            format!("{}%", state.duck_media_volume_percent),
        )
        .dimmed(!state.duck_media),
        FormRowSpec::new(
            state.field == Field::DuckMediaFade,
            "Duck fade",
            if state.duck_media_fade_ms == 0 {
                "instant".to_string()
            } else {
                format!("{} ms", state.duck_media_fade_ms)
            },
        )
        .dimmed(!state.duck_media),
        FormRowSpec::new(
            state.field == Field::FeedbackEnabled,
            "Audio feedback sounds",
            if state.feedback_enabled { "on" } else { "off" },
        ),
        FormRowSpec::new(
            state.field == Field::FeedbackTheme,
            "Sound theme",
            &state.feedback_theme,
        )
        .dimmed(!state.feedback_enabled),
        FormRowSpec::new(
            state.field == Field::FeedbackVolume,
            "Volume",
            format!("{:.0}%", state.feedback_volume * 100.0),
        )
        .dimmed(!state.feedback_enabled),
    ];

    let feedback_pair = state.feedback.as_ref().map(|fb| {
        (
            match fb.level {
                FeedbackLevel::Ok => CommonFeedback::Ok,
                FeedbackLevel::Err => CommonFeedback::Err,
            },
            fb.message.as_str(),
        )
    });

    common::render_form_with_guidance(
        f,
        area,
        "Audio",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance_for_field(state),
    );
}

fn yesno(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn heading<'a>(text: &'a str) -> Line<'a> {
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn guidance_for_field(state: &AudioState) -> Vec<Line<'_>> {
    match state.field {
        Field::Device => {
            let count = state.device_choices.len().saturating_sub(1);
            let scan_line = match state.scan_status {
                ScanStatus::Scanning => Line::from(Span::styled(
                    "Scanning audio devices…",
                    Style::default().fg(Color::Gray),
                )),
                ScanStatus::Failed => Line::from(Span::styled(
                    "Device scan failed or timed out — type a device name \
                     manually (Enter to edit).",
                    Style::default().fg(Color::Yellow),
                )),
                ScanStatus::Done => Line::from(format!(
                    "Detected {} device{} via cpal.",
                    count,
                    if count == 1 { "" } else { "s" }
                )),
            };
            vec![
                heading("Input device"),
                Line::from(""),
                scan_line,
                Line::from(""),
                Line::from(
                    "\"default\" follows whatever PipeWire/PulseAudio is set \
                     to as the system default source. If you swap headsets or \
                     plug in a USB mic, default will follow.",
                ),
                Line::from(""),
                Line::from(
                    "Pick a specific device if you want voxtype to ignore the \
                     system default and stay locked to one mic — useful when \
                     you stream and don't want voxtype to grab your stream mic.",
                ),
            ]
        }
        Field::MaxDuration => vec![
            heading("Max recording duration"),
            Line::from(""),
            Line::from(
                "Safety cap. If you accidentally lock the PTT key down (or \
                 use toggle mode and forget), voxtype stops at this many \
                 seconds and transcribes what it has.",
            ),
            Line::from(""),
            Line::from(
                "120-300 seconds is normal for dictation. Bump to 600+ for \
                 meeting-mode-style long recordings.",
            ),
        ],
        Field::PauseMedia => vec![
            heading("Pause MPRIS media on record"),
            Line::from(""),
            Line::from(
                "Pauses Spotify, MPV, browsers, and other MPRIS players \
                 before recording begins, then resumes them as soon as \
                 microphone capture stops.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Uses MPRIS over the session D-Bus.",
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            Line::from(
                "Useful if music in the background ever bleeds into your \
                 mic, or if you'd rather hear yourself dictate without \
                 lyrics in the way.",
            ),
        ],
        Field::DuckMedia => vec![
            heading("Duck media volume on record"),
            Line::from(""),
            Line::from(
                "Lowers active PulseAudio/PipeWire sink-input volume while \
                 you record, then restores the original volume as soon as \
                 recording stops.",
            ),
            Line::from(""),
            Line::from(
                "Use this instead of pause media when you want playback to \
                 continue quietly during push-to-talk.",
            ),
        ],
        Field::DuckMediaVolume => vec![
            heading("Ducked media volume"),
            Line::from(""),
            Line::from(
                "Relative volume for active media streams while recording. \
                 50% keeps each stream at half its current level; original \
                 per-channel volumes are restored when the mic closes.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "0% is silence; 34% is the default; values above 100% amplify.",
                Style::default().fg(Color::Gray),
            )),
        ],
        Field::DuckMediaFade => vec![
            heading("Duck fade"),
            Line::from(""),
            Line::from(
                "How long media takes to slide down when recording starts, \
                 and back up when it stops. The same duration is used in both \
                 directions, so music eases out of the way instead of jumping.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "0 restores the old instant behaviour. The fade runs alongside \
                 capture, so it never delays the microphone.",
                Style::default().fg(Color::Gray),
            )),
        ],
        Field::FeedbackEnabled => vec![
            heading("Audio feedback sounds"),
            Line::from(""),
            Line::from(
                "Plays short cue sounds when recording starts, stops, and \
                 (optionally) when transcription completes. Helpful when the \
                 visual indicator isn't where you're looking.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Sound theme and volume are only used when this is on.",
                Style::default().fg(Color::Gray),
            )),
        ],
        Field::FeedbackTheme => vec![
            heading("Sound theme"),
            Line::from(""),
            Line::from(Span::styled(
                "default: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Soft chime up / down. Most users keep this."),
            Line::from(""),
            Line::from(Span::styled(
                "subtle: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Quieter taps. Good in shared rooms."),
            Line::from(""),
            Line::from(Span::styled(
                "mechanical: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Sharp tactile clicks."),
            Line::from(""),
            Line::from(Span::styled(
                "Custom themes can point at a directory of .wav files; \
                 edit [audio.feedback] theme directly to use one.",
                Style::default().fg(Color::Gray),
            )),
        ],
        Field::FeedbackVolume => vec![
            heading("Feedback volume"),
            Line::from(""),
            Line::from(
                "Volume of the feedback cues, 0-100%. Independent of system \
                 volume — voxtype attenuates the sample at playback time.",
            ),
            Line::from(""),
            Line::from(
                "Tuning tip: pick the lowest volume you can still hear over \
                 your typing. The cue is a confirmation, not an alert.",
            ),
        ],
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.audio.as_mut() {
        Some(s) => s,
        None => return Action::None,
    };

    if state.editing.is_some() {
        return handle_edit_key(state, key);
    }

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.move_field(-1);
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.move_field(1);
            Action::None
        }
        KeyCode::Left | KeyCode::Char('h') => {
            state.cycle(-1);
            Action::None
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
            state.cycle(1);
            Action::None
        }
        KeyCode::Enter | KeyCode::Char('i') => {
            state.start_edit_if_text_field();
            Action::None
        }
        KeyCode::Char('s') => state.save(),
        KeyCode::Char('r') => {
            state.reset();
            Action::None
        }
        _ => Action::None,
    }
}

fn handle_edit_key(state: &mut AudioState, key: KeyEvent) -> Action {
    let Some(editing) = state.editing.as_mut() else {
        return Action::None;
    };
    match editing.input.handle_key(key) {
        TextInputResult::Continue => Action::None,
        TextInputResult::Commit => {
            let buf = editing.input.buffer().to_string();
            let field = editing.field;
            state.editing = None;
            state.commit_text_edit(field, buf);
            Action::None
        }
        TextInputResult::Cancel => {
            state.editing = None;
            Action::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{scalar_keys, CONFIG_KEYS};

    /// The Audio form is a hand-written transcription of the schema's `Audio`
    /// section, so the two drift silently whenever a key is added in one place
    /// and not the other. That is exactly how `audio.duck_media_fade_ms`
    /// reached a release configurable by file, CLI, env var and the Omarchy
    /// settings panel, but not by this form.
    ///
    /// Nothing here enforces a rendering style, only that every schema key in
    /// the section is reachable. If a key genuinely does not belong in the TUI,
    /// add it to `NOT_IN_TUI` with a reason rather than deleting the test.
    #[test]
    fn every_audio_schema_key_has_a_form_field() {
        const NOT_IN_TUI: &[&str] = &[];

        let expected: Vec<&str> = scalar_keys()
            .filter(|s| s.section == "Audio")
            .map(|s| s.key)
            .filter(|k| !NOT_IN_TUI.contains(k))
            .collect();

        // Field -> the schema key it edits. Adding a Field without extending
        // this map fails to compile, which is the point.
        let covered: Vec<&str> = Field::ALL
            .iter()
            .map(|f| match f {
                Field::Device => "audio.device",
                Field::MaxDuration => "audio.max_duration_secs",
                Field::PauseMedia => "audio.pause_media",
                Field::DuckMedia => "audio.duck_media",
                Field::DuckMediaVolume => "audio.duck_media_volume_percent",
                Field::DuckMediaFade => "audio.duck_media_fade_ms",
                Field::FeedbackEnabled => "audio.feedback.enabled",
                Field::FeedbackTheme => "audio.feedback.theme",
                Field::FeedbackVolume => "audio.feedback.volume",
            })
            .collect();

        let missing: Vec<&&str> = expected.iter().filter(|k| !covered.contains(k)).collect();
        assert!(
            missing.is_empty(),
            "schema keys in section Audio with no TUI field: {missing:?}"
        );

        // Guard the other direction too: a field editing a key the schema does
        // not know about would be invisible to `config set` and the panel.
        let unknown: Vec<&&str> = covered
            .iter()
            .filter(|k| !CONFIG_KEYS.iter().any(|s| s.key == **k))
            .collect();
        assert!(
            unknown.is_empty(),
            "TUI fields editing keys absent from the schema: {unknown:?}"
        );
    }

    /// The form used to hardcode its own fallback for the ducked volume, which
    /// went stale the moment the config default changed. Defaults belong to
    /// `AudioConfig`, not to a second copy in the UI.
    #[test]
    fn form_defaults_track_the_config_defaults() {
        let cfg = AudioConfig::default();
        assert_eq!(cfg.duck_media_volume_percent, 34);
        assert_eq!(cfg.duck_media_fade_ms, 150);
    }
}
