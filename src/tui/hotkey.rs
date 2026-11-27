//! Hotkey settings: PTT key, mode, cancel/modifier keys, evdev enable.

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

/// In-memory copy of the hotkey state, owned by `App`. Edits mutate this; `s`
/// commits via [`ConfigEditor`] and rolls back on validation error.
#[derive(Debug, Clone)]
pub struct HotkeyState {
    pub key: String,
    pub mode: Mode,
    pub enabled: bool,
    pub cancel_key: Option<String>,
    pub paste_last_key: Option<String>,
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
    PasteLastKey,
    Modifier,
}

impl Field {
    const ALL: &'static [Field] = &[
        Field::Enabled,
        Field::Key,
        Field::Mode,
        Field::CancelKey,
        Field::PasteLastKey,
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

const PASTE_LAST_CHOICES: &[Option<&str>] = &[
    None,
    Some("CTRL+ALT+V"),
    Some("F9"),
    Some("F10"),
    Some("SUPER+ALT+V"),
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
            paste_last_key: ed.get_string("hotkey", "paste_last_key"),
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
        match &self.paste_last_key {
            Some(k) => ed.set_string("hotkey", "paste_last_key", k),
            None => ed.unset("hotkey", "paste_last_key"),
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
        matches!(
            field,
            Field::Key | Field::CancelKey | Field::PasteLastKey | Field::Modifier
        )
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
            Field::PasteLastKey => self.paste_last_key.clone().unwrap_or_default(),
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
            Field::PasteLastKey => {
                self.paste_last_key = if trimmed.is_empty() {
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
            Field::PasteLastKey => {
                self.paste_last_key =
                    cycle_opt(PASTE_LAST_CHOICES, self.paste_last_key.as_deref(), delta);
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
            "Built-in evdev listener",
            if state.enabled { "enabled" } else { "disabled" },
        ),
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
            state.field == Field::PasteLastKey,
            "Paste last dictation",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::PasteLastKey => e.input.caret_string(),
                _ => state
                    .paste_last_key
                    .as_deref()
                    .unwrap_or("(none)")
                    .to_string(),
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
        Field::Key => guidance_key(state),
        Field::Mode => guidance_mode(state),
        Field::CancelKey => guidance_cancel(state),
        Field::PasteLastKey => guidance_paste_last(state),
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
        heading("Built-in evdev listener"),
        Line::from(""),
        Line::from(
            "When enabled, voxtype reads keyboard events directly from \
             /dev/input/event* (this needs the `input` group, which lets \
             every program read your keystrokes; prefer a compositor \
             binding). It owns the chosen PTT key globally, so no compositor \
             binding is needed.",
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

    let streaming = streaming_requires_toggle(ConfigEditor::load().ok().as_ref());
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

fn guidance_paste_last<'a>(_state: &'a HotkeyState) -> Vec<Line<'a>> {
    vec![
        heading("Paste last dictation"),
        Line::from(""),
        Line::from(
            "Sends your last dictation again, into whatever window has focus. \
             For when the text went to the wrong window, or you need it twice.",
        ),
        Line::from(""),
        Line::from(
            "A key, or modifiers and a key joined with +. CTRL+ALT+V is the \
             combination other dictation tools use. CTRL, ALT, SHIFT and SUPER \
             match either side of the keyboard.",
        ),
        Line::from(""),
        Line::from(
            "The text comes from history, so history has to be on. The focused \
             app also sees the keys you press, so pick a combination it ignores.",
        ),
    ]
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

/// Whether the configured engine streams output that types while recording,
/// so only a toggle binding works: suggest that instead of a press/release
/// pair. The answer is the daemon's [`Config::streaming_active`], the check
/// that promotes push-to-talk to toggle at startup, asked of the config the
/// daemon would build from the file ([`ConfigEditor::daemon_config`]), so
/// buffered streaming output and engines that record in batch keep the
/// press/release pair.
///
/// [`Config::streaming_active`]: crate::config::Config::streaming_active
fn streaming_requires_toggle(ed: Option<&ConfigEditor>) -> bool {
    ed.and_then(ConfigEditor::daemon_config)
        .is_some_and(|config| config.streaming_active())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requires_toggle(source: &str) -> bool {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, source).unwrap();
        let ed = ConfigEditor::load_from_path(path).unwrap();
        streaming_requires_toggle(Some(&ed))
    }

    /// Streaming that types while the key is held needs a toggle binding.
    /// Buffered streaming output types nothing until the stop, so the
    /// press/release pair works. The answer is the daemon's
    /// (`Config::streaming_active`): local Whisper streams, remote Whisper
    /// and the Soniox async API record in batch.
    #[test]
    fn only_typing_streaming_needs_a_toggle_binding() {
        let soniox = "engine = \"soniox\"\n[soniox]\nstreaming = true\n";
        let cases = [
            (String::new(), false),
            (soniox.to_string(), true),
            (
                format!("{soniox}\n[output]\nstreaming_buffer_output = true\n"),
                false,
            ),
            ("[whisper]\nstreaming = true\n".to_string(), true),
            (
                "[whisper]\nstreaming = true\nmode = \"remote\"\n".to_string(),
                false,
            ),
            (
                "engine = \"soniox\"\n[soniox]\nasync_api = true\n".to_string(),
                false,
            ),
            // The daemon fills a missing [soniox] table with its defaults
            // (streaming on), as every Soniox daemon that starts has one.
            ("engine = \"soniox\"\n".to_string(), true),
        ];
        let got: Vec<bool> = cases.iter().map(|(src, _)| requires_toggle(src)).collect();
        let want: Vec<bool> = cases.iter().map(|(_, want)| *want).collect();
        assert_eq!(got, want);
    }
}
