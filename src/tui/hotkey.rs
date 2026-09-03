//! Hotkey settings: backend, PTT key, mode, cancel/modifier keys, listener enable.

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
use super::compositor_bindings;
use super::config_editor::{ConfigEditor, EditorError};
use crate::config::HotkeyBackend;
use strum::IntoEnumIterator;

/// In-memory copy of the hotkey state, owned by `App`. Edits mutate this; `s`
/// commits via [`ConfigEditor`] and rolls back on validation error.
#[derive(Debug, Clone)]
pub struct HotkeyState {
    pub backend: Backend,
    /// Whether `save` writes `hotkey.backend`. The shipped config leaves the
    /// key commented out so the compiled-in default applies, and saving an
    /// unrelated field must not pin it.
    pub backend_is_explicit: bool,
    pub key: String,
    pub mode: Mode,
    pub enabled: bool,
    pub cancel_key: Option<String>,
    pub modifier: Option<String>,
    /// Status banner shown after Save / Reset, cleared on the next edit.
    pub feedback: Option<Feedback>,
    pub dirty_since_load: bool,
    pub field: Field,
    pub editing: Option<TextEdit>,
}

#[derive(Debug, Clone)]
pub struct TextEdit {
    pub field: Field,
    pub input: TextInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    PushToTalk,
    Toggle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    Known(HotkeyBackend),
    /// A `hotkey.backend` value voxtype cannot parse. The daemon refuses to
    /// start on one, so the form shows it as written rather than presenting it
    /// as evdev and overwriting it on the next save.
    Unrecognised(String),
}

impl Backend {
    /// The string this choice is written as in the config file, or `None` for
    /// an unparseable value read from the file.
    fn config_value(&self) -> Option<&str> {
        match self {
            Self::Known(backend) => Some(backend.name()),
            Self::Unrecognised(_) => None,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Known(HotkeyBackend::Evdev) => "evdev (/dev/input)".to_string(),
            Self::Known(HotkeyBackend::Portal) => "XDG GlobalShortcuts portal".to_string(),
            Self::Known(HotkeyBackend::Auto) => "portal, then evdev if unavailable".to_string(),
            Self::Unrecognised(value) => format!("{}  (unrecognised)", value),
        }
    }

    /// The choice `delta` steps away in `HotkeyBackend`'s declaration order,
    /// wrapping at either end. An unrecognised value steps to the default.
    fn cycled(&self, delta: i32) -> Self {
        let Self::Known(current) = self else {
            return Self::Known(HotkeyBackend::default());
        };
        let choices: Vec<HotkeyBackend> = HotkeyBackend::iter().collect();
        let index = choices
            .iter()
            .position(|choice| choice == current)
            .unwrap_or(0);
        let next = (index as i32 + delta).rem_euclid(choices.len() as i32) as usize;

        Self::Known(choices[next])
    }
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
    Enabled,
    Backend,
    Key,
    Mode,
    CancelKey,
    Modifier,
}

impl Field {
    const ALL: &'static [Field] = &[
        Field::Enabled,
        Field::Backend,
        Field::Key,
        Field::Mode,
        Field::CancelKey,
        Field::Modifier,
    ];
}

/// Sensible PTT key choices, in order. Values match what voxtype's evdev
/// listener accepts (KEY_* names without the prefix).
const KEY_CHOICES: &[&str] = &[
    "HOME",
    "PAUSE",
    "SCROLLLOCK",
    "INSERT",
    "MENU",
    "F13",
    "F14",
    "F15",
    "F16",
    "RIGHTCTRL",
    "RIGHTALT",
    "RIGHTMETA",
    "CAPSLOCK",
];

const CANCEL_CHOICES: &[Option<&str>] = &[
    None,
    Some("ESC"),
    Some("BACKSPACE"),
    Some("F12"),
    Some("DELETE"),
    Some("END"),
];

const MODIFIER_CHOICES: &[Option<&str>] = &[
    None,
    Some("LEFTSHIFT"),
    Some("RIGHTSHIFT"),
    Some("LEFTCTRL"),
    Some("LEFTALT"),
    Some("LEFTMETA"),
];

impl HotkeyState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        let backend = ed.get_string("hotkey", "backend");
        Ok(Self {
            backend_is_explicit: backend.is_some(),
            backend: match backend.as_deref() {
                None => Backend::Known(HotkeyBackend::default()),
                Some(value) => value
                    .parse()
                    .map(Backend::Known)
                    .unwrap_or_else(|_| Backend::Unrecognised(value.to_string())),
            },
            key: ed
                .get_string("hotkey", "key")
                .unwrap_or_else(|| "HOME".to_string()),
            mode: match ed.get_string("hotkey", "mode").as_deref() {
                Some("toggle") => Mode::Toggle,
                _ => Mode::PushToTalk,
            },
            enabled: ed.get_bool("hotkey", "enabled").unwrap_or(true),
            cancel_key: ed.get_string("hotkey", "cancel_key"),
            modifier: ed.get_string("hotkey", "model_modifier"),
            feedback: None,
            dirty_since_load: false,
            field: Field::Enabled,
            editing: None,
        })
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
        ed.set_string("hotkey", "key", &self.key);
        if self.backend_is_explicit {
            // An unrecognised value is the only record of what the user typed,
            // so leave it in the file for them to correct.
            if let Some(value) = self.backend.config_value() {
                ed.set_string("hotkey", "backend", value);
            }
        }
        ed.set_string(
            "hotkey",
            "mode",
            match self.mode {
                Mode::PushToTalk => "push_to_talk",
                Mode::Toggle => "toggle",
            },
        );
        ed.set_bool("hotkey", "enabled", self.enabled);
        match &self.cancel_key {
            Some(k) => ed.set_string("hotkey", "cancel_key", k),
            None => ed.unset("hotkey", "cancel_key"),
        }
        match &self.modifier {
            Some(k) => ed.set_string("hotkey", "model_modifier", k),
            None => ed.unset("hotkey", "model_modifier"),
        }

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
                *self = fresh;
                self.field = field;
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

    /// Cycle the value of the focused field by `delta` (-1 for ← / +1 for →).
    fn is_text_field(field: Field) -> bool {
        // Free-text on Key / CancelKey / Modifier so users can type custom
        // KEY_* names that aren't in the curated cycle list.
        matches!(field, Field::Key | Field::CancelKey | Field::Modifier)
    }

    fn start_edit_if_text_field(&mut self) -> bool {
        // Edit only makes sense when the listener is enabled — otherwise
        // these fields are dimmed/inert.
        if !self.enabled || !Self::is_text_field(self.field) {
            return false;
        }
        let initial = match self.field {
            Field::Key => self.key.clone(),
            Field::CancelKey => self.cancel_key.clone().unwrap_or_default(),
            Field::Modifier => self.modifier.clone().unwrap_or_default(),
            _ => String::new(),
        };
        self.editing = Some(TextEdit {
            field: self.field,
            input: TextInput::new(initial),
        });
        true
    }

    fn commit_text_edit(&mut self, field: Field, buffer: String) {
        let trimmed = buffer.trim();
        match field {
            Field::Key if !trimmed.is_empty() => {
                self.key = trimmed.to_uppercase();
            }
            Field::CancelKey => {
                self.cancel_key = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_uppercase())
                };
            }
            Field::Modifier => {
                self.modifier = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_uppercase())
                };
            }
            _ => {}
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }

    fn cycle(&mut self, delta: i32) {
        // When the evdev listener is off, only the Enabled toggle responds —
        // the rest of the form is greyed out and inert.
        if !self.enabled && self.field != Field::Enabled {
            return;
        }
        match self.field {
            Field::Backend => {
                self.backend = self.backend.cycled(delta);
                self.backend_is_explicit = true;
            }
            Field::Key => {
                self.key = cycle_str(KEY_CHOICES, &self.key, delta);
            }
            Field::Mode => {
                self.mode = match self.mode {
                    Mode::PushToTalk => Mode::Toggle,
                    Mode::Toggle => Mode::PushToTalk,
                };
            }
            Field::CancelKey => {
                self.cancel_key = cycle_opt(CANCEL_CHOICES, self.cancel_key.as_deref(), delta);
            }
            Field::Modifier => {
                self.modifier = cycle_opt(MODIFIER_CHOICES, self.modifier.as_deref(), delta);
            }
            Field::Enabled => {
                self.enabled = !self.enabled;
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

fn cycle_str(choices: &[&'static str], current: &str, delta: i32) -> String {
    if choices.is_empty() {
        return current.to_string();
    }
    let idx = choices
        .iter()
        .position(|c| *c == current)
        .map(|i| i as i32)
        .unwrap_or(-1);
    let new = (idx + delta).rem_euclid(choices.len() as i32);
    choices[new as usize].to_string()
}

fn cycle_opt(
    choices: &[Option<&'static str>],
    current: Option<&str>,
    delta: i32,
) -> Option<String> {
    if choices.is_empty() {
        return current.map(|s| s.to_string());
    }
    let idx = choices
        .iter()
        .position(|c| c.as_deref() == current)
        .map(|i| i as i32)
        .unwrap_or(0);
    let new = (idx + delta).rem_euclid(choices.len() as i32);
    choices[new as usize].map(|s| s.to_string())
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.hotkey {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Hotkey");
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

    // Greyout fields after Enabled when the evdev listener is off — those
    // controls don't affect anything until the listener turns back on.
    let greyout = !state.enabled;

    let rows = vec![
        FormRowSpec::new(
            state.field == Field::Enabled,
            "Built-in hotkey listener",
            if state.enabled { "enabled" } else { "disabled" },
        ),
        FormRowSpec::new(
            state.field == Field::Backend,
            "Linux backend",
            state.backend.label(),
        )
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::Key,
            "Push-to-talk key",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::Key => e.input.caret_string(),
                _ => display_key(&state.key),
            },
        )
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::Mode,
            "Mode",
            match state.mode {
                Mode::PushToTalk => "Push-to-talk (hold)",
                Mode::Toggle => "Toggle (press to start/stop)",
            },
        )
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::CancelKey,
            "Cancel key",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::CancelKey => e.input.caret_string(),
                _ => state.cancel_key.as_deref().unwrap_or("(none)").to_string(),
            },
        )
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::Modifier,
            "Modifier (secondary model)",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::Modifier => e.input.caret_string(),
                _ => state.modifier.as_deref().unwrap_or("(none)").to_string(),
            },
        )
        .dimmed(greyout),
    ];

    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|fb| (to_common_level(fb.level), fb.message.as_str()));

    let guidance = guidance_for_field(state);

    common::render_form_with_guidance(
        f,
        area,
        "Hotkey",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance,
    );
}

fn to_common_level(level: FeedbackLevel) -> CommonFeedback {
    match level {
        FeedbackLevel::Ok => CommonFeedback::Ok,
        FeedbackLevel::Err => CommonFeedback::Err,
    }
}

/// Right-pane explanation for the focused field.
fn guidance_for_field(state: &HotkeyState) -> Vec<Line<'_>> {
    match state.field {
        Field::Enabled => guidance_enabled(state),
        Field::Backend => guidance_backend(state),
        Field::Key => guidance_key(state),
        Field::Mode => guidance_mode(state),
        Field::CancelKey => guidance_cancel(state),
        Field::Modifier => guidance_modifier(state),
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

fn guidance_enabled<'a>(state: &'a HotkeyState) -> Vec<Line<'a>> {
    let mut lines = vec![
        heading("Built-in hotkey listener"),
        Line::from(""),
        Line::from(
            "When enabled, voxtype starts the selected backend and receives \
             PTT shortcut events globally. No compositor binding is needed.",
        ),
        Line::from(""),
        Line::from(
            "When disabled, voxtype reads no keys. Bind your compositor (\
             Hyprland, Sway, Niri, KDE shortcuts) to call:",
        ),
        Line::from(Span::styled(
            "    voxtype record start    voxtype record stop",
            Style::default().fg(Color::Gray),
        )),
        Line::from(Span::styled(
            "    voxtype record toggle   voxtype record cancel",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
    ];

    let bindings = compositor_bindings::detect();
    if !bindings.is_empty() {
        lines.push(Line::from(Span::styled(
            "Compositor bindings detected",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for b in &bindings {
            let file = b.source.file_name().and_then(|s| s.to_str()).unwrap_or("");
            lines.push(Line::from(format!(
                "  • [{}] {}  →  voxtype {}",
                b.compositor, b.keys, b.action
            )));
            lines.push(Line::from(Span::styled(
                format!("      from {}", file),
                Style::default().fg(Color::Gray),
            )));
        }
        lines.push(Line::from(""));
    } else if !state.enabled {
        lines.push(Line::from(Span::styled(
            "No compositor bindings detected — voxtype will not receive any \
             PTT key events.",
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(""));
    }

    // Streaming dictation requires toggle activation; if the user has it
    // enabled, suppress PTT-pair suggestions in favor of a toggle binding.
    // Covers all streaming-capable backends (Parakeet, Soniox, future).
    let streaming = {
        let ed = ConfigEditor::load().ok();
        let engine = ed
            .as_ref()
            .and_then(|e| e.get_string("", "engine"))
            .unwrap_or_else(|| "whisper".to_string());
        match engine.as_str() {
            "parakeet" => ed
                .as_ref()
                .and_then(|e| e.get_bool("parakeet", "streaming"))
                .unwrap_or(false),
            "soniox" => ed
                .as_ref()
                .and_then(|e| e.get_bool("soniox", "streaming"))
                .unwrap_or(true),
            _ => false,
        }
    };
    let suggestions = compositor_bindings::suggest_missing(&bindings, streaming);
    if !suggestions.is_empty() {
        let comp = compositor_bindings::dominant_compositor(&bindings);
        lines.push(Line::from(Span::styled(
            format!("Suggested additions ({} format)", comp.name()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        for s in &suggestions {
            lines.push(Line::from(Span::styled(
                format!("  ▸ {}", s.label),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(format!("    {}", s.purpose)));
            for cfg in &s.config_lines {
                lines.push(Line::from(Span::styled(
                    format!("    {}", cfg),
                    Style::default().fg(Color::Gray),
                )));
            }
            lines.push(Line::from(""));
        }
    }

    if !state.enabled {
        lines.push(Line::from(Span::styled(
            "Compositor mode active: the rest of this section is ignored.",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn guidance_backend(state: &HotkeyState) -> Vec<Line<'_>> {
    let mut lines = vec![heading("Linux hotkey backend"), Line::from("")];
    match &state.backend {
        Backend::Known(HotkeyBackend::Evdev) => {
            lines.push(Line::from(
                "evdev reads keyboard events from /dev/input. It works independently of the desktop but requires membership in the input group.",
            ));
        }
        Backend::Known(HotkeyBackend::Portal) => {
            lines.push(Line::from(
                "The portal asks the desktop to own each Voxtype shortcut. The first start can open a shortcut configuration dialog.",
            ));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "The key fields are preferred bindings for the first request. The desktop's assigned bindings are authoritative afterwards.",
            ));
        }
        Backend::Known(HotkeyBackend::Auto) => {
            lines.push(Line::from(
                "Auto tries the portal first. It uses evdev when the portal or host-application registry cannot be reached, at startup or later.",
            ));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "Cancellation or refusal in the desktop's shortcut dialog does not fall back to raw keyboard access.",
            ));
        }
        Backend::Unrecognised(value) => {
            lines.push(Line::from(Span::styled(
                format!("config.toml sets backend = \"{value}\", which voxtype cannot parse. The daemon will not start until it is corrected."),
                Style::default().fg(Color::Red),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "Press ← or → to choose a valid backend, then s to save.",
            ));
        }
    }
    if !state.enabled {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(Ignored: the built-in listener is disabled.)",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn guidance_key<'a>(state: &'a HotkeyState) -> Vec<Line<'a>> {
    let mut lines = vec![
        heading("Push-to-talk key"),
        Line::from(""),
        Line::from(
            "Pick a key your fingers reach for without thinking. HOME, PAUSE, \
             SCROLLLOCK, F13 are popular because they don't conflict with \
             editor shortcuts.",
        ),
        Line::from(""),
        Line::from(
            "RIGHT* keys (RIGHTCTRL, RIGHTALT, RIGHTMETA) work well if you \
             touch-type with your left hand on the home row.",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Custom keys can be set in config.toml directly using KEY_* \
             names from <linux/input-event-codes.h>.",
            Style::default().fg(Color::Gray),
        )),
    ];
    if !state.enabled {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(Ignored: evdev listener is disabled.)",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn guidance_mode<'a>(state: &'a HotkeyState) -> Vec<Line<'a>> {
    let mut lines = vec![
        heading("Activation mode"),
        Line::from(""),
        Line::from(Span::styled(
            "Push-to-talk: ",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(
            "Hold the key while you speak; release to transcribe. Most \
             responsive — voice never starts running while you're thinking.",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Toggle: ",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(
            "Press once to start, press again to stop. Friendlier for long \
             dictation sessions but easy to leave running by accident.",
        ),
    ];
    if !state.enabled {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(Ignored: evdev listener is disabled.)",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn guidance_cancel<'a>(state: &'a HotkeyState) -> Vec<Line<'a>> {
    let mut lines = vec![
        heading("Cancel key"),
        Line::from(""),
        Line::from(
            "Aborts an in-progress recording or transcription and discards \
             audio without typing anything. Useful when you trip the PTT key \
             by accident or the wrong window is focused.",
        ),
        Line::from(""),
        Line::from(
            "ESC is the obvious pick. F12 / DELETE / END are good alternatives \
             if ESC is bound to something else in the foreground app.",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "(none) leaves cancellation off — kill the recording with \
             `voxtype record cancel` instead.",
            Style::default().fg(Color::Gray),
        )),
    ];
    if !state.enabled {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(Ignored: evdev listener is disabled.)",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn guidance_modifier<'a>(state: &'a HotkeyState) -> Vec<Line<'a>> {
    let mut lines = vec![
        heading("Secondary-model modifier"),
        Line::from(""),
        Line::from(
            "When this key is held alongside the PTT key, voxtype switches to \
             the [whisper] secondary_model for that recording.",
        ),
        Line::from(""),
        Line::from(
            "Common usage: large-v3 as your main model for accuracy, \
             small.en under the modifier for instant short notes.",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "(none) disables the modifier behavior; the PTT key always uses \
             the primary model.",
            Style::default().fg(Color::Gray),
        )),
    ];
    if !state.enabled {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(Ignored: evdev listener is disabled.)",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn display_key(key: &str) -> String {
    if KEY_CHOICES.contains(&key) {
        key.to_string()
    } else {
        format!("{}  (custom)", key)
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.hotkey.as_mut() {
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

fn handle_edit_key(state: &mut HotkeyState, key: KeyEvent) -> Action {
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

    fn state_with(backend: Backend) -> HotkeyState {
        HotkeyState {
            backend,
            backend_is_explicit: false,
            key: "HOME".to_string(),
            mode: Mode::PushToTalk,
            enabled: true,
            cancel_key: None,
            modifier: None,
            feedback: None,
            dirty_since_load: false,
            field: Field::Backend,
            editing: None,
        }
    }

    fn cycled(from: Backend, delta: i32) -> Backend {
        let mut state = state_with(from);
        state.cycle(delta);
        state.backend
    }

    fn known(backend: HotkeyBackend) -> Backend {
        Backend::Known(backend)
    }

    #[test]
    fn backend_cycles_forwards_through_every_choice() {
        let cycle = [
            cycled(known(HotkeyBackend::Evdev), 1),
            cycled(known(HotkeyBackend::Portal), 1),
            cycled(known(HotkeyBackend::Auto), 1),
        ];

        assert_eq!(
            cycle,
            [
                known(HotkeyBackend::Portal),
                known(HotkeyBackend::Auto),
                known(HotkeyBackend::Evdev)
            ]
        );
    }

    #[test]
    fn backend_cycles_backwards_through_every_choice() {
        let cycle = [
            cycled(known(HotkeyBackend::Portal), -1),
            cycled(known(HotkeyBackend::Auto), -1),
            cycled(known(HotkeyBackend::Evdev), -1),
        ];

        assert_eq!(
            cycle,
            [
                known(HotkeyBackend::Evdev),
                known(HotkeyBackend::Portal),
                known(HotkeyBackend::Auto)
            ]
        );
    }

    #[test]
    fn cycling_replaces_an_unrecognised_backend() {
        let unrecognised = Backend::Unrecognised("protal".to_string());

        let replaced = [cycled(unrecognised.clone(), 1), cycled(unrecognised, -1)];

        assert_eq!(
            replaced,
            [known(HotkeyBackend::Evdev), known(HotkeyBackend::Evdev)]
        );
    }

    #[test]
    fn cycling_the_backend_makes_save_write_it() {
        let mut state = state_with(known(HotkeyBackend::Evdev));
        state.cycle(1);

        assert!(state.backend_is_explicit);
    }

    #[test]
    fn a_known_backend_saves_its_config_name_and_an_unrecognised_one_saves_nothing() {
        let known_backend = known(HotkeyBackend::Portal);
        let unrecognised = Backend::Unrecognised("protal".to_string());

        let values = [known_backend.config_value(), unrecognised.config_value()];

        assert_eq!(values, [Some("portal"), None]);
    }
}
