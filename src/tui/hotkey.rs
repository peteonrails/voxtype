//! Hotkey settings: PTT key, mode, cancel/modifier keys, evdev enable.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::config_editor::{ConfigEditor, EditorError};

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
    Key,
    Mode,
    CancelKey,
    Modifier,
    Enabled,
}

impl Field {
    const ALL: &'static [Field] = &[
        Field::Key,
        Field::Mode,
        Field::CancelKey,
        Field::Modifier,
        Field::Enabled,
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
            field: Field::Key,
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
        let cur = Field::ALL.iter().position(|f| *f == self.field).unwrap_or(0) as i32;
        let new = (cur + delta).rem_euclid(len);
        self.field = Field::ALL[new as usize];
    }

    /// Cycle the value of the focused field by `delta` (-1 for ← / +1 for →).
    fn cycle(&mut self, delta: i32) {
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

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("Hotkey");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let state = match &app.hotkey {
        Some(s) => s,
        None => {
            f.render_widget(
                Paragraph::new("Failed to load config; check ~/.config/voxtype/config.toml.")
                    .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if state.feedback.is_some() { 2 } else { 0 }),
            Constraint::Length(2), // header
            Constraint::Length(7), // form
            Constraint::Min(0),    // help text
            Constraint::Length(1), // bottom hint
        ])
        .split(inner);

    if let Some(fb) = &state.feedback {
        render_feedback(f, chunks[0], fb);
    }
    render_header(f, chunks[1], state);
    render_form(f, chunks[2], state);
    render_help_text(f, chunks[3]);
    render_bottom_hint(f, chunks[4], state);
}

fn render_feedback(f: &mut Frame, area: Rect, fb: &Feedback) {
    let style = match fb.level {
        FeedbackLevel::Ok => Style::default().fg(Color::Green),
        FeedbackLevel::Err => Style::default().fg(Color::Red),
    };
    let prefix = match fb.level {
        FeedbackLevel::Ok => "✓ ",
        FeedbackLevel::Err => "✗ ",
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{}{}", prefix, fb.message),
            style,
        ))),
        area,
    );
}

fn render_header(f: &mut Frame, area: Rect, state: &HotkeyState) {
    let dirty = if state.dirty_since_load {
        Span::styled(
            "  • unsaved",
            Style::default().fg(Color::Yellow),
        )
    } else {
        Span::raw("")
    };
    let line = Line::from(vec![
        Span::styled(
            "Hotkey",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        dirty,
    ]);
    f.render_widget(Paragraph::new(vec![line, Line::from("")]), area);
}

fn render_form(f: &mut Frame, area: Rect, state: &HotkeyState) {
    let rows = [
        (Field::Key, "Push-to-talk key", display_key(&state.key)),
        (
            Field::Mode,
            "Mode",
            match state.mode {
                Mode::PushToTalk => "Push-to-talk (hold)".to_string(),
                Mode::Toggle => "Toggle (press to start/stop)".to_string(),
            },
        ),
        (
            Field::CancelKey,
            "Cancel key",
            state
                .cancel_key
                .as_deref()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "(none)".to_string()),
        ),
        (
            Field::Modifier,
            "Modifier (secondary model)",
            state
                .modifier
                .as_deref()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "(none)".to_string()),
        ),
        (
            Field::Enabled,
            "Built-in evdev listener",
            (if state.enabled { "enabled" } else { "disabled" }).to_string(),
        ),
    ];

    let lines: Vec<Line> = rows
        .iter()
        .map(|(field, label, value)| {
            let focused = *field == state.field;
            let label_style = if focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let value_style = if focused {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            let prefix = if focused { "▸ " } else { "  " };
            Line::from(vec![
                Span::styled(format!("{}{:<28}", prefix, label), label_style),
                Span::styled(format!(" ◂ {} ▸", value), value_style),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(lines), area);
}

fn render_help_text(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Tips",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(
            "  • Disable the evdev listener if you bind voxtype record start/stop/toggle \
             from your compositor (Hyprland, Sway, River) — those bindings call voxtype \
             directly without needing /dev/input access.",
        ),
        Line::from(""),
        Line::from(
            "  • The cancel key aborts an in-progress recording and discards audio without \
             transcribing.",
        ),
        Line::from(
            "  • The modifier key, when held while pressing the PTT key, swaps to the \
             secondary model defined in [whisper] secondary_model.",
        ),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn render_bottom_hint(f: &mut Frame, area: Rect, state: &HotkeyState) {
    let dirty_marker = if state.dirty_since_load {
        Span::styled("  ●", Style::default().fg(Color::Yellow))
    } else {
        Span::raw("")
    };
    let line = Line::from(vec![
        Span::styled(
            " ↑↓ field   ←→ change   s save   r revert ",
            Style::default().fg(Color::DarkGray),
        ),
        dirty_marker,
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn display_key(key: &str) -> String {
    if KEY_CHOICES.iter().any(|c| *c == key) {
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
        KeyCode::Char('s') => state.save(),
        KeyCode::Char('r') => {
            state.reset();
            Action::None
        }
        _ => Action::None,
    }
}
