//! Recording queue settings for overlapping normal batch dictation.

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
pub struct RecordingState {
    pub queue_enabled: bool,
    pub queue_size: i64,
    pub field: Field,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    QueueEnabled,
    QueueSize,
}

impl Field {
    const ALL: &'static [Field] = &[Field::QueueEnabled, Field::QueueSize];
}

const TABLE: &str = "recording";
const DEFAULT_QUEUE_SIZE: i64 = 5;
const QUEUE_STEP: i64 = 1;

fn normalize_queue_size(value: i64) -> i64 {
    value.max(0)
}

impl RecordingState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            queue_enabled: ed.get_bool(TABLE, "queue_enabled").unwrap_or(false),
            queue_size: normalize_queue_size(
                ed.get_int(TABLE, "queue_size")
                    .unwrap_or(DEFAULT_QUEUE_SIZE),
            ),
            field: Field::QueueEnabled,
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
        ed.set_bool(TABLE, "queue_enabled", self.queue_enabled);
        ed.set_int(TABLE, "queue_size", self.queue_size);

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.feedback = Some((
                    FeedbackLevel::Ok,
                    format!("Saved to {}", ed.path().display()),
                ));
            }
            Err(e) => {
                self.feedback = Some((FeedbackLevel::Err, format!("save: {}", e)));
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
                self.feedback = Some((FeedbackLevel::Ok, "Reverted".to_string()));
            }
            Err(e) => {
                self.feedback = Some((FeedbackLevel::Err, format!("reload: {}", e)));
            }
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
            Field::QueueEnabled => self.queue_enabled = !self.queue_enabled,
            Field::QueueSize => {
                self.queue_size = normalize_queue_size(self.queue_size + delta as i64 * QUEUE_STEP);
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.recording {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Recording");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(
                Paragraph::new("Failed to load config.").wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }
    };

    let rows = vec![
        FormRowSpec::new(
            state.field == Field::QueueEnabled,
            "Queue normal batch recordings",
            yesno(state.queue_enabled),
        ),
        FormRowSpec::new(
            state.field == Field::QueueSize,
            "Maximum queued recordings",
            state.queue_size.to_string(),
        ),
    ];

    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|(lvl, msg)| (*lvl, msg.as_str()));

    common::render_form_with_guidance(
        f,
        area,
        "Recording",
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

fn guidance_for_field(state: &RecordingState) -> Vec<Line<'_>> {
    match state.field {
        Field::QueueEnabled => vec![
            heading("Recording queue"),
            Line::from(""),
            Line::from(concat!(
                "When enabled, normal batch dictation requests are queued while ",
                "a previous normal batch is transcribing or outputting."
            )),
            Line::from(""),
            Line::from("The queue is FIFO and applies only to push-to-talk batch dictation."),
            Line::from("Starting a live recording requires one available stopped slot."),
            Line::from(""),
            Line::from("Turn it on if:"),
            Line::from(concat!(
                "  - You need to keep dictation fluid when each utterance is shorter ",
                "than transcription can keep up with."
            )),
            Line::from("Leave it off if:"),
            Line::from(
                "  - You use eager processing or streaming modes and want immediate cancellation/ordering behavior to remain unchanged.",
            ),
            Line::from(
                "  - Queueing is ignored with eager/streaming modes, and the daemon logs a startup warning.",
            ),
        ],
        Field::QueueSize => vec![
            heading("Maximum queued recordings"),
            Line::from(""),
            Line::from("How many stopped recordings can wait, transcribe, or output."),
            Line::from("0 or 1 disables queueing; minimum enabled value is 2."),
            Line::from("Live capture is counted only when it stops into the queue."),
            Line::from("Set a larger value to allow more queued recordings."),
            Line::from(""),
            Line::from("Set to 0 or 1 to disable queueing even when queue-enabled is true."),
            Line::from(Span::styled(
                "If the queue is full, new batch recordings are rejected.",
                Style::default().fg(Color::Gray),
            )),
        ],
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.recording.as_mut() {
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
