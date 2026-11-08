//! Meeting mode settings: enabled, audio source, diarization on/off.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FeedbackLevel, FormRowSpec};
use super::config_editor::{ConfigEditor, EditorError};
use super::theme;

#[derive(Debug, Clone)]
pub struct MeetingState {
    pub enabled: bool,
    pub diarization_enabled: bool,
    pub audio_source: String,
    pub field: Field,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Enabled,
    Diarization,
    AudioSource,
}
impl Field {
    const ALL: &'static [Field] = &[Field::Enabled, Field::Diarization, Field::AudioSource];
}
const SOURCES: &[&str] = &["mic", "system", "both"];

impl MeetingState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            enabled: ed.get_bool("meeting", "enabled").unwrap_or(false),
            diarization_enabled: ed
                .get_bool("meeting.diarization", "enabled")
                .unwrap_or(false),
            audio_source: ed
                .get_string("meeting.audio", "source")
                .unwrap_or_else(|| "mic".to_string()),
            field: Field::Enabled,
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
        ed.set_bool("meeting", "enabled", self.enabled);
        ed.set_bool("meeting.diarization", "enabled", self.diarization_enabled);
        ed.set_string("meeting.audio", "source", &self.audio_source);
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
    fn cycle(&mut self, delta: i32) {
        match self.field {
            Field::Enabled => self.enabled = !self.enabled,
            Field::Diarization => self.diarization_enabled = !self.diarization_enabled,
            Field::AudioSource => {
                let idx = SOURCES
                    .iter()
                    .position(|s| *s == self.audio_source)
                    .map(|i| i as i32)
                    .unwrap_or(0);
                self.audio_source =
                    SOURCES[(idx + delta).rem_euclid(SOURCES.len() as i32) as usize].to_string();
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

/// `mic` reads as `Microphone`, `system` as `Computer audio`.
fn source_name(source: &str) -> String {
    match source {
        "mic" => "Microphone".to_string(),
        "system" => "Computer audio".to_string(),
        "both" => "Both".to_string(),
        other => other.to_string(),
    }
}

/// The page's rows, from its in-memory settings.
pub fn rows(state: &MeetingState) -> Vec<FormRowSpec> {
    let off = !state.enabled;
    let sources: Vec<String> = SOURCES.iter().map(|s| source_name(s)).collect();
    vec![
        FormRowSpec::new(
            state.field == Field::Enabled,
            "Meeting mode",
            yesno(state.enabled),
        ),
        FormRowSpec::new(
            state.field == Field::Diarization,
            "Tell speakers apart",
            yesno(state.diarization_enabled),
        )
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::AudioSource,
            "Listen to",
            source_name(&state.audio_source),
        )
        .choices(&sources)
        .dimmed(off),
    ]
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.meeting else {
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

fn guidance_for_field(state: &MeetingState) -> Vec<Line<'_>> {
    let text = |s: &'static str| Line::from(s);
    let dim = |s: &'static str| Line::from(Span::styled(s, theme::muted()));
    match state.field {
        Field::Enabled => vec![
            text(
                "Transcribes long meetings in chunks into one timestamped \
                 transcript, saved as it goes in ~/.local/share/voxtype/meetings/.",
            ),
            dim("Start one with: voxtype meeting start"),
        ],
        Field::Diarization => vec![text(
            "Labels who is talking (Speaker 1, Speaker 2) so the transcript \
             reads like a dialogue. Uses more CPU.",
        )],
        Field::AudioSource => vec![text(
            "Microphone for your own voice, Computer audio for the call, or Both. \
             With Both, voxtype removes the echo of your voice.",
        )],
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.meeting.as_mut() {
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
