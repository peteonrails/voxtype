//! Output section: how transcribed text is delivered.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FormRowSpec, TextInput, TextInputResult};
use super::config_editor::{ConfigEditor, EditorError};
use super::theme;

#[derive(Debug, Clone)]
pub struct OutputState {
    pub mode: String,
    /// Where `file` mode appends; `None` when unset.
    pub file_path: Option<String>,
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
    FilePath,
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
        Field::FilePath,
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
            file_path: ed.get_string("output", "file_path"),
            fallback_to_clipboard: ed
                .get_bool("output", "fallback_to_clipboard")
                .unwrap_or(true),
            auto_submit: ed.get_bool("output", "auto_submit").unwrap_or(false),
            shift_enter_newlines: ed
                .get_bool("output", "shift_enter_newlines")
                .unwrap_or(false),
            pre_type_delay_ms: ed.get_int("output", "pre_type_delay_ms").unwrap_or(0),
            append_text: ed.get_string("output", "append_text"),
            post_process_command: ed.get_string("output.post_process", "command"),
            field: Field::Mode,
            feedback: None,
            dirty_since_load: false,
            editing: None,
        })
    }

    fn is_text_field(field: Field) -> bool {
        matches!(
            field,
            Field::FilePath | Field::AppendText | Field::PostProcess
        )
    }

    fn start_edit_if_text_field(&mut self) -> bool {
        if !Self::is_text_field(self.field) {
            return false;
        }
        let initial = match self.field {
            Field::FilePath => self.file_path.clone().unwrap_or_default(),
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
            Field::FilePath => {
                let trimmed = buffer.trim();
                self.file_path = (!trimmed.is_empty()).then(|| trimmed.to_string());
            }
            Field::AppendText => {
                self.append_text = if buffer.is_empty() {
                    None
                } else {
                    Some(buffer)
                };
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
                    message: format!("Couldn't read the config: {}", e),
                });
                return Action::None;
            }
        };
        ed.set_string("output", "mode", &self.mode);
        match &self.file_path {
            Some(path) => ed.set_string("output", "file_path", path),
            None => ed.unset("output", "file_path"),
        }
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
            Some(c) if !c.is_empty() => ed.set_string("output.post_process", "command", c),
            _ => ed.unset("output.post_process", "command"),
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
                    message: format!("Not saved: {}", e),
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
                    message: "Put back the saved settings".to_string(),
                });
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("Couldn't read the config: {}", e),
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
            // Typed only; Enter opens it.
            Field::FilePath => {
                self.start_edit_if_text_field();
                return;
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
                self.post_process_command = POST_PROCESS_PRESETS[n as usize].map(|s| s.to_string());
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

/// The page's rows, from its in-memory settings.
pub fn rows(state: &OutputState) -> Vec<FormRowSpec> {
    let typed = |field: Field| match state.editing.as_ref() {
        Some(e) if e.field == field => Some(e.input.caret_string()),
        _ => None,
    };
    let modes: Vec<String> = MODE_CHOICES.iter().map(|m| mode_name(m)).collect();
    vec![
        FormRowSpec::new(state.field == Field::Mode, "Output", mode_name(&state.mode))
            .choices(&modes),
        FormRowSpec::new(
            state.field == Field::FilePath,
            "File",
            typed(Field::FilePath).unwrap_or_else(|| {
                state
                    .file_path
                    .clone()
                    .unwrap_or_else(|| common::NOT_SET.to_string())
            }),
        )
        .dimmed(state.mode != "file"),
        FormRowSpec::new(
            state.field == Field::Fallback,
            "Fall back to clipboard",
            yesno(state.fallback_to_clipboard),
        ),
        FormRowSpec::new(
            state.field == Field::AutoSubmit,
            "Press Enter after",
            yesno(state.auto_submit),
        ),
        FormRowSpec::new(
            state.field == Field::ShiftEnterNewlines,
            "Line breaks as Shift+Enter",
            yesno(state.shift_enter_newlines),
        ),
        FormRowSpec::new(
            state.field == Field::PreTypeDelay,
            "Delay before typing",
            format!("{} ms", state.pre_type_delay_ms),
        ),
        FormRowSpec::new(
            state.field == Field::AppendText,
            "Add after the text",
            typed(Field::AppendText)
                .unwrap_or_else(|| display_append(state.append_text.as_deref())),
        ),
        FormRowSpec::new(
            state.field == Field::PostProcess,
            "Post-process command",
            typed(Field::PostProcess).unwrap_or_else(|| {
                state
                    .post_process_command
                    .clone()
                    .unwrap_or_else(|| common::NOT_SET.to_string())
            }),
        ),
    ]
}

/// Put the cursor on the row labelled `label`, for Basics.
pub fn focus_label(state: &mut OutputState, label: &str) {
    if let Some(i) = rows(state).iter().position(|r| r.label == label) {
        state.field = Field::ALL[i];
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.output else {
        common::render_load_error(f, area, app);
        return;
    };
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
        app,
        feedback_pair,
        &rows(state),
        guidance_for_field(state),
    );
}

fn yesno(b: bool) -> String {
    common::on_off(b)
}

fn display_append(s: Option<&str>) -> String {
    match s {
        None => "Nothing".to_string(),
        Some(" ") => "A space".to_string(),
        Some("\n") => "A new line".to_string(),
        Some(other) => format!("{:?}", other),
    }
}

/// `type` reads as `Type`.
fn mode_name(mode: &str) -> String {
    match mode {
        "type" => "Type".to_string(),
        "clipboard" => "Clipboard".to_string(),
        "paste" => "Paste".to_string(),
        "file" => "File".to_string(),
        other => other.to_string(),
    }
}

fn guidance_for_field(state: &OutputState) -> Vec<Line<'_>> {
    let text = |s: &'static str| Line::from(s);
    let dim = |s: &'static str| Line::from(Span::styled(s, theme::muted()));
    match state.field {
        Field::Mode => vec![
            text(
                "Type sends keystrokes to the focused window. Clipboard only copies \
                 the text. Paste copies it and presses the paste keys, which is \
                 faster for long text. File appends it to a file.",
            ),
            dim("Typing tries wtype, then dotool, then ydotool."),
        ],
        Field::FilePath => vec![
            text("The file that File output appends to."),
            dim("Enter types a path."),
        ],
        Field::Fallback => vec![text(
            "If typing fails, the text goes to the clipboard so it isn't lost. \
             Turn off only if you want failures to be loud.",
        )],
        Field::AutoSubmit => vec![text(
            "Presses Enter after the text, for chat boxes and prompts you would \
             send anyway.",
        )],
        Field::ShiftEnterNewlines => vec![text(
            "Types line breaks as Shift+Enter, for apps where Enter sends the \
             message (Slack, Discord, ChatGPT).",
        )],
        Field::PreTypeDelay => vec![text(
            "Waits before typing. Raise it to 50 or 100 ms if the first letter \
             goes missing.",
        )],
        Field::AppendText => vec![text(
            "Added after every transcription. A space lets sentences dictated one \
             at a time run on; a new line puts each on its own line.",
        )],
        Field::PostProcess => vec![
            text(
                "A command the text passes through before output: it reads the \
                 text on stdin and prints the result, for example a local LLM \
                 cleanup.",
            ),
            dim("←→ goes through examples; Enter types your own."),
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
