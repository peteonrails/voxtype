//! Text-processing settings: spoken punctuation, smart auto-submit, and an
//! inline editor for the [text.replacements] map.

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
pub struct TextState {
    pub spoken_punctuation: bool,
    pub smart_auto_submit: bool,
    /// Sorted by key for stable display. The user can edit keys/values via
    /// the inline editor below.
    pub replacements: Vec<(String, String)>,
    /// Set of original keys at load time, so save() can detect deletions.
    pub original_keys: Vec<String>,
    pub cursor: usize,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
    pub editing: Option<ReplacementEdit>,
}

/// Editing state for the replacement list. Users edit the key first, then
/// the value; commit on the value commits the whole pair.
#[derive(Debug, Clone)]
pub struct ReplacementEdit {
    pub target: EditTarget,
    pub phase: EditPhase,
    pub key_buffer: String,
    pub input: TextInput,
}

#[derive(Debug, Clone, Copy)]
pub enum EditTarget {
    /// Editing the replacement at this index in `replacements`.
    Existing(usize),
    /// Adding a new replacement at the end of the list.
    New,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditPhase {
    Key,
    Value,
}

/// Row-position vocabulary. Position 0 is the first toggle, and the last
/// position is always the "+ Add new replacement" row.
fn toggle_count() -> usize {
    2
}

fn add_row_index(replacements: &[(String, String)]) -> usize {
    toggle_count() + replacements.len()
}

fn total_rows(replacements: &[(String, String)]) -> usize {
    add_row_index(replacements) + 1
}

impl TextState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        let replacements = read_replacements(&ed);
        let original_keys: Vec<String> = replacements.iter().map(|(k, _)| k.clone()).collect();
        Ok(Self {
            spoken_punctuation: ed.get_bool("text", "spoken_punctuation").unwrap_or(false),
            smart_auto_submit: ed.get_bool("text", "smart_auto_submit").unwrap_or(false),
            replacements,
            original_keys,
            cursor: 0,
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
        ed.set_bool("text", "spoken_punctuation", self.spoken_punctuation);
        ed.set_bool("text", "smart_auto_submit", self.smart_auto_submit);

        // Replacements: write every current entry, then unset any original
        // keys that are no longer in the list (deletions).
        let current_keys: std::collections::HashSet<&String> =
            self.replacements.iter().map(|(k, _)| k).collect();
        for original in &self.original_keys {
            if !current_keys.contains(original) {
                ed.unset("text.replacements", original);
            }
        }
        for (k, v) in &self.replacements {
            ed.set_string("text.replacements", k, v);
        }

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.original_keys = self.replacements.iter().map(|(k, _)| k.clone()).collect();
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
            let cursor = self.cursor.min(total_rows(&fresh.replacements).saturating_sub(1));
            *self = fresh;
            self.cursor = cursor;
            self.feedback = Some((FeedbackLevel::Ok, "Reverted unsaved changes".to_string()));
        }
    }

    fn move_field(&mut self, delta: i32) {
        let len = total_rows(&self.replacements) as i32;
        let new = (self.cursor as i32 + delta).rem_euclid(len);
        self.cursor = new as usize;
    }

    fn cycle(&mut self) {
        match self.cursor {
            0 => self.spoken_punctuation = !self.spoken_punctuation,
            1 => self.smart_auto_submit = !self.smart_auto_submit,
            _ => {} // replacement / add rows don't cycle
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }

    fn start_edit(&mut self) {
        let target = if self.cursor == add_row_index(&self.replacements) {
            EditTarget::New
        } else if self.cursor >= toggle_count() {
            EditTarget::Existing(self.cursor - toggle_count())
        } else {
            return; // toggles, not editable as text
        };

        let initial_key = match target {
            EditTarget::Existing(i) => self.replacements[i].0.clone(),
            EditTarget::New => String::new(),
        };

        self.editing = Some(ReplacementEdit {
            target,
            phase: EditPhase::Key,
            key_buffer: String::new(),
            input: TextInput::new(initial_key),
        });
    }

    fn delete_replacement_at_cursor(&mut self) {
        if self.cursor >= toggle_count() && self.cursor < add_row_index(&self.replacements) {
            let idx = self.cursor - toggle_count();
            self.replacements.remove(idx);
            self.dirty_since_load = true;
            self.feedback = None;
            // Clamp cursor in case we removed the last entry.
            let max = total_rows(&self.replacements).saturating_sub(1);
            if self.cursor > max {
                self.cursor = max;
            }
        }
    }

    /// Called when the inline TextInput commits. Advances the edit phase or
    /// finalizes the replacement.
    fn commit_edit(&mut self) {
        let Some(edit) = self.editing.take() else {
            return;
        };
        let buf = edit.input.buffer().to_string();
        match edit.phase {
            EditPhase::Key => {
                let trimmed = buf.trim().to_string();
                if trimmed.is_empty() {
                    // Empty key → cancel the whole flow.
                    self.feedback = None;
                    return;
                }
                let initial_value = match edit.target {
                    EditTarget::Existing(i) => self.replacements[i].1.clone(),
                    EditTarget::New => String::new(),
                };
                self.editing = Some(ReplacementEdit {
                    target: edit.target,
                    phase: EditPhase::Value,
                    key_buffer: trimmed,
                    input: TextInput::new(initial_value),
                });
            }
            EditPhase::Value => {
                let key = edit.key_buffer;
                let value = buf;
                if value.is_empty() {
                    // Empty value is allowed but doesn't make much sense; treat
                    // as a cancel for the new-entry flow.
                    if let EditTarget::New = edit.target {
                        return;
                    }
                }
                match edit.target {
                    EditTarget::Existing(i) => {
                        // Key may have changed; rewrite the entry in place.
                        self.replacements[i] = (key, value);
                    }
                    EditTarget::New => {
                        self.replacements.push((key, value));
                    }
                }
                self.replacements.sort_by(|a, b| a.0.cmp(&b.0));
                self.dirty_since_load = true;
                self.feedback = None;
            }
        }
    }
}

fn read_replacements(ed: &ConfigEditor) -> Vec<(String, String)> {
    // Walk the [text.replacements] table directly via toml_edit, since the
    // ConfigEditor accessor only returns single keyed values.
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(table) = ed.raw_table("text.replacements") {
        for (k, v) in table.iter() {
            if let Some(s) = v.as_value().and_then(|v| v.as_str()) {
                out.push((k.to_string(), s.to_string()));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.text {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Text");
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

    let editing_idx = state.editing.as_ref().and_then(|e| match e.target {
        EditTarget::Existing(i) => Some(i),
        EditTarget::New => None,
    });
    let editing_new = matches!(state.editing.as_ref().map(|e| e.target), Some(EditTarget::New));

    let mut rows: Vec<FormRowSpec> = Vec::new();

    rows.push(FormRowSpec::new(
        state.cursor == 0,
        "Spoken punctuation conversion",
        yesno(state.spoken_punctuation),
    ));
    rows.push(FormRowSpec::new(
        state.cursor == 1,
        "Smart auto-submit on \"submit\"",
        yesno(state.smart_auto_submit),
    ));

    for (i, (k, v)) in state.replacements.iter().enumerate() {
        let row_idx = toggle_count() + i;
        let label = format!("\"{}\"", k);
        let value = if editing_idx == Some(i) {
            replacement_edit_value(state)
        } else {
            format!("→ \"{}\"", v)
        };
        rows.push(FormRowSpec::new(state.cursor == row_idx, label, value));
    }

    let add_idx = add_row_index(&state.replacements);
    let add_label = if editing_new {
        "(new entry)".to_string()
    } else {
        "+ Add new replacement".to_string()
    };
    let add_value = if editing_new {
        replacement_edit_value(state)
    } else {
        "press Enter".to_string()
    };
    rows.push(FormRowSpec::new(state.cursor == add_idx, add_label, add_value));

    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|(lvl, msg)| (*lvl, msg.as_str()));

    common::render_form_with_guidance(
        f,
        area,
        "Text",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance(state),
    );
}

fn replacement_edit_value(state: &TextState) -> String {
    let Some(edit) = state.editing.as_ref() else {
        return String::new();
    };
    match edit.phase {
        EditPhase::Key => format!("editing key: {}", edit.input.caret_string()),
        EditPhase::Value => format!(
            "\"{}\" → {}",
            edit.key_buffer,
            edit.input.caret_string()
        ),
    }
}

fn yesno(b: bool) -> String {
    (if b { "yes" } else { "no" }).to_string()
}

fn heading(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        text.into(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn guidance(state: &TextState) -> Vec<Line<'static>> {
    let total = total_rows(&state.replacements);
    let on_replacement_row = state.cursor >= toggle_count() && state.cursor < total - 1;
    let on_add_row = state.cursor == total - 1;

    if let Some(edit) = state.editing.as_ref() {
        let header = match edit.phase {
            EditPhase::Key => "✎ Editing key — Enter for value, Esc to cancel",
            EditPhase::Value => "✎ Editing value — Enter to commit, Esc to cancel",
        };
        return vec![
            Line::from(Span::styled(
                header,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(
                "Replacements run as a case-insensitive substring match \
                 across the transcript before output. The dictated word goes \
                 on the left, the replacement on the right.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Examples:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("  \"vox type\"  →  \"voxtype\""),
            Line::from("  \"a i\"       →  \"AI\""),
            Line::from("  \"slack\"     →  \"Slack\""),
        ];
    }

    if state.cursor == 0 {
        return vec![
            heading("Spoken punctuation"),
            Line::from(""),
            Line::from(
                "Maps words like \"period\", \"comma\", \"question mark\", \
                 \"new line\" to their symbol equivalents in the transcript.",
            ),
            Line::from(""),
            Line::from(
                "Useful when the model can't reliably punctuate from prosody \
                 (smaller Whisper models, accented speech).",
            ),
        ];
    }

    if state.cursor == 1 {
        return vec![
            heading("Smart auto-submit"),
            Line::from(""),
            Line::from(
                "Watches for \"submit\" at the end of a recording. If found, \
                 voxtype strips it and presses Enter for you.",
            ),
            Line::from(""),
            Line::from(
                "Pair with [output] auto_submit = false: most dictations \
                 don't auto-send, but ending with \"submit\" explicitly fires \
                 Enter.",
            ),
        ];
    }

    if on_replacement_row {
        let idx = state.cursor - toggle_count();
        let (k, v) = &state.replacements[idx];
        return vec![
            heading("Custom replacement"),
            Line::from(""),
            Line::from(format!("  \"{}\"  →  \"{}\"", k, v)),
            Line::from(""),
            Line::from(
                "Press Enter to edit (key first, then value). Press d to \
                 delete this entry.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Replacements run before output. Match is case-insensitive \
                 and operates on the whole transcript text.",
                Style::default().fg(Color::Gray),
            )),
        ];
    }

    if on_add_row {
        return vec![
            heading("Add a replacement"),
            Line::from(""),
            Line::from(
                "Press Enter to start a new entry. You'll be prompted for the \
                 key first, then the value.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Examples:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("  \"vox type\"  →  \"voxtype\""),
            Line::from("  \"hyperland\" →  \"Hyprland\""),
            Line::from("  \"github\"    →  \"GitHub\""),
        ];
    }

    Vec::new()
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.text.as_mut() {
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
        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l')
        | KeyCode::Char(' ') => {
            state.cycle();
            Action::None
        }
        KeyCode::Enter | KeyCode::Char('i') => {
            // Enter on toggles flips them; on replacement rows starts edit.
            if state.cursor < toggle_count() {
                state.cycle();
            } else {
                state.start_edit();
            }
            Action::None
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            state.delete_replacement_at_cursor();
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

fn handle_edit_key(state: &mut TextState, key: KeyEvent) -> Action {
    let Some(editing) = state.editing.as_mut() else {
        return Action::None;
    };
    match editing.input.handle_key(key) {
        TextInputResult::Continue => Action::None,
        TextInputResult::Commit => {
            state.commit_edit();
            Action::None
        }
        TextInputResult::Cancel => {
            state.editing = None;
            Action::None
        }
    }
}
