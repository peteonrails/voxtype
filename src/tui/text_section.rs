//! Text Cleanup: independent deterministic stages, optional model-assisted
//! cleanup, and inline editors for the local model path and replacements.

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
    pub filter_filler_words: bool,
    pub collapse_restarts: bool,
    pub repair_sentence_breaks: bool,
    pub format_numbers: bool,
    pub lard_disfluency: bool,
    pub punctuation_casing: bool,
    original_switches: [bool; 8],
    pub lard_model_path: Option<String>,
    original_model_path: Option<String>,
    pub model_edit: Option<TextInput>,
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
    8
}

fn replacement_start() -> usize {
    toggle_count() + 1 // local model path follows the switches
}

fn add_row_index(replacements: &[(String, String)]) -> usize {
    replacement_start() + replacements.len()
}

fn total_rows(replacements: &[(String, String)]) -> usize {
    add_row_index(replacements) + 1
}

impl TextState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Self::from_editor(&ed)
    }

    fn from_editor(ed: &ConfigEditor) -> Result<Self, EditorError> {
        let replacements = read_replacements(ed);
        let original_keys: Vec<String> = replacements.iter().map(|(k, _)| k.clone()).collect();
        // Deserialize just the file layer. In particular, an absent [text] uses
        // TextConfig::default(), while a present table uses serde field defaults.
        // Never persist effective CLI/environment overrides into the file.
        let config: crate::config::TextConfig = match ed.raw_table("text") {
            Some(table) => toml::from_str(&table.to_string())
                .map_err(|e| EditorError::Validate(e.to_string()))?,
            None => crate::config::TextConfig::default(),
        };
        let original_switches = [
            config.spoken_punctuation,
            config.filter_filler_words,
            config.collapse_restarts,
            config.repair_sentence_breaks,
            config.format_numbers,
            config.lard_disfluency,
            config.punctuation_casing,
            config.smart_auto_submit,
        ];
        Ok(Self {
            spoken_punctuation: config.spoken_punctuation,
            smart_auto_submit: config.smart_auto_submit,
            filter_filler_words: config.filter_filler_words,
            collapse_restarts: config.collapse_restarts,
            repair_sentence_breaks: config.repair_sentence_breaks,
            format_numbers: config.format_numbers,
            lard_disfluency: config.lard_disfluency,
            punctuation_casing: config.punctuation_casing,
            original_switches,
            lard_model_path: ed.get_string("text", "lard_model_path"),
            original_model_path: ed.get_string("text", "lard_model_path"),
            model_edit: None,
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
        self.save_with_editor(&mut ed)
    }

    fn save_with_editor(&mut self, ed: &mut ConfigEditor) -> Action {
        for ((key, value), original) in self.switches().into_iter().zip(self.original_switches) {
            if value != original {
                ed.set_bool("text", key, value);
            }
        }

        if self.lard_model_path != self.original_model_path {
            if let Some(path) = &self.lard_model_path {
                ed.set_string("text", "lard_model_path", path);
            } else {
                ed.unset("text", "lard_model_path");
            }
        }

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
            if ed.get_string("text.replacements", k).as_ref() != Some(v) {
                ed.set_string("text.replacements", k, v);
            }
        }

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.original_switches = self.switches().map(|(_, value)| value);
                self.original_model_path = self.lard_model_path.clone();
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
            let cursor = self
                .cursor
                .min(total_rows(&fresh.replacements).saturating_sub(1));
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

    fn switches(&self) -> [(&'static str, bool); 8] {
        [
            ("spoken_punctuation", self.spoken_punctuation),
            ("filter_filler_words", self.filter_filler_words),
            ("collapse_restarts", self.collapse_restarts),
            ("repair_sentence_breaks", self.repair_sentence_breaks),
            ("format_numbers", self.format_numbers),
            ("lard_disfluency", self.lard_disfluency),
            ("punctuation_casing", self.punctuation_casing),
            ("smart_auto_submit", self.smart_auto_submit),
        ]
    }

    fn cycle(&mut self) {
        match self.cursor {
            0 => self.spoken_punctuation = !self.spoken_punctuation,
            1 => self.filter_filler_words = !self.filter_filler_words,
            2 => self.collapse_restarts = !self.collapse_restarts,
            3 => self.repair_sentence_breaks = !self.repair_sentence_breaks,
            4 => self.format_numbers = !self.format_numbers,
            5 => self.lard_disfluency = !self.lard_disfluency,
            6 if cfg!(feature = "punctuation-casing") => {
                self.punctuation_casing = !self.punctuation_casing
            }
            6 => {
                self.feedback = Some((FeedbackLevel::Err, "Learned punctuation/casing is not compiled into this build; requires punctuation-casing and its separate model.".into()));
                return;
            }
            7 => self.smart_auto_submit = !self.smart_auto_submit,
            _ => return,
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }

    fn start_edit(&mut self) {
        if self.cursor == toggle_count() {
            self.model_edit = Some(TextInput::new(
                self.lard_model_path.clone().unwrap_or_default(),
            ));
            return;
        }
        let target = if self.cursor == add_row_index(&self.replacements) {
            EditTarget::New
        } else if self.cursor >= replacement_start() {
            EditTarget::Existing(self.cursor - replacement_start())
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
        if self.cursor >= replacement_start() && self.cursor < add_row_index(&self.replacements) {
            let idx = self.cursor - replacement_start();
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
            let block = Block::default().borders(Borders::ALL).title("Text Cleanup");
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

    render_state(f, area, state);
}

fn render_state(f: &mut Frame, area: Rect, state: &TextState) {
    let editing_idx = state.editing.as_ref().and_then(|e| match e.target {
        EditTarget::Existing(i) => Some(i),
        EditTarget::New => None,
    });
    let editing_new = matches!(
        state.editing.as_ref().map(|e| e.target),
        Some(EditTarget::New)
    );

    let mut rows: Vec<FormRowSpec> = Vec::new();

    rows.push(FormRowSpec::new(false, "Clean up text", "deterministic"));
    let labels = [
        "Spoken punctuation",
        "Remove filler words",
        "Collapse restarts",
        "Repair sentence breaks",
        "Format numbers",
        "LARD disfluency cleanup",
        "Learned punctuation/casing",
        "Smart auto-submit",
    ];
    for (i, ((_, value), label)) in state.switches().into_iter().zip(labels).enumerate() {
        if i == 5 {
            rows.push(FormRowSpec::new(
                false,
                "Optional model cleanup",
                "independent opt-in",
            ));
        }
        if i == 6 {
            rows.push(FormRowSpec::new(false, "Other text options", ""));
        }
        let value = if i == 6 && !cfg!(feature = "punctuation-casing") {
            format!("{} (not compiled)", yesno(value))
        } else {
            yesno(value)
        };
        rows.push(
            FormRowSpec::new(state.cursor == i, label, value)
                .dimmed(i == 6 && !cfg!(feature = "punctuation-casing")),
        );
    }
    let model_value = match &state.model_edit {
        Some(input) => input.caret_string(),
        None => state
            .lard_model_path
            .clone()
            .unwrap_or_else(|| "default path (not checked)".into()),
    };
    rows.push(FormRowSpec::new(
        state.cursor == toggle_count(),
        "LARD model path",
        model_value,
    ));

    for (i, (k, v)) in state.replacements.iter().enumerate() {
        let row_idx = replacement_start() + i;
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
    rows.push(FormRowSpec::new(
        state.cursor == add_idx,
        add_label,
        add_value,
    ));

    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|(lvl, msg)| (*lvl, msg.as_str()));

    common::render_form_with_guidance(
        f,
        area,
        "Text Cleanup",
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
        EditPhase::Value => format!("\"{}\" → {}", edit.key_buffer, edit.input.caret_string()),
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
    let on_replacement_row = state.cursor >= replacement_start() && state.cursor < total - 1;
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

    if state.cursor == 7 {
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

    if (1..=4).contains(&state.cursor) {
        let descriptions = [
            ("Remove filler words", "Remove configured fillers such as uh and um. The filler_words list remains configurable in the config file."),
            ("Collapse restarts", "Collapse repeated phrases, stutters and explicit retractions. Off by default: false positives can delete intended words."),
            ("Repair sentence breaks", "Join fragments split at a pause and repair structural questions using deterministic rules. Off by default."),
            ("Format numbers", "Format dictated quantities, money and times using deterministic rules. Off by default."),
        ];
        let (title, description) = descriptions[state.cursor - 1];
        return vec![heading(title), Line::from(""), Line::from(description), Line::from(""), Line::from("Clean up text stages are independent. They do not enable LARD or download any model.")];
    }
    if state.cursor == 5 || state.cursor == toggle_count() {
        return vec![
            heading("LARD disfluency cleanup (opt-in)"),
            Line::from("English only: set the engine language explicitly to en. Independent of deterministic cleanup."),
            Line::from(""),
            Line::from("Requires a separately installed local VXDF model. Missing/invalid models skip LARD; no automatic download."),
            Line::from("Setup: voxtype setup lard --help"),
            Line::from("Model license: CC BY-SA 4.0 (separate from MIT code)"),
            Line::from("https://creativecommons.org/licenses/by-sa/4.0/"),
            Line::from("Enter/i edits the model path; blank restores the default path. Esc cancels. Setting a path does not enable LARD or prove the model is installed."),
        ];
    }
    if state.cursor == 6 {
        return vec![heading("Learned punctuation/casing"), Line::from("Separate model-assisted feature, not deterministic cleanup. Requires a punctuation-casing build and its separately set-up model."), Line::from("Only applies to plain CTC-output engines. Engines already providing punctuation/casing are left unchanged."), Line::from(if cfg!(feature = "punctuation-casing") { "Compiled in this client; model availability is not checked here." } else { "Not compiled in this client. Existing config choice is preserved; this switch cannot be changed here." })];
    }

    if on_replacement_row {
        let idx = state.cursor - replacement_start();
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

    handle_state_key(state, key)
}

fn handle_state_key(state: &mut TextState, key: KeyEvent) -> Action {
    if let Some(input) = state.model_edit.as_mut() {
        match input.handle_key(key) {
            TextInputResult::Continue => {}
            TextInputResult::Cancel => state.model_edit = None,
            TextInputResult::Commit => {
                let path = input.buffer().to_string();
                state.lard_model_path = if path.is_empty() { None } else { Some(path) };
                state.model_edit = None;
                state.dirty_since_load = true;
                state.feedback = None;
            }
        }
        return Action::None;
    }
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
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Char('h')
        | KeyCode::Char('l')
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_persist_independently_without_rewriting_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let source = "# keep top\nengine = 'cohere'\n[cohere]\nmodel = 'legacy-onnx'\n[text]\n# keep fillers\nfilter_filler_words = false # user choice\ncollapse_restarts = false\nrepair_sentence_breaks = false\nformat_numbers = false\nlard_disfluency = false\npunctuation_casing = false\nspoken_punctuation = false\nsmart_auto_submit = false\nlard_model_path = '/missing/model.vxdf'\n[text.replacements]\n'vox type' = 'VoxType'\n";
        let keys = [
            "spoken_punctuation",
            "filter_filler_words",
            "collapse_restarts",
            "repair_sentence_breaks",
            "format_numbers",
            "lard_disfluency",
            "punctuation_casing",
            "smart_auto_submit",
        ];
        for (row, changed) in keys.iter().enumerate() {
            std::fs::write(&path, source).unwrap();
            let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
            let mut state = TextState::from_editor(&ed).unwrap();
            state.cursor = row;
            state.cycle();
            state.save_with_editor(&mut ed);
            let saved = std::fs::read_to_string(&path).unwrap();
            let doc: toml::Value = saved.parse().unwrap();
            for key in keys {
                let expected =
                    key == *changed && (row != 6 || cfg!(feature = "punctuation-casing"));
                assert_eq!(
                    doc["text"][key].as_bool(),
                    Some(expected),
                    "row {row}: {key}"
                );
            }
            assert!(saved.contains("# keep top"));
            assert!(saved.contains("# keep fillers"));
            assert_eq!(doc["cohere"]["model"].as_str(), Some("legacy-onnx"));
            assert_eq!(
                doc["text"]["lard_model_path"].as_str(),
                Some("/missing/model.vxdf")
            );
        }
    }

    #[test]
    fn model_path_keyboard_edit_is_not_a_replacement_or_activation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let source =
            "[text]\nlard_disfluency = false\n[text.replacements]\n'vox type' = 'VoxType'\n";
        std::fs::write(&path, source).unwrap();
        let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        let mut state = TextState::from_editor(&ed).unwrap();
        state.cursor = 8;
        handle_state_key(&mut state, KeyCode::Enter.into());
        handle_state_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('u'), crossterm::event::KeyModifiers::CONTROL),
        );
        for ch in "/missing/new.vxdf".chars() {
            handle_state_key(&mut state, KeyCode::Char(ch).into());
        }
        handle_state_key(&mut state, KeyCode::Enter.into());
        state.save_with_editor(&mut ed);
        let saved: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(
            saved["text"]["lard_model_path"].as_str(),
            Some("/missing/new.vxdf")
        );
        assert_eq!(saved["text"]["lard_disfluency"].as_bool(), Some(false));
        assert_eq!(
            saved["text"]["replacements"]["vox type"].as_str(),
            Some("VoxType")
        );
    }

    #[test]
    fn unchanged_save_preserves_omissions_and_file_bytes() {
        for source in ["# absent text\nengine = 'cohere'\n", "# old config\n[text]\nsmart_auto_submit = false # submit\n[text.replacements]\n'vox type' = 'VoxType' # keep spelling\n"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, source).unwrap();
            let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
            let mut state = TextState::from_editor(&ed).unwrap();
            let expected: crate::config::Config = toml::from_str(source).unwrap();
            assert_eq!(state.filter_filler_words, expected.text.filter_filler_words);
            assert!(!state.collapse_restarts);
            assert!(!state.lard_disfluency);
            state.save_with_editor(&mut ed);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
            state.cursor = 7;
            state.cycle();
            state.save_with_editor(&mut ed);
            let saved = std::fs::read_to_string(&path).unwrap();
            let doc: toml::Value = saved.parse().unwrap();
            assert_eq!(doc["text"].as_table().unwrap().len(), if source.contains("replacements") { 2 } else { 1 });
            assert!(!saved.contains("lard_disfluency"));
            assert!(!saved.contains("filter_filler_words"));
        }
    }

    #[test]
    fn changed_values_preserve_inline_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[text]\nfilter_filler_words = false # keep user explanation\n",
        )
        .unwrap();
        let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        let mut state = TextState::from_editor(&ed).unwrap();
        state.cursor = 1;
        state.cycle();
        state.save_with_editor(&mut ed);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("true # keep user explanation"));
    }

    #[test]
    fn cleanup_title_and_opt_in_are_visible() {
        assert_eq!(super::super::section::Section::Text.label(), "Text Cleanup");
        for spec in crate::config::schema::scalar_keys().filter(|s| s.key.starts_with("text.")) {
            assert_eq!(spec.section, "Text Cleanup");
        }
        let dir = tempfile::tempdir().unwrap();
        let ed = ConfigEditor::load_from_path(dir.path().join("config.toml")).unwrap();
        let state = TextState::from_editor(&ed).unwrap();
        let backend = ratatui::backend::TestBackend::new(140, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_state(f, f.area(), &state))
            .unwrap();
        let screen = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(screen.contains("Text Cleanup"));
        assert!(screen.contains("LARD disfluency cleanup"));
        assert!(screen.contains("Clean up text"));
    }

    #[test]
    fn model_edit_swallows_global_shortcuts() {
        let dir = tempfile::tempdir().unwrap();
        let ed = ConfigEditor::load_from_path(dir.path().join("config.toml")).unwrap();
        let mut state = TextState::from_editor(&ed).unwrap();
        state.cursor = 8;
        state.start_edit();
        let mut app = App::new(false);
        app.current_section = super::super::section::Section::Text;
        app.text = Some(state);
        assert!(
            app.is_editing(),
            "q, Tab and Esc must reach the path input, not global navigation"
        );
    }

    #[test]
    fn navigation_scrolls_to_replacements_and_add_row() {
        let dir = tempfile::tempdir().unwrap();
        let ed = ConfigEditor::load_from_path(dir.path().join("config.toml")).unwrap();
        let mut state = TextState::from_editor(&ed).unwrap();
        state.replacements = (0..30)
            .map(|i| (format!("word{i}"), format!("Value{i}")))
            .collect();
        // Up from the first switch wraps to Add. Down wraps back.
        handle_state_key(&mut state, KeyCode::Up.into());
        assert_eq!(state.cursor, add_row_index(&state.replacements));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 18)).unwrap();
        terminal
            .draw(|f| render_state(f, f.area(), &state))
            .unwrap();
        let screen = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(
            screen.contains("+ Add new replacement"),
            "selected Add row scrolled out: {screen}"
        );
        handle_state_key(&mut state, KeyCode::Enter.into());
        for c in "new key".chars() {
            handle_state_key(&mut state, KeyCode::Char(c).into());
        }
        handle_state_key(&mut state, KeyCode::Enter.into());
        for c in "new value".chars() {
            handle_state_key(&mut state, KeyCode::Char(c).into());
        }
        handle_state_key(&mut state, KeyCode::Enter.into());
        assert!(state
            .replacements
            .contains(&("new key".into(), "new value".into())));
        state.cursor = replacement_start();
        let before = state.replacements.len();
        handle_state_key(&mut state, KeyCode::Delete.into());
        assert_eq!(state.replacements.len(), before - 1);
        state.cursor = add_row_index(&state.replacements);
        handle_state_key(&mut state, KeyCode::Down.into());
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn cleanup_controls_precede_model_path_and_replacements() {
        assert_eq!(
            toggle_count(),
            8,
            "five deterministic stages, LARD, learned casing, submit"
        );
        assert_eq!(
            add_row_index(&[]),
            9,
            "model path is an editable row, not a toggle"
        );
        assert_eq!(total_rows(&[("vox".into(), "Vox".into())]), 11);
    }
}
