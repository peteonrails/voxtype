//! Desktop notifications section.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{layout::Rect, text::Line, Frame};

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
                self.feedback = Some((
                    FeedbackLevel::Err,
                    format!("Couldn't read the config: {}", e),
                ));
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
            Err(e) => self.feedback = Some((FeedbackLevel::Err, format!("Not saved: {}", e))),
        }
        Action::None
    }
    pub fn reset(&mut self) {
        if let Ok(fresh) = Self::load() {
            let field = self.field;
            *self = fresh;
            self.field = field;
            self.feedback = Some((FeedbackLevel::Ok, "Put back the saved settings".to_string()));
        }
    }
    fn move_field(&mut self, delta: i32) {
        let len = Field::ALL.len() as i32;
        let cur = Field::ALL
            .iter()
            .position(|f| *f == self.field)
            .unwrap_or(0) as i32;
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

/// The page's rows, from its in-memory settings.
pub fn rows(state: &NotificationsState) -> Vec<FormRowSpec> {
    vec![
        FormRowSpec::new(
            state.field == Field::OnStart,
            "When recording starts",
            yesno(state.on_recording_start),
        ),
        FormRowSpec::new(
            state.field == Field::OnStop,
            "When recording stops",
            yesno(state.on_recording_stop),
        ),
        FormRowSpec::new(
            state.field == Field::OnTranscription,
            "With the text",
            yesno(state.on_transcription),
        ),
        FormRowSpec::new(
            state.field == Field::ShowEngineIcon,
            "Engine icon in title",
            yesno(state.show_engine_icon),
        ),
    ]
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.notifications else {
        common::render_load_error(f, area, app);
        return;
    };
    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|(lvl, msg)| (*lvl, msg.as_str()));
    common::render_form_with_guidance(
        f,
        area,
        app,
        feedback_pair,
        &rows(state),
        guidance_for_field(state),
    );
}

fn yesno(b: bool) -> String {
    common::on_off(b)
}

fn guidance_for_field(state: &NotificationsState) -> Vec<Line<'_>> {
    let text = |s: &'static str| Line::from(s);
    match state.field {
        Field::OnStart => vec![text(
            "A notification as soon as voxtype starts listening. Most people \
             rely on the sound, the overlay or Waybar instead.",
        )],
        Field::OnStop => vec![text(
            "A notification when recording stops and transcription begins, so \
             you know the stop was heard.",
        )],
        Field::OnTranscription => vec![text(
            "Shows what was transcribed, which helps when the text went to the \
             wrong window.",
        )],
        Field::ShowEngineIcon => vec![text(
            "Puts the engine's icon in the notification title, handy if you \
             switch engines.",
        )],
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
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Char('h')
        | KeyCode::Char('l')
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
