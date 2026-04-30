//! Voice Activity Detection settings.

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
            threshold: ed
                .get_string("vad", "threshold")
                .and_then(|s| s.parse().ok())
                .or_else(|| ed.get_int("vad", "threshold").map(|n| n as f32))
                .unwrap_or(0.5),
            field: Field::Enabled,
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
        ed.set_bool("vad", "enabled", self.enabled);
        ed.set_string("vad", "backend", &self.backend);
        ed.set_string("vad", "threshold", &format!("{:.2}", self.threshold));
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
        match Self::load() {
            Ok(fresh) => {
                let field = self.field;
                *self = fresh;
                self.field = field;
                self.feedback = Some((FeedbackLevel::Ok, "Reverted".to_string()));
            }
            Err(e) => self.feedback = Some((FeedbackLevel::Err, format!("reload: {}", e))),
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

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.vad {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("VAD");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(
                Paragraph::new("Failed to load config.").wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }
    };

    let dim_when_off = !state.enabled;
    let rows = vec![
        FormRowSpec::new(state.field == Field::Enabled, "Enabled", yesno(state.enabled)),
        FormRowSpec::new(state.field == Field::Backend, "Backend", &state.backend)
            .dimmed(dim_when_off),
        FormRowSpec::new(
            state.field == Field::Threshold,
            "Speech threshold",
            format!("{:.2}", state.threshold),
        )
        .dimmed(dim_when_off),
    ];

    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|(lvl, msg)| (*lvl, msg.as_str()));

    common::render_form_with_guidance(
        f,
        area,
        "Voice Activity Detection",
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

fn guidance_for_field(state: &VadState) -> Vec<Line<'_>> {
    match state.field {
        Field::Enabled => vec![
            heading("Voice Activity Detection"),
            Line::from(""),
            Line::from(
                "Filters out silence-only recordings before transcription. \
                 Without VAD, Whisper sometimes hallucinates phrases like \
                 \"Thank you.\" on a clip with no speech.",
            ),
            Line::from(""),
            Line::from(
                "Keep this on if you sometimes accidentally tap the PTT key \
                 without speaking, or use toggle mode and forget you started \
                 a recording.",
            ),
        ],
        Field::Backend => vec![
            heading("VAD backend"),
            Line::from(""),
            Line::from(Span::styled(
                "auto: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Whisper VAD for the Whisper engine; Energy VAD for ONNX. \
                 Pick this unless you want to override.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "energy: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "RMS-amplitude threshold. Fast, no model needed, works with \
                 any engine. Less accurate in noisy environments.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "whisper: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Silero VAD via whisper-rs. Most accurate. Requires \
                 ggml-silero-vad.bin (run `voxtype setup vad` to fetch).",
            ),
        ],
        Field::Threshold => vec![
            heading("Speech threshold"),
            Line::from(""),
            Line::from(
                "0.0-1.0. Higher values mean voxtype demands more confident \
                 speech detection before transcribing.",
            ),
            Line::from(""),
            Line::from(
                "0.5 is the default and works for most setups. Bump higher \
                 (0.65-0.75) if voxtype occasionally transcribes background \
                 noise. Lower (0.35-0.45) if it's rejecting your real speech.",
            ),
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
