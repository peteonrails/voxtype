//! Audio settings: input device, max duration, media behavior, feedback sounds.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
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
use super::theme;
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
                    message: format!("Couldn't read the config: {}", e),
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
                    message: format!("Not saved: {}", e),
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
                    message: "Put back the saved settings".to_string(),
                });
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("Couldn't read the config: {}", e),
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

/// `300` seconds reads as `5 min`.
fn duration_name(secs: u32) -> String {
    if secs >= 60 && secs.is_multiple_of(60) {
        format!("{} min", secs / 60)
    } else {
        format!("{} s", secs)
    }
}

/// `default` reads as `Default`; a custom theme folder stays as written.
fn theme_name(theme: &str) -> String {
    match theme {
        "default" => "Default".to_string(),
        "subtle" => "Subtle".to_string(),
        "mechanical" => "Mechanical".to_string(),
        other => other.to_string(),
    }
}

/// The page's rows, from its in-memory settings.
pub fn rows(state: &AudioState) -> Vec<FormRowSpec> {
    let themes: Vec<String> = THEME_CHOICES.iter().map(|t| theme_name(t)).collect();
    vec![
        FormRowSpec::new(
            state.field == Field::Device,
            "Microphone",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::Device => e.input.caret_string(),
                _ if state.device == "default" => "System default".to_string(),
                _ => state.device.clone(),
            },
        ),
        FormRowSpec::new(
            state.field == Field::MaxDuration,
            "Longest recording",
            duration_name(state.max_duration_secs),
        ),
        FormRowSpec::new(
            state.field == Field::PauseMedia,
            "Pause media",
            yesno(state.pause_media),
        ),
        FormRowSpec::new(
            state.field == Field::DuckMedia,
            "Lower media volume",
            yesno(state.duck_media),
        ),
        FormRowSpec::new(
            state.field == Field::DuckMediaVolume,
            "Lowered to",
            format!("{}%", state.duck_media_volume_percent),
        )
        .dimmed(!state.duck_media),
        FormRowSpec::new(
            state.field == Field::DuckMediaFade,
            "Fade",
            if state.duck_media_fade_ms == 0 {
                "Instant".to_string()
            } else {
                format!("{} ms", state.duck_media_fade_ms)
            },
        )
        .dimmed(!state.duck_media),
        FormRowSpec::new(
            state.field == Field::FeedbackEnabled,
            "Sounds",
            yesno(state.feedback_enabled),
        ),
        FormRowSpec::new(
            state.field == Field::FeedbackTheme,
            "Sound theme",
            theme_name(&state.feedback_theme),
        )
        .choices(&themes)
        .dimmed(!state.feedback_enabled),
        FormRowSpec::new(
            state.field == Field::FeedbackVolume,
            "Sound volume",
            format!("{:.0}%", state.feedback_volume * 100.0),
        )
        .dimmed(!state.feedback_enabled),
    ]
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.audio else {
        common::render_load_error(f, area, app);
        return;
    };
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
        app,
        feedback_pair,
        &rows(state),
        guidance_for_field(state),
    );
}

fn yesno(b: bool) -> &'static str {
    if b {
        "On"
    } else {
        "Off"
    }
}

fn guidance_for_field(state: &AudioState) -> Vec<Line<'_>> {
    let text = |s: &'static str| Line::from(s);
    let dim = |s: &'static str| Line::from(Span::styled(s, theme::muted()));
    match state.field {
        Field::Device => {
            let count = state.device_choices.len().saturating_sub(1);
            let scan = match state.scan_status {
                ScanStatus::Scanning => dim("Looking for microphones…"),
                ScanStatus::Failed => Line::from(Span::styled(
                    "Couldn't list microphones. Enter types a device name.",
                    theme::warn(),
                )),
                ScanStatus::Done => Line::from(Span::styled(
                    format!(
                        "{} microphone{} found. ←→ goes through them; Enter types a name.",
                        count,
                        if count == 1 { "" } else { "s" }
                    ),
                    theme::muted(),
                )),
            };
            vec![
                text(
                    "System default follows whatever your desktop uses, so it moves \
                     with a headset you plug in. Pick one to stay on that microphone.",
                ),
                scan,
            ]
        }
        Field::MaxDuration => vec![text(
            "A recording left running stops here and is transcribed. A few \
             minutes suits dictation; raise it for long notes.",
        )],
        Field::PauseMedia => vec![text(
            "Pauses music and video players while you record and resumes them \
             after.",
        )],
        Field::DuckMedia => vec![text(
            "Turns playing media down while you record instead of pausing it, \
             and back up after.",
        )],
        Field::DuckMediaVolume => vec![text(
            "How loud media plays while you record, relative to its volume now.",
        )],
        Field::DuckMediaFade => vec![text(
            "How long the volume takes to go down and come back. It never \
             delays the microphone.",
        )],
        Field::FeedbackEnabled => vec![text(
            "Short sounds when recording starts and stops, for when you aren't \
             looking at the screen.",
        )],
        Field::FeedbackTheme => vec![
            text("Default is a soft chime, Subtle quiet taps, Mechanical sharp clicks."),
            dim("A folder of .wav files also works; set it in the config file."),
        ],
        Field::FeedbackVolume => vec![text(
            "The lowest you can still hear over your typing is usually right.",
        )],
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
