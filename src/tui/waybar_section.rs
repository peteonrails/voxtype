//! Waybar status integration: icon theme + per-state icon overrides.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FeedbackLevel, FormRowSpec, TextInput, TextInputResult};
use super::config_editor::{ConfigEditor, EditorError};

#[derive(Debug, Clone)]
pub struct WaybarState {
    pub icon_theme: String,
    pub icon_idle: Option<String>,
    pub icon_recording: Option<String>,
    pub icon_transcribing: Option<String>,
    pub icon_stopped: Option<String>,
    pub field: Field,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
    pub editing: Option<TextEdit>,
}

#[derive(Debug, Clone)]
pub struct TextEdit {
    pub field: Field,
    pub input: TextInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Theme,
    IconIdle,
    IconRecording,
    IconTranscribing,
    IconStopped,
}

impl Field {
    const ALL: &'static [Field] = &[
        Field::Theme,
        Field::IconIdle,
        Field::IconRecording,
        Field::IconTranscribing,
        Field::IconStopped,
    ];
}

const THEMES: &[&str] = &[
    "emoji",
    "nerd-font",
    "material",
    "phosphor",
    "codicons",
    "omarchy",
    "minimal",
    "dots",
    "arrows",
    "text",
];

impl WaybarState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            icon_theme: ed
                .get_string("status", "icon_theme")
                .unwrap_or_else(|| "emoji".to_string()),
            icon_idle: ed.get_string("status.icons", "idle"),
            icon_recording: ed.get_string("status.icons", "recording"),
            icon_transcribing: ed.get_string("status.icons", "transcribing"),
            icon_stopped: ed.get_string("status.icons", "stopped"),
            field: Field::Theme,
            feedback: None,
            dirty_since_load: false,
            editing: None,
        })
    }

    pub fn save(&mut self) -> Action {
        let mut ed = match ConfigEditor::load() {
            Ok(e) => e,
            Err(e) => {
                self.feedback = Some((FeedbackLevel::Err, format!("load: {}", e)));
                return Action::None;
            }
        };
        ed.set_string("status", "icon_theme", &self.icon_theme);
        for (key, val) in [
            ("idle", &self.icon_idle),
            ("recording", &self.icon_recording),
            ("transcribing", &self.icon_transcribing),
            ("stopped", &self.icon_stopped),
        ] {
            match val {
                Some(v) if !v.is_empty() => ed.set_string("status.icons", key, v),
                _ => ed.unset("status.icons", key),
            }
        }
        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.feedback = Some((
                    FeedbackLevel::Ok,
                    format!("Saved to {}", ed.path().display()),
                ));
            }
            Err(e) => self.feedback = Some((FeedbackLevel::Err, format!("save: {}", e))),
        }
        Action::None
    }

    pub fn reset(&mut self) {
        if let Ok(fresh) = Self::load() {
            let field = self.field;
            *self = fresh;
            self.field = field;
            self.feedback = Some((FeedbackLevel::Ok, "Reverted".to_string()));
        }
    }

    fn move_field(&mut self, delta: i32) {
        let len = Field::ALL.len() as i32;
        let cur = Field::ALL.iter().position(|f| *f == self.field).unwrap_or(0) as i32;
        self.field = Field::ALL[((cur + delta).rem_euclid(len)) as usize];
    }

    fn cycle(&mut self, delta: i32) {
        match self.field {
            Field::Theme => {
                let idx = THEMES
                    .iter()
                    .position(|t| *t == self.icon_theme)
                    .map(|i| i as i32)
                    .unwrap_or(0);
                self.icon_theme = THEMES
                    [((idx + delta).rem_euclid(THEMES.len() as i32)) as usize]
                    .to_string();
            }
            // Icon overrides are free-text; ←→ kicks off inline edit.
            Field::IconIdle | Field::IconRecording | Field::IconTranscribing | Field::IconStopped => {
                self.start_edit_if_text_field();
                return;
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }

    fn is_text_field(field: Field) -> bool {
        matches!(
            field,
            Field::IconIdle
                | Field::IconRecording
                | Field::IconTranscribing
                | Field::IconStopped
        )
    }

    fn start_edit_if_text_field(&mut self) -> bool {
        if !Self::is_text_field(self.field) {
            return false;
        }
        let initial = match self.field {
            Field::IconIdle => self.icon_idle.clone().unwrap_or_default(),
            Field::IconRecording => self.icon_recording.clone().unwrap_or_default(),
            Field::IconTranscribing => self.icon_transcribing.clone().unwrap_or_default(),
            Field::IconStopped => self.icon_stopped.clone().unwrap_or_default(),
            _ => String::new(),
        };
        self.editing = Some(TextEdit {
            field: self.field,
            input: TextInput::new(initial),
        });
        true
    }

    fn commit_text_edit(&mut self, field: Field, buffer: String) {
        let opt = if buffer.is_empty() {
            None
        } else {
            Some(buffer)
        };
        match field {
            Field::IconIdle => self.icon_idle = opt,
            Field::IconRecording => self.icon_recording = opt,
            Field::IconTranscribing => self.icon_transcribing = opt,
            Field::IconStopped => self.icon_stopped = opt,
            _ => {}
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.waybar {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Waybar");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(
                Paragraph::new("Failed to load config.").wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }
    };

    let icon_value = |field: Field, value: &Option<String>| -> String {
        match state.editing.as_ref() {
            Some(e) if e.field == field => e.input.caret_string(),
            _ => value.clone().unwrap_or_else(|| "(theme default)".to_string()),
        }
    };

    let rows = vec![
        FormRowSpec::new(state.field == Field::Theme, "Icon theme", &state.icon_theme),
        FormRowSpec::new(
            state.field == Field::IconIdle,
            "Override · idle",
            icon_value(Field::IconIdle, &state.icon_idle),
        ),
        FormRowSpec::new(
            state.field == Field::IconRecording,
            "Override · recording",
            icon_value(Field::IconRecording, &state.icon_recording),
        ),
        FormRowSpec::new(
            state.field == Field::IconTranscribing,
            "Override · transcribing",
            icon_value(Field::IconTranscribing, &state.icon_transcribing),
        ),
        FormRowSpec::new(
            state.field == Field::IconStopped,
            "Override · stopped",
            icon_value(Field::IconStopped, &state.icon_stopped),
        ),
    ];

    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|(lvl, msg)| (*lvl, msg.as_str()));

    common::render_form_with_guidance(
        f,
        area,
        "Waybar / Status",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance(state),
    );
}

fn heading(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        text.into(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn guidance(state: &WaybarState) -> Vec<Line<'static>> {
    match state.field {
        Field::Theme => vec![
            heading("Icon theme"),
            Line::from(""),
            Line::from(
                "The glyph set `voxtype status --follow` emits to your status \
                 bar. Match it to whatever your bar's font supports.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Common picks:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("  • emoji — works everywhere, no special font needed."),
            Line::from("  • nerd-font — for users on a Nerd Font."),
            Line::from("  • phosphor — Phosphor icon font."),
            Line::from("  • omarchy — matches Omarchy's stock ricing."),
            Line::from("  • text — plain ASCII, no glyphs at all."),
            Line::from(""),
            Line::from(Span::styled(
                "Run `voxtype setup waybar` for ready-to-paste Waybar config.",
                Style::default().fg(Color::Gray),
            )),
        ],
        Field::IconIdle => icon_guidance(
            state,
            "idle",
            "Shown when voxtype is loaded but not actively recording or \
             transcribing.",
        ),
        Field::IconRecording => icon_guidance(
            state,
            "recording",
            "Shown while voxtype is capturing audio.",
        ),
        Field::IconTranscribing => icon_guidance(
            state,
            "transcribing",
            "Shown while voxtype is running inference on a captured clip.",
        ),
        Field::IconStopped => icon_guidance(
            state,
            "stopped",
            "Shown when the daemon isn't running. Useful for spotting that \
             voxtype crashed or wasn't started.",
        ),
    }
}

fn icon_guidance(state: &WaybarState, label: &str, purpose: &str) -> Vec<Line<'static>> {
    let mut lines = vec![
        heading(format!("Override · {}", label)),
        Line::from(""),
        Line::from(purpose.to_string()),
        Line::from(""),
        Line::from(format!(
            "Set a glyph here to override the {} theme's choice for this \
             state. Leave empty to fall back to the theme default.",
            state.icon_theme
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press Enter or i to edit. Type any unicode glyph (emoji, Nerd \
             Font glyph, ASCII). Esc cancels.",
            Style::default().fg(Color::Gray),
        )),
    ];
    if state
        .editing
        .as_ref()
        .map(|e| e.field == state.field)
        .unwrap_or(false)
    {
        lines.insert(
            0,
            Line::from(Span::styled(
                "✎ Editing — Enter to commit, Esc to cancel",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
        );
        lines.insert(1, Line::from(""));
    }
    lines
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.waybar.as_mut() {
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

fn handle_edit_key(state: &mut WaybarState, key: KeyEvent) -> Action {
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
