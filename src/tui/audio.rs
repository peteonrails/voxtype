//! Audio settings: input device, max duration, feedback sounds, MPRIS pause.

use cpal::traits::{DeviceTrait, HostTrait};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::{
    self, FeedbackLevel as CommonFeedback, FormRowSpec, TextInput, TextInputResult,
};
use super::config_editor::{ConfigEditor, EditorError};

#[derive(Debug, Clone)]
pub struct AudioState {
    pub device: String,
    pub max_duration_secs: u32,
    pub pause_media: bool,
    pub feedback_enabled: bool,
    pub feedback_theme: String,
    pub feedback_volume: f32,

    pub field: Field,
    pub feedback: Option<Feedback>,
    pub dirty_since_load: bool,
    /// Cached device list (default + everything cpal finds). Loaded once.
    pub device_choices: Vec<String>,
    pub editing: Option<TextEdit>,
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
    FeedbackEnabled,
    FeedbackTheme,
    FeedbackVolume,
}

impl Field {
    const ALL: &'static [Field] = &[
        Field::Device,
        Field::MaxDuration,
        Field::PauseMedia,
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
            feedback_enabled: ed
                .get_bool("audio.feedback", "enabled")
                .unwrap_or(false),
            feedback_theme: ed
                .get_string("audio.feedback", "theme")
                .unwrap_or_else(|| "default".to_string()),
            feedback_volume: ed
                .get_string("audio.feedback", "volume")
                .and_then(|s| s.parse().ok())
                .or_else(|| {
                    ed.get_int("audio.feedback", "volume")
                        .map(|n| n as f32)
                })
                .unwrap_or(0.7),
            field: Field::Device,
            feedback: None,
            dirty_since_load: false,
            device_choices: enumerate_input_devices(),
            editing: None,
        })
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
        ed.set_int(
            "audio",
            "max_duration_secs",
            self.max_duration_secs as i64,
        );
        ed.set_bool("audio", "pause_media", self.pause_media);
        ed.set_bool("audio.feedback", "enabled", self.feedback_enabled);
        ed.set_string("audio.feedback", "theme", &self.feedback_theme);
        ed.set_string(
            "audio.feedback",
            "volume",
            &format!("{:.2}", self.feedback_volume),
        );

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
                *self = fresh;
                self.field = field;
                self.device_choices = cached;
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
        let cur = Field::ALL.iter().position(|f| *f == self.field).unwrap_or(0) as i32;
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

fn enumerate_input_devices() -> Vec<String> {
    // ALSA's PCM probing prints "Cannot open device /dev/dsp" and similar
    // messages to stderr for every device cpal touches. Inside the TUI's
    // alternate screen those lines paint over our frame and corrupt the
    // next redraw. Silence stderr for the duration of the probe.
    let _silenced = SilencedStderr::install();

    let mut out = vec!["default".to_string()];
    let host = cpal::default_host();
    if let Ok(devices) = host.input_devices() {
        for d in devices {
            if let Ok(name) = d.name() {
                if name != "default" && !out.contains(&name) {
                    out.push(name);
                }
            }
        }
    }
    out
}

/// RAII guard that redirects fd 2 (stderr) to /dev/null on construction and
/// restores the original fd on drop. Used to swallow noisy ALSA / cpal
/// stderr during device enumeration so it doesn't bleed into the TUI's
/// alternate screen.
struct SilencedStderr {
    saved_fd: Option<libc::c_int>,
}

impl SilencedStderr {
    fn install() -> Self {
        let null_fd = unsafe {
            libc::open(
                b"/dev/null\0".as_ptr() as *const libc::c_char,
                libc::O_WRONLY,
            )
        };
        if null_fd < 0 {
            return Self { saved_fd: None };
        }
        let saved = unsafe { libc::dup(libc::STDERR_FILENO) };
        if saved < 0 {
            unsafe { libc::close(null_fd) };
            return Self { saved_fd: None };
        }
        unsafe { libc::dup2(null_fd, libc::STDERR_FILENO) };
        unsafe { libc::close(null_fd) };
        Self {
            saved_fd: Some(saved),
        }
    }
}

impl Drop for SilencedStderr {
    fn drop(&mut self) {
        if let Some(saved) = self.saved_fd.take() {
            unsafe {
                libc::dup2(saved, libc::STDERR_FILENO);
                libc::close(saved);
            }
        }
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
            vec![
                heading("Input device"),
                Line::from(""),
                Line::from(format!(
                    "Detected {} device{} via cpal.",
                    count,
                    if count == 1 { "" } else { "s" }
                )),
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
                 while you record, then resumes them when transcription \
                 finishes.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Requires playerctl to be installed.",
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            Line::from(
                "Useful if music in the background ever bleeds into your \
                 mic, or if you'd rather hear yourself dictate without \
                 lyrics in the way.",
            ),
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
