//! Voice Activity Detection settings.

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
pub struct VadState {
    pub enabled: bool,
    pub backend: String,
    pub threshold: f32,
    pub field: Field,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Enabled,
    Backend,
    Threshold,
}
impl Field {
    const ALL: &'static [Field] = &[Field::Enabled, Field::Backend, Field::Threshold];
}
const BACKEND_CHOICES: &[&str] = &["auto", "energy", "whisper"];

impl VadState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            enabled: ed.get_bool("vad", "enabled").unwrap_or(false),
            backend: ed
                .get_string("vad", "backend")
                .unwrap_or_else(|| "auto".to_string()),
            threshold: ed.get_f32_or("vad", "threshold", 0.5),
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
        ed.set_bool("vad", "enabled", self.enabled);
        ed.set_string("vad", "backend", &self.backend);
        ed.set_f32("vad", "threshold", self.threshold);
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
        match Self::load() {
            Ok(fresh) => {
                let field = self.field;
                *self = fresh;
                self.field = field;
                self.feedback =
                    Some((FeedbackLevel::Ok, "Put back the saved settings".to_string()));
            }
            Err(e) => {
                self.feedback = Some((
                    FeedbackLevel::Err,
                    format!("Couldn't read the config: {}", e),
                ))
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
            Field::Enabled => self.enabled = !self.enabled,
            Field::Backend => {
                let idx = BACKEND_CHOICES
                    .iter()
                    .position(|c| *c == self.backend)
                    .map(|i| i as i32)
                    .unwrap_or(0);
                let n = (idx + delta).rem_euclid(BACKEND_CHOICES.len() as i32);
                self.backend = BACKEND_CHOICES[n as usize].to_string();
            }
            Field::Threshold => {
                self.threshold = (self.threshold + delta as f32 * 0.05).clamp(0.0, 1.0);
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

/// `auto` reads as `Automatic`, `energy` as `Loudness`, `whisper` as `Silero`.
fn backend_name(backend: &str) -> String {
    match backend {
        "auto" => "Automatic".to_string(),
        "energy" => "Loudness".to_string(),
        "whisper" => "Silero".to_string(),
        other => other.to_string(),
    }
}

/// The page's rows, from its in-memory settings.
pub fn rows(state: &VadState) -> Vec<FormRowSpec> {
    let off = !state.enabled;
    let backends: Vec<String> = BACKEND_CHOICES.iter().map(|b| backend_name(b)).collect();
    vec![
        FormRowSpec::new(
            state.field == Field::Enabled,
            "Voice detection",
            yesno(state.enabled),
        ),
        FormRowSpec::new(
            state.field == Field::Backend,
            "Method",
            backend_name(&state.backend),
        )
        .choices(&backends)
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::Threshold,
            "Sensitivity",
            format!("{:.2}", state.threshold),
        )
        .dimmed(off),
    ]
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.vad else {
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

fn guidance_for_field(state: &VadState) -> Vec<Line<'_>> {
    let text = |s: &'static str| Line::from(s);
    let dim = |s: &'static str| Line::from(Span::styled(s, theme::muted()));
    match state.field {
        Field::Enabled => vec![text(
            "Skips recordings with no speech in them, so an accidental press \
             doesn't type a made-up \"Thank you.\"",
        )],
        Field::Backend => vec![text(
            "Loudness is fast and needs nothing extra. Silero is more \
                 accurate in noise and needs its model: voxtype setup vad. \
                 Automatic uses Silero with Whisper and Loudness otherwise.",
        )],
        Field::Threshold => vec![
            text(
                "How sure voxtype must be that you spoke, from 0 to 1. Raise it \
                 if background noise gets transcribed; lower it if real speech \
                 is skipped.",
            ),
            dim("0.50 suits most microphones."),
        ],
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.vad.as_mut() {
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
