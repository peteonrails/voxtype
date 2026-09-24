//! Text-processing settings: spoken punctuation, an inline editor for the
//! [text.replacements] map, and auto-submit configuration (the smart
//! "submit" keyword plus per-app [output.auto_submit_apps] rules).

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

/// Auto-submit app rule values. True = press Enter after dictation in the
/// matched app, false = never submit there (overrides the global setting).
type AppRule = (String, bool);

#[derive(Debug, Clone)]
pub struct TextState {
    pub spoken_punctuation: bool,
    pub smart_auto_submit: bool,
    /// Sorted by key for stable display. The user can edit keys/values via
    /// the inline editor below.
    pub apps: Vec<AppRule>,
    /// Set of original app keys at load time, so save() can detect deletions.
    pub original_app_keys: Vec<String>,
    /// Sorted by key for stable display. The user can edit keys/values via
    /// the inline editor below.
    pub replacements: Vec<(String, String)>,
    /// Set of original keys at load time, so save() can detect deletions.
    pub original_keys: Vec<String>,
    pub cursor: usize,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
    pub editing: Option<ListEdit>,
}

/// Which list the inline editor is working on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditList {
    /// [output.auto_submit_apps] entries.
    Apps,
    /// [text.replacements] entries.
    Replacements,
}

/// Editing state for either list. Users edit the key first, then the
/// value; commit on the value commits the whole pair. The value phase is
/// a free-text input for replacements and a yes/no toggle for app rules.
#[derive(Debug, Clone)]
pub struct ListEdit {
    pub list: EditList,
    pub target: EditTarget,
    pub phase: EditPhase,
    pub key_buffer: String,
    pub input: TextInput,
    /// Value for the Apps value phase (toggle with left/right).
    pub submit: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum EditTarget {
    /// Editing the entry at this index in the target list.
    Existing(usize),
    /// Adding a new entry at the end of the list.
    New,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditPhase {
    Key,
    Value,
}

/// Cursor vocabulary. The cursor indexes *selectable* rows only; headings
/// are display separators that navigation skips over. Positions 0 and 1
/// are the toggles, then app-rule rows, then the "+ Add app rule" row,
/// then replacement rows, then the "+ Add replacement" row.
fn toggle_count() -> usize {
    2
}

fn app_row_index(i: usize) -> usize {
    toggle_count() + i
}

fn add_app_row(apps: &[AppRule]) -> usize {
    app_row_index(apps.len())
}

fn first_replacement_row(apps: &[AppRule]) -> usize {
    add_app_row(apps) + 1
}

fn replacement_row_index(apps: &[AppRule], i: usize) -> usize {
    first_replacement_row(apps) + i
}

fn add_replacement_row(apps: &[AppRule], replacements: &[(String, String)]) -> usize {
    replacement_row_index(apps, replacements.len())
}

fn total_rows(apps: &[AppRule], replacements: &[(String, String)]) -> usize {
    add_replacement_row(apps, replacements) + 1
}

impl TextState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        let apps = read_apps(&ed);
        let replacements = read_replacements(&ed);
        let original_app_keys: Vec<String> = apps.iter().map(|(k, _)| k.clone()).collect();
        let original_keys: Vec<String> = replacements.iter().map(|(k, _)| k.clone()).collect();
        Ok(Self {
            spoken_punctuation: ed.get_bool("text", "spoken_punctuation").unwrap_or(false),
            smart_auto_submit: ed.get_bool("text", "smart_auto_submit").unwrap_or(false),
            apps,
            original_app_keys,
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

        // Both maps: write every current entry, then unset any original keys
        // that are no longer in the list (deletions).
        save_table(
            &mut ed,
            "text.replacements",
            &self.replacements,
            &self.original_keys,
            |ed, k, v| ed.set_string("text.replacements", k, v),
        );
        save_table(
            &mut ed,
            "output.auto_submit_apps",
            &self.apps,
            &self.original_app_keys,
            |ed, k, v| ed.set_bool("output.auto_submit_apps", k, *v),
        );

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.original_keys = self.replacements.iter().map(|(k, _)| k.clone()).collect();
                self.original_app_keys = self.apps.iter().map(|(k, _)| k.clone()).collect();
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
                .min(total_rows(&fresh.apps, &fresh.replacements).saturating_sub(1));
            *self = fresh;
            self.cursor = cursor;
            self.feedback = Some((FeedbackLevel::Ok, "Reverted unsaved changes".to_string()));
        }
    }

    fn move_field(&mut self, delta: i32) {
        let len = total_rows(&self.apps, &self.replacements) as i32;
        let new = (self.cursor as i32 + delta).rem_euclid(len);
        self.cursor = new as usize;
    }

    fn cycle(&mut self) {
        match self.cursor {
            0 => self.spoken_punctuation = !self.spoken_punctuation,
            1 => self.smart_auto_submit = !self.smart_auto_submit,
            _ => {} // list rows don't cycle
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }

    fn start_edit(&mut self) {
        let add_app = add_app_row(&self.apps);
        let first_repl = first_replacement_row(&self.apps);

        let (list, target, initial_key) = if self.cursor < toggle_count() {
            return; // toggles flip via cycle(), they aren't text-editable
        } else if self.cursor < add_app {
            let i = self.cursor - toggle_count();
            let (k, _) = self.apps[i].clone();
            (EditList::Apps, EditTarget::Existing(i), k)
        } else if self.cursor == add_app {
            (EditList::Apps, EditTarget::New, String::new())
        } else if self.cursor < add_replacement_row(&self.apps, &self.replacements) {
            let i = self.cursor - first_repl;
            let (k, _) = self.replacements[i].clone();
            (EditList::Replacements, EditTarget::Existing(i), k)
        } else {
            (EditList::Replacements, EditTarget::New, String::new())
        };

        // `submit` is populated when the Key phase commits (see commit_edit).
        self.editing = Some(ListEdit {
            list,
            target,
            phase: EditPhase::Key,
            key_buffer: String::new(),
            input: TextInput::new(initial_key),
            submit: false,
        });
    }

    fn delete_at_cursor(&mut self) {
        let add_app = add_app_row(&self.apps);
        let add_repl = add_replacement_row(&self.apps, &self.replacements);
        if self.cursor >= toggle_count() && self.cursor < add_app {
            let idx = self.cursor - toggle_count();
            self.apps.remove(idx);
            self.dirty_since_load = true;
            self.feedback = None;
        } else if self.cursor >= first_replacement_row(&self.apps) && self.cursor < add_repl {
            let idx = self.cursor - first_replacement_row(&self.apps);
            self.replacements.remove(idx);
            self.dirty_since_load = true;
            self.feedback = None;
        } else {
            return;
        }
        // Clamp cursor in case we removed the last entry of a list.
        let max = total_rows(&self.apps, &self.replacements).saturating_sub(1);
        if self.cursor > max {
            self.cursor = max;
        }
    }

    /// Called when the inline TextInput commits. Advances the edit phase or
    /// finalizes the entry.
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
                let (initial_value, initial_submit) = match (edit.list, edit.target) {
                    (EditList::Apps, EditTarget::Existing(i)) => (String::new(), self.apps[i].1),
                    (EditList::Apps, EditTarget::New) => (String::new(), true),
                    (EditList::Replacements, EditTarget::Existing(i)) => {
                        (self.replacements[i].1.clone(), true)
                    }
                    (EditList::Replacements, EditTarget::New) => (String::new(), true),
                };
                self.editing = Some(ListEdit {
                    list: edit.list,
                    target: edit.target,
                    phase: EditPhase::Value,
                    key_buffer: trimmed,
                    input: TextInput::new(initial_value),
                    submit: initial_submit,
                });
            }
            EditPhase::Value => {
                let key = edit.key_buffer;
                match edit.list {
                    EditList::Apps => {
                        // Bool toggled directly in handle_edit_key; not text.
                        match edit.target {
                            EditTarget::Existing(i) => self.apps[i] = (key, edit.submit),
                            EditTarget::New => self.apps.push((key, edit.submit)),
                        }
                        self.apps.sort_by(|a, b| a.0.cmp(&b.0));
                        self.dirty_since_load = true;
                        self.feedback = None;
                    }
                    EditList::Replacements => {
                        let value = buf;
                        if value.is_empty() {
                            // Empty value is allowed but doesn't make much
                            // sense; treat as a cancel for the new-entry flow.
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
    }
}

/// Write every current entry of a key/value table, then unset any original
/// key that is no longer present (deletions).
fn save_table<V>(
    ed: &mut ConfigEditor,
    table: &str,
    entries: &[(String, V)],
    original_keys: &[String],
    set: impl Fn(&mut ConfigEditor, &str, &V),
) {
    let current: std::collections::HashSet<&String> = entries.iter().map(|(k, _)| k).collect();
    for original in original_keys {
        if !current.contains(original) {
            ed.unset(table, original);
        }
    }
    for (k, v) in entries {
        set(ed, k, v);
    }
}

fn read_apps(ed: &ConfigEditor) -> Vec<AppRule> {
    ed.raw_table("output.auto_submit_apps")
        .map(apps_from_table)
        .unwrap_or_default()
}

fn apps_from_table(table: &toml_edit::Table) -> Vec<AppRule> {
    let mut out: Vec<AppRule> = table
        .iter()
        .filter_map(|(k, v)| {
            v.as_value()
                .and_then(|v| v.as_bool())
                .map(|b| (k.to_string(), b))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn read_replacements(ed: &ConfigEditor) -> Vec<(String, String)> {
    // Walk the table directly via toml_edit, since the ConfigEditor accessor
    // only returns single keyed values.
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(t) = ed.raw_table("text.replacements") {
        for (k, v) in t.iter() {
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

    let editing_row = state.editing.as_ref().map(|e| match (e.list, e.target) {
        (_, EditTarget::New) => add_target_row(state, e.list),
        (list, EditTarget::Existing(i)) => match list {
            EditList::Apps => app_row_index(i),
            EditList::Replacements => replacement_row_index(&state.apps, i),
        },
    });

    let mut rows: Vec<FormRowSpec> = Vec::new();

    rows.push(FormRowSpec::new(
        state.cursor == 0,
        "Spoken punctuation conversion",
        yesno(state.spoken_punctuation),
    ));

    rows.push(FormRowSpec::heading("Auto-submit"));
    rows.push(FormRowSpec::new(
        state.cursor == 1,
        "Smart auto-submit on \"submit\"",
        yesno(state.smart_auto_submit),
    ));

    for (i, (k, v)) in state.apps.iter().enumerate() {
        let row_idx = app_row_index(i);
        let label = format!("\"{}\"", k);
        let value = if editing_row == Some(row_idx) {
            list_edit_value(state)
        } else if *v {
            "→ submit".to_string()
        } else {
            "→ don't submit".to_string()
        };
        rows.push(FormRowSpec::new(state.cursor == row_idx, label, value));
    }

    let add_app = add_app_row(&state.apps);
    let add_label = if editing_row == Some(add_app) {
        "(new rule)".to_string()
    } else {
        "+ Add new app rule".to_string()
    };
    let add_value = if editing_row == Some(add_app) {
        list_edit_value(state)
    } else {
        "press Enter".to_string()
    };
    rows.push(FormRowSpec::new(
        state.cursor == add_app,
        add_label,
        add_value,
    ));

    rows.push(FormRowSpec::heading("Word replacements"));
    for (i, (k, v)) in state.replacements.iter().enumerate() {
        let row_idx = replacement_row_index(&state.apps, i);
        let label = format!("\"{}\"", k);
        let value = if editing_row == Some(row_idx) {
            list_edit_value(state)
        } else {
            format!("→ \"{}\"", v)
        };
        rows.push(FormRowSpec::new(state.cursor == row_idx, label, value));
    }

    let add_repl = add_replacement_row(&state.apps, &state.replacements);
    let add_label = if editing_row == Some(add_repl) {
        "(new entry)".to_string()
    } else {
        "+ Add new replacement".to_string()
    };
    let add_value = if editing_row == Some(add_repl) {
        list_edit_value(state)
    } else {
        "press Enter".to_string()
    };
    rows.push(FormRowSpec::new(
        state.cursor == add_repl,
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
        "Text",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance(state),
    );
}

/// Cursor position of the row being edited (for a New target this is the
/// add row of the edited list).
fn add_target_row(state: &TextState, list: EditList) -> usize {
    match list {
        EditList::Apps => add_app_row(&state.apps),
        EditList::Replacements => add_replacement_row(&state.apps, &state.replacements),
    }
}

fn list_edit_value(state: &TextState) -> String {
    let Some(edit) = state.editing.as_ref() else {
        return String::new();
    };
    match (edit.list, edit.phase) {
        (_, EditPhase::Key) => format!("editing key: {}", edit.input.caret_string()),
        (EditList::Apps, EditPhase::Value) => format!(
            "{} → {} (◂▸ to toggle, Enter)",
            edit.key_buffer,
            yesno(edit.submit)
        ),
        (EditList::Replacements, EditPhase::Value) => {
            format!("\"{}\" → {}", edit.key_buffer, edit.input.caret_string())
        }
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
    if let Some(edit) = state.editing.as_ref() {
        let header = match (edit.list, edit.phase) {
            (EditList::Apps, EditPhase::Key) => {
                "✎ Editing window pattern — Enter for action, Esc to cancel"
            }
            (EditList::Apps, EditPhase::Value) => {
                "✎ Editing rule — ◂▸ to toggle, Enter to commit, Esc to cancel"
            }
            (_, EditPhase::Key) => "✎ Editing key — Enter for value, Esc to cancel",
            (_, EditPhase::Value) => "✎ Editing value — Enter to commit, Esc to cancel",
        };
        let mut lines = vec![
            Line::from(Span::styled(
                header,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        if edit.list == EditList::Apps {
            lines.extend(vec![
                Line::from(
                    "Patterns are case-insensitive substrings matched against \
                     the focused window's class or title.",
                ),
                Line::from(""),
                Line::from(Span::styled(
                    "Examples:",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from("  \"slack\"   →  submit"),
                Line::from("  \"chatgpt\" →  submit"),
                Line::from("  \"nvim\"    →  don't submit"),
            ]);
        } else {
            lines.extend(vec![
                Line::from(
                    "Replacements run as a case-insensitive substring match \
                     across the transcript before output. The dictated word \
                     goes on the left, the replacement on the right.",
                ),
                Line::from(""),
                Line::from(Span::styled(
                    "Examples:",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from("  \"vox type\"  →  \"voxtype\""),
                Line::from("  \"a i\"       →  \"AI\""),
                Line::from("  \"slack\"     →  \"Slack\""),
            ]);
        }
        return lines;
    }

    let add_app = add_app_row(&state.apps);
    let add_repl = add_replacement_row(&state.apps, &state.replacements);

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

    if state.cursor < add_app {
        let (k, v) = &state.apps[state.cursor - toggle_count()];
        return vec![
            heading("Auto-submit in apps"),
            Line::from(""),
            Line::from(format!(
                "  \"{}\"  →  {}",
                k,
                if *v { "submit" } else { "don't submit" }
            )),
            Line::from(""),
            Line::from(
                "Press Enter to edit (pattern first, then ◂▸ to choose the \
                 action). Press d to delete this rule.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Rules override the global auto_submit setting while the \
                 matched window is focused. Longest matching pattern wins.",
                Style::default().fg(Color::Gray),
            )),
        ];
    }

    if state.cursor == add_app {
        return vec![
            heading("Add an app rule"),
            Line::from(""),
            Line::from(
                "Press Enter to start a new rule. You'll be prompted for the \
                 window pattern first, then choose submit or don't submit.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Examples:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("  \"slack\"   →  submit (chat apps)"),
            Line::from("  \"obsidian\" →  don't submit (markdown)"),
            Line::from("  \"kitty\"   →  don't submit (terminals)"),
        ];
    }

    if state.cursor < add_repl {
        let idx = state.cursor - first_replacement_row(&state.apps);
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

    if state.cursor == add_repl {
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
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Char('h')
        | KeyCode::Char('l')
        | KeyCode::Char(' ') => {
            state.cycle();
            Action::None
        }
        KeyCode::Enter | KeyCode::Char('i') => {
            // Enter on toggles flips them; on list rows starts edit.
            if state.cursor < toggle_count() {
                state.cycle();
            } else {
                state.start_edit();
            }
            Action::None
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            state.delete_at_cursor();
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

    // The app-rule value phase is a bool selector, not a text input.
    if editing.list == EditList::Apps && editing.phase == EditPhase::Value {
        match key.code {
            KeyCode::Enter => state.commit_edit(),
            KeyCode::Esc => state.editing = None,
            KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') => {
                editing.submit = !editing.submit;
            }
            _ => {}
        }
        return Action::None;
    }

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

    fn sample() -> TextState {
        TextState {
            spoken_punctuation: false,
            smart_auto_submit: false,
            apps: vec![("slack".to_string(), true)],
            original_app_keys: vec![],
            replacements: vec![("vox type".to_string(), "voxtype".to_string())],
            original_keys: vec![],
            cursor: 0,
            feedback: None,
            dirty_since_load: false,
            editing: None,
        }
    }

    #[test]
    fn test_row_model_counts_both_lists() {
        let s = sample();
        // 2 toggles + 1 app + 1 add-app + 1 replacement + 1 add-replacement
        assert_eq!(total_rows(&s.apps, &s.replacements), 6);
        assert_eq!(app_row_index(0), 2);
        assert_eq!(add_app_row(&s.apps), 3);
        assert_eq!(first_replacement_row(&s.apps), 4);
        assert_eq!(replacement_row_index(&s.apps, 0), 4);
        assert_eq!(add_replacement_row(&s.apps, &s.replacements), 5);
    }

    #[test]
    fn test_row_model_empty_lists() {
        let s = TextState {
            apps: vec![],
            replacements: vec![],
            ..sample()
        };
        // 2 toggles + add-app + add-replacement
        assert_eq!(total_rows(&s.apps, &s.replacements), 4);
        assert_eq!(first_replacement_row(&s.apps), 3);
    }

    #[test]
    fn test_apps_from_table_parses_bools_and_sorts() {
        let toml = "[output.auto_submit_apps]\nslack = true\nkitty = false\nbad = \"x\"\n";
        let doc: toml_edit::DocumentMut = toml.parse().unwrap();
        let table = doc["output"]["auto_submit_apps"].as_table().unwrap();
        assert_eq!(
            apps_from_table(table),
            vec![("kitty".to_string(), false), ("slack".to_string(), true)]
        );
    }
}
