//! Output section: how transcribed text is delivered.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FormRowSpec, TextInput, TextInputResult};
use super::config_editor::{ConfigEditor, EditorError};

#[derive(Debug, Clone)]
pub struct OutputState {
    pub mode: String,
    pub fallback_to_clipboard: bool,
    pub auto_submit: bool,
    pub shift_enter_newlines: bool,
    pub pre_type_delay_ms: i64,
    pub append_text: Option<String>,
    pub post_process_command: Option<String>,
    pub field: Field,
    pub feedback: Option<Feedback>,
    pub dirty_since_load: bool,
    pub editing: Option<TextEdit>,
}

#[derive(Debug, Clone)]
pub struct TextEdit {
    pub field: Field,
    pub input: TextInput,
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
    Mode,
    Fallback,
    AutoSubmit,
    ShiftEnterNewlines,
    PreTypeDelay,
    AppendText,
    PostProcess,
}

impl Field {
    const ALL: &'static [Field] = &[
        Field::Mode,
        Field::Fallback,
        Field::AutoSubmit,
        Field::ShiftEnterNewlines,
        Field::PreTypeDelay,
        Field::AppendText,
        Field::PostProcess,
    ];
}

const MODE_CHOICES: &[&str] = &["type", "clipboard", "paste", "file"];
const APPEND_CHOICES: &[Option<&str>] = &[None, Some(" "), Some("\n"), Some(". ")];
const POST_PROCESS_PRESETS: &[Option<&str>] = &[
    None,
    Some("ollama run llama3.2 'Polish: '"),
    Some("sed 's/uh, //g'"),
];
const DELAY_STEP: i64 = 25;

impl OutputState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            mode: ed
                .get_string("output", "mode")
                .unwrap_or_else(|| "type".to_string()),
            fallback_to_clipboard: ed
                .get_bool("output", "fallback_to_clipboard")
                .unwrap_or(true),
            auto_submit: ed.get_bool("output", "auto_submit").unwrap_or(false),
            shift_enter_newlines: ed
                .get_bool("output", "shift_enter_newlines")
                .unwrap_or(false),
            pre_type_delay_ms: ed.get_int("output", "pre_type_delay_ms").unwrap_or(0),
            append_text: ed.get_string("output", "append_text"),
            post_process_command: ed.get_string("post_process", "command"),
            field: Field::Mode,
            feedback: None,
            dirty_since_load: false,
            editing: None,
        })
    }

    fn is_text_field(field: Field) -> bool {
        matches!(field, Field::AppendText | Field::PostProcess)
    }

    fn start_edit_if_text_field(&mut self) -> bool {
        if !Self::is_text_field(self.field) {
            return false;
        }
        let initial = match self.field {
            Field::AppendText => self.append_text.clone().unwrap_or_default(),
            Field::PostProcess => self.post_process_command.clone().unwrap_or_default(),
            _ => String::new(),
        };
        self.editing = Some(TextEdit {
            field: self.field,
            input: TextInput::new(initial),
        });
        true
    }

    fn commit_text_edit(&mut self, field: Field, buffer: String) {
        match field {
            Field::AppendText => {
                self.append_text = if buffer.is_empty() { None } else { Some(buffer) };
            }
            Field::PostProcess => {
                self.post_process_command = if buffer.trim().is_empty() {
                    None
                } else {
                    Some(buffer)
                };
            }
            _ => {}
        }
        self.dirty_since_load = true;
        self.feedback = None;
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
        ed.set_string("output", "mode", &self.mode);
        ed.set_bool(
            "output",
            "fallback_to_clipboard",
            self.fallback_to_clipboard,
        );
        ed.set_bool("output", "auto_submit", self.auto_submit);
        ed.set_bool("output", "shift_enter_newlines", self.shift_enter_newlines);
        ed.set_int("output", "pre_type_delay_ms", self.pre_type_delay_ms);
        match &self.append_text {
            Some(t) => ed.set_string("output", "append_text", t),
            None => ed.unset("output", "append_text"),
        }
        match &self.post_process_command {
            Some(c) if !c.is_empty() => ed.set_string("post_process", "command", c),
            _ => ed.unset("post_process", "command"),
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

    fn cycle(&mut self, delta: i32) {
        match self.field {
            Field::Mode => {
                let idx = MODE_CHOICES
                    .iter()
                    .position(|c| *c == self.mode)
                    .map(|i| i as i32)
                    .unwrap_or(0);
                let n = (idx + delta).rem_euclid(MODE_CHOICES.len() as i32);
                self.mode = MODE_CHOICES[n as usize].to_string();
            }
            Field::Fallback => self.fallback_to_clipboard = !self.fallback_to_clipboard,
            Field::AutoSubmit => self.auto_submit = !self.auto_submit,
            Field::ShiftEnterNewlines => self.shift_enter_newlines = !self.shift_enter_newlines,
            Field::PreTypeDelay => {
                self.pre_type_delay_ms =
                    (self.pre_type_delay_ms + delta as i64 * DELAY_STEP).clamp(0, 5000);
            }
            Field::AppendText => {
                let idx = APPEND_CHOICES
                    .iter()
                    .position(|c| c.as_deref() == self.append_text.as_deref())
                    .map(|i| i as i32)
                    .unwrap_or(0);
                let n = (idx + delta).rem_euclid(APPEND_CHOICES.len() as i32);
                self.append_text = APPEND_CHOICES[n as usize].map(|s| s.to_string());
            }
            Field::PostProcess => {
                let idx = POST_PROCESS_PRESETS
                    .iter()
                    .position(|c| c.as_deref() == self.post_process_command.as_deref())
                    .map(|i| i as i32)
                    .unwrap_or(0);
                let n = (idx + delta).rem_euclid(POST_PROCESS_PRESETS.len() as i32);
                self.post_process_command =
                    POST_PROCESS_PRESETS[n as usize].map(|s| s.to_string());
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.output {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Output");
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

    let rows = vec![
        FormRowSpec::new(state.field == Field::Mode, "Output mode", &state.mode),
        FormRowSpec::new(
            state.field == Field::Fallback,
            "Fallback to clipboard",
            yesno(state.fallback_to_clipboard),
        ),
        FormRowSpec::new(
            state.field == Field::AutoSubmit,
            "Auto-submit (press Enter)",
            yesno(state.auto_submit),
        ),
        FormRowSpec::new(
            state.field == Field::ShiftEnterNewlines,
            "Newlines as Shift+Enter",
            yesno(state.shift_enter_newlines),
        ),
        FormRowSpec::new(
            state.field == Field::PreTypeDelay,
            "Pre-type delay (ms)",
            state.pre_type_delay_ms.to_string(),
        ),
        FormRowSpec::new(
            state.field == Field::AppendText,
            "Append after each",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::AppendText => e.input.caret_string(),
                _ => display_append(state.append_text.as_deref()),
            },
        ),
        FormRowSpec::new(
            state.field == Field::PostProcess,
            "Post-process command",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::PostProcess => e.input.caret_string(),
                _ => state
                    .post_process_command
                    .as_deref()
                    .map(|s| {
                        if s.len() > 30 {
                            format!("{}…", &s[..30])
                        } else {
                            s.to_string()
                        }
                    })
                    .unwrap_or_else(|| "(none)".to_string()),
            },
        ),
    ];

    let feedback_pair = state.feedback.as_ref().map(|fb| {
        (
            match fb.level {
                FeedbackLevel::Ok => common::FeedbackLevel::Ok,
                FeedbackLevel::Err => common::FeedbackLevel::Err,
            },
            fb.message.as_str(),
        )
    });

    common::render_form_with_guidance(
        f,
        area,
        "Output",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance_for_field(state),
    );
}

fn yesno(b: bool) -> String {
    (if b { "yes" } else { "no" }).to_string()
}

fn display_append(s: Option<&str>) -> String {
    match s {
        None => "(none)".to_string(),
        Some(" ") => "space".to_string(),
        Some("\n") => "newline".to_string(),
        Some(other) => format!("{:?}", other),
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

fn guidance_for_field(state: &OutputState) -> Vec<Line<'_>> {
    match state.field {
        Field::Mode => vec![
            heading("Output mode"),
            Line::from(""),
            Line::from(Span::styled(
                "type: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Simulates keyboard typing via wtype → dotool → ydotool fallback. \
                 Default; works in most apps.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "clipboard: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Puts text on the clipboard only — you paste it yourself."),
            Line::from(""),
            Line::from(Span::styled(
                "paste: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Clipboard + Ctrl+V. Faster than typing for long transcripts."),
            Line::from(""),
            Line::from(Span::styled(
                "file: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Appends to a file. Set [output] file_path before using.",
            ),
        ],
        Field::Fallback => vec![
            heading("Fallback to clipboard"),
            Line::from(""),
            Line::from(
                "When the active output method fails (no compositor support, \
                 no daemon running, etc.), drop the transcript on the \
                 clipboard so you don't lose it.",
            ),
            Line::from(""),
            Line::from(
                "Recommend keeping this on. The only reason to disable is if \
                 you want voxtype to fail loudly when typing breaks — useful \
                 in scripted setups.",
            ),
        ],
        Field::AutoSubmit => vec![
            heading("Auto-submit"),
            Line::from(""),
            Line::from(
                "After typing the transcript, press Enter automatically.",
            ),
            Line::from(""),
            Line::from(
                "Useful for chat boxes (Slack, Discord, terminal prompts) \
                 where you'd hit Enter anyway. Skip if you typically want to \
                 review/edit before sending.",
            ),
        ],
        Field::ShiftEnterNewlines => vec![
            heading("Newlines as Shift+Enter"),
            Line::from(""),
            Line::from(
                "Convert any newline in the transcript to Shift+Enter \
                 instead of regular Enter.",
            ),
            Line::from(""),
            Line::from(
                "Match this to apps where Enter submits and Shift+Enter \
                 inserts a newline (Cursor, Slack, Discord, ChatGPT, …). \
                 Otherwise multi-line dictations submit prematurely.",
            ),
        ],
        Field::PreTypeDelay => vec![
            heading("Pre-type delay"),
            Line::from(""),
            Line::from(
                "Milliseconds to wait before voxtype starts typing. Helps \
                 some compositors that drop the first character if the \
                 virtual keyboard hasn't fully initialized.",
            ),
            Line::from(""),
            Line::from(
                "0 is the default. If you see the first character of \
                 transcripts dropped, bump to 50-100ms.",
            ),
        ],
        Field::AppendText => vec![
            heading("Append after each transcription"),
            Line::from(""),
            Line::from(
                "Adds a fixed string after every transcription, before \
                 auto-submit fires. Lets you tack on a separator without \
                 saying it.",
            ),
            Line::from(""),
            Line::from(
                "space: dictate sentences incrementally and end up with \
                 \"Sentence one. Sentence two.\" without manual spacing.",
            ),
            Line::from(""),
            Line::from(
                "newline: list-style notes where each PTT press should \
                 start a new line.",
            ),
        ],
        Field::PostProcess => vec![
            heading("Post-process command"),
            Line::from(""),
            Line::from(
                "Pipes the transcript through an external command before \
                 output. The transcript goes in via stdin; the command's \
                 stdout is what gets typed.",
            ),
            Line::from(""),
            Line::from(
                "Common uses: local LLM cleanup (Ollama), filler-word \
                 stripping (sed), markdown formatting.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "TUI cycles a few presets. Edit the command body in \
                 [post_process] command in config.toml directly.",
                Style::default().fg(Color::Gray),
            )),
        ],
    }
}

impl From<FeedbackLevel> for super::common::FeedbackLevel {
    fn from(v: FeedbackLevel) -> Self {
        match v {
            FeedbackLevel::Ok => super::common::FeedbackLevel::Ok,
            FeedbackLevel::Err => super::common::FeedbackLevel::Err,
        }
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.output.as_mut() {
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

fn handle_edit_key(state: &mut OutputState, key: KeyEvent) -> Action {
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
