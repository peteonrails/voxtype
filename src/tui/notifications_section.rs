//! Desktop notifications section.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FeedbackLevel, FormRowSpec};
use super::config_editor::{ConfigEditor, EditorError};

#[derive(Debug, Clone)]
pub struct NotificationsState {
    pub on_recording_start: bool,
    pub on_recording_stop: bool,
    pub on_transcription: bool,
    pub show_engine_icon: bool,
    pub field: Field,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    OnStart,
    OnStop,
    OnTranscription,
    ShowEngineIcon,
}
impl Field {
    const ALL: &'static [Field] = &[
        Field::OnStart,
        Field::OnStop,
        Field::OnTranscription,
        Field::ShowEngineIcon,
    ];
}

const TABLE: &str = "output.notification";

impl NotificationsState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            on_recording_start: ed.get_bool(TABLE, "on_recording_start").unwrap_or(false),
            on_recording_stop: ed.get_bool(TABLE, "on_recording_stop").unwrap_or(false),
            on_transcription: ed.get_bool(TABLE, "on_transcription").unwrap_or(true),
            show_engine_icon: ed.get_bool(TABLE, "show_engine_icon").unwrap_or(false),
            field: Field::OnStart,
            feedback: None,
            dirty_since_load: false,
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
        ed.set_bool(TABLE, "on_recording_start", self.on_recording_start);
        ed.set_bool(TABLE, "on_recording_stop", self.on_recording_stop);
        ed.set_bool(TABLE, "on_transcription", self.on_transcription);
        ed.set_bool(TABLE, "show_engine_icon", self.show_engine_icon);
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
    fn cycle(&mut self) {
        match self.field {
            Field::OnStart => self.on_recording_start = !self.on_recording_start,
            Field::OnStop => self.on_recording_stop = !self.on_recording_stop,
            Field::OnTranscription => self.on_transcription = !self.on_transcription,
            Field::ShowEngineIcon => self.show_engine_icon = !self.show_engine_icon,
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.notifications {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Notifications");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(Paragraph::new("Failed to load config.").wrap(Wrap { trim: true }), inner);
            return;
        }
    };

    let rows = vec![
        FormRowSpec::new(
            state.field == Field::OnStart,
            "On recording start",
            yesno(state.on_recording_start),
        ),
        FormRowSpec::new(
            state.field == Field::OnStop,
            "On recording stop",
            yesno(state.on_recording_stop),
        ),
        FormRowSpec::new(
            state.field == Field::OnTranscription,
            "Show transcribed text",
            yesno(state.on_transcription),
        ),
        FormRowSpec::new(
            state.field == Field::ShowEngineIcon,
            "Engine icon in title",
            yesno(state.show_engine_icon),
        ),
    ];

    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|(lvl, msg)| (*lvl, msg.as_str()));

    common::render_form_with_guidance(
        f,
        area,
        "Desktop Notifications",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance_for_field(state),
    );
}

fn yesno(b: bool) -> String {
    (if b { "yes" } else { "no" }).to_string()
}

fn heading<'a>(text: &'a str) -> Line<'a> {
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn guidance_for_field(state: &NotificationsState) -> Vec<Line<'_>> {
    match state.field {
        Field::OnStart => vec![
            heading("On recording start"),
            Line::from(""),
            Line::from(
                "Fires a desktop notification the moment voxtype begins \
                 capturing audio.",
            ),
            Line::from(""),
            Line::from(
                "Useful when you have audio feedback off and want a visual \
                 cue. Most users leave this off — the recording indicator in \
                 Waybar covers it.",
            ),
        ],
        Field::OnStop => vec![
            heading("On recording stop"),
            Line::from(""),
            Line::from(
                "Notifies when voxtype stops recording and starts \
                 transcribing. Helpful when transcription takes a few \
                 seconds — you know voxtype heard the stop.",
            ),
        ],
        Field::OnTranscription => vec![
            heading("Show transcribed text"),
            Line::from(""),
            Line::from(
                "After transcription completes, posts the transcript text \
                 in a desktop notification.",
            ),
            Line::from(""),
            Line::from(
                "Most useful when output goes to the wrong window (e.g. you \
                 changed focus mid-dictation). The notification is the \
                 receipt.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Notifications go through libnotify, so they respect mako/\
                 dunst/KDE/GNOME settings.",
                Style::default().fg(Color::Gray),
            )),
        ],
        Field::ShowEngineIcon => vec![
            heading("Engine icon in title"),
            Line::from(""),
            Line::from(
                "Prefixes the notification title with an engine icon \
                 (🦜 for Parakeet, 🗣️ for Whisper) so you can see at a \
                 glance which engine produced the transcript.",
            ),
            Line::from(""),
            Line::from(
                "Helpful when you switch engines often or run multiple \
                 voxtype configurations side by side.",
            ),
        ],
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.notifications.as_mut() {
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
        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l')
        | KeyCode::Char(' ') => {
            state.cycle();
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
