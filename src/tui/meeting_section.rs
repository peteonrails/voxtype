//! Meeting mode settings: enabled, audio source, diarization on/off.

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
                self.feedback = Some((FeedbackLevel::Err, format!("load: {}", e)));
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
            Field::Enabled => self.enabled = !self.enabled,
            Field::Diarization => self.diarization_enabled = !self.diarization_enabled,
            Field::AudioSource => {
                let idx = SOURCES
                    .iter()
                    .position(|s| *s == self.audio_source)
                    .map(|i| i as i32)
                    .unwrap_or(0);
                self.audio_source = SOURCES
                    [(idx + delta).rem_euclid(SOURCES.len() as i32) as usize]
                    .to_string();
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.meeting {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Meeting");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(Paragraph::new("Failed to load config.").wrap(Wrap { trim: true }), inner);
            return;
        }
    };

    let dim_when_off = !state.enabled;
    let rows = vec![
        FormRowSpec::new(state.field == Field::Enabled, "Meeting mode", yesno(state.enabled)),
        FormRowSpec::new(
            state.field == Field::Diarization,
            "Speaker diarization",
            yesno(state.diarization_enabled),
        )
        .dimmed(dim_when_off),
        FormRowSpec::new(
            state.field == Field::AudioSource,
            "Audio source",
            &state.audio_source,
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
        "Meeting Mode",
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

fn guidance_for_field(state: &MeetingState) -> Vec<Line<'_>> {
    match state.field {
        Field::Enabled => vec![
            heading("Meeting mode"),
            Line::from(""),
            Line::from(
                "Long-form recording mode. Voxtype chunks audio into \
                 segments, transcribes each, and stitches a continuous \
                 transcript with timestamps.",
            ),
            Line::from(""),
            Line::from(
                "Persists segments to ~/.local/share/voxtype/meetings/ so a \
                 crash doesn't lose your transcript.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Other [meeting.*] fields (chunk duration, summary command, \
                 storage path) live in config.toml directly.",
                Style::default().fg(Color::Gray),
            )),
        ],
        Field::Diarization => vec![
            heading("Speaker diarization"),
            Line::from(""),
            Line::from(
                "Tags each segment with a speaker label (Speaker 1, \
                 Speaker 2, …) so the transcript reads like dialogue.",
            ),
            Line::from(""),
            Line::from(
                "Uses an ONNX speaker-embedding model (ECAPA-TDNN) plus \
                 clustering. Requires the ml-diarization feature in your \
                 build.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Off by default — adds CPU cost and isn't useful for \
                 single-speaker dictation.",
                Style::default().fg(Color::Gray),
            )),
        ],
        Field::AudioSource => vec![
            heading("Audio source"),
            Line::from(""),
            Line::from(Span::styled(
                "mic: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Microphone only. Standard interview/podcast capture."),
            Line::from(""),
            Line::from(Span::styled(
                "system: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "System audio (loopback) only. Captures meeting playback \
                 from Zoom/Meet/etc.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "both: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Mic + system loopback. Voxtype runs GTCRN echo cancellation \
                 to keep your voice from doubling.",
            ),
        ],
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
