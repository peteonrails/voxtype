//! Hotkey settings: PTT key, mode, cancel/modifier keys, evdev enable.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use super::app::{Action, App};
use super::common::{
    self, FeedbackLevel as CommonFeedback, FormRowSpec, TextInput, TextInputResult,
};
use super::compositor_bindings;
use super::config_editor::{ConfigEditor, EditorError};
use super::theme;

/// In-memory copy of the hotkey state, owned by `App`. Edits mutate this; `s`
/// commits via [`ConfigEditor`] and rolls back on validation error.
#[derive(Debug, Clone)]
pub struct HotkeyState {
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
    Key,
    Mode,
    CancelKey,
    Modifier,
}

impl Field {
    const ALL: &'static [Field] = &[
        Field::Enabled,
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
        Ok(Self {
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
                    message: format!("Couldn't read the config: {}", e),
                });
                return Action::None;
            }
        };
        ed.set_string("hotkey", "key", &self.key);
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
                *self = fresh;
                self.field = field;
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

/// The page's rows, from its in-memory settings.
pub fn rows(state: &HotkeyState) -> Vec<FormRowSpec> {
    // Greyout fields after Enabled when the evdev listener is off — those
    // controls don't affect anything until the listener turns back on.
    let greyout = !state.enabled;

    let rows = vec![
        FormRowSpec::new(
            state.field == Field::Enabled,
            "Built-in listener",
            if state.enabled { "On" } else { "Off" },
        ),
        FormRowSpec::new(
            state.field == Field::Key,
            "Hotkey",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::Key => e.input.caret_string(),
                _ => display_key(&state.key),
            },
        )
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::Mode,
            "Hotkey mode",
            match state.mode {
                Mode::PushToTalk => "Hold to talk",
                Mode::Toggle => "Toggle",
            },
        )
        .choices(&["Hold to talk", "Toggle"])
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::CancelKey,
            "Cancel key",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::CancelKey => e.input.caret_string(),
                _ => state
                    .cancel_key
                    .as_deref()
                    .unwrap_or(common::NOT_SET)
                    .to_string(),
            },
        )
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::Modifier,
            "Second model key",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::Modifier => e.input.caret_string(),
                _ => state
                    .modifier
                    .as_deref()
                    .unwrap_or(common::NOT_SET)
                    .to_string(),
            },
        )
        .dimmed(greyout),
    ];
    rows
}

/// Put the cursor on the row labelled `label`, for Basics.
pub fn focus_label(state: &mut HotkeyState, label: &str) {
    if let Some(i) = rows(state).iter().position(|r| r.label == label) {
        state.field = Field::ALL[i];
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.hotkey else {
        common::render_load_error(f, area, app);
        return;
    };
    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|fb| (to_common_level(fb.level), fb.message.as_str()));

    let guidance = guidance_for_field(state);

    common::render_form_with_guidance(f, area, app, feedback_pair, &rows(state), guidance);
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
        Field::Key => guidance_key(state),
        Field::Mode => guidance_mode(state),
        Field::CancelKey => guidance_cancel(state),
        Field::Modifier => guidance_modifier(state),
    }
}

/// A dim note that the setting does nothing while the listener is off.
fn ignored_note(state: &HotkeyState) -> Option<Line<'static>> {
    (!state.enabled).then(|| {
        Line::from(Span::styled(
            "The built-in listener is off, so this does nothing.",
            theme::muted(),
        ))
    })
}

fn guidance_enabled<'a>(state: &'a HotkeyState) -> Vec<Line<'a>> {
    let mut lines = vec![
        Line::from(
            "On: voxtype watches the keyboard for the hotkey itself (you must be \
             in the input group). Off: your desktop's shortcuts run voxtype \
             record start, stop or toggle.",
        ),
        Line::from(""),
    ];

    let bindings = compositor_bindings::detect();
    if !bindings.is_empty() {
        lines.push(Line::from(Span::styled(
            "Shortcuts found in your desktop's config",
            theme::heading(),
        )));
        for b in &bindings {
            let file = b.source.file_name().and_then(|s| s.to_str()).unwrap_or("");
            lines.push(Line::from(format!(
                "  • [{}] {}  →  voxtype {}",
                b.compositor, b.keys, b.action
            )));
            lines.push(Line::from(Span::styled(
                format!("      from {}", file),
                theme::muted(),
            )));
        }
        lines.push(Line::from(""));
    } else if !state.enabled {
        lines.push(Line::from(Span::styled(
            "No desktop shortcut for voxtype was found, so nothing will start a \
             recording. Add one of these:",
            theme::error(),
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
            format!("Shortcuts to add ({} format)", comp.name()),
            theme::heading(),
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
                    theme::muted(),
                )));
            }
            lines.push(Line::from(""));
        }
    }

    lines
}

fn guidance_key(state: &HotkeyState) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(
            "Pick a key you can reach without looking that nothing else uses, \
             such as SCROLLLOCK, PAUSE, F13 or a right-hand modifier.",
        ),
        Line::from(Span::styled(
            "←→ goes through common keys; Enter types any key name.",
            theme::muted(),
        )),
    ];
    lines.extend(ignored_note(state));
    lines
}

fn guidance_mode(state: &HotkeyState) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(
        "Hold to talk records while the key is down. Toggle starts on one press \
         and stops on the next, which suits long dictation.",
    )];
    lines.extend(ignored_note(state));
    lines
}

fn guidance_cancel(state: &HotkeyState) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(
            "Throws away the recording in progress without typing anything. \
             ESC is the usual choice.",
        ),
        Line::from(Span::styled(
            "Not set: cancel with voxtype record cancel.",
            theme::muted(),
        )),
    ];
    lines.extend(ignored_note(state));
    lines
}

fn guidance_modifier(state: &HotkeyState) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(
            "Hold this together with the hotkey to use Whisper's secondary model \
             for that recording, for example a small fast one for quick notes.",
        ),
        Line::from(Span::styled(
            "Not set: always the main model.",
            theme::muted(),
        )),
    ];
    lines.extend(ignored_note(state));
    lines
}

fn display_key(key: &str) -> String {
    key.to_string()
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
