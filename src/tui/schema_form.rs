//! A form built from the config schema ([`crate::config::schema`]).
//!
//! Every [`KeySpec`] the section passes in becomes a row, typed by its
//! [`KeyType`]: a bool toggles, an enum cycles through its choices, a number
//! steps within its bounds, and anything can be typed with Enter. Labels and
//! help come from the schema, so this page and the Quickshell Settings panel
//! (which renders the same schema) agree. Rows are grouped by their table,
//! groups ordered by where their first key appears in the schema, keys in
//! schema order inside a group, which is the order Quickshell lists them in.
//!
//! Edits are kept as pending changes and written in one go by
//! [`SchemaForm::save_with_editor`] through [`ConfigEditor`], which keeps
//! comments and validates the whole file before replacing it, so a rejected
//! edit leaves the file as it was.
//!
//! Secrets (values set through a command and never kept in config.toml, such
//! as the calendar link) are shown only as set or not set.

use std::collections::{BTreeMap, HashMap};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    Frame,
};
use serde_json::Value as Json;

use super::common::{self, FeedbackLevel, FormRowSpec, TextInput, TextInputResult};
use super::config_editor::ConfigEditor;
use crate::config::schema::{self, Found, KeySpec, KeyType, TypedValue};
use crate::config::Config;

/// A value set through a command rather than in config.toml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRow {
    pub key: String,
    pub label: String,
    pub description: String,
    /// How to set or clear it, shown in the help panel.
    pub help: String,
    pub set: bool,
}

impl SecretRow {
    /// From an entry of `voxtype config schema --json`'s `secrets` list.
    /// Only the key, label, description, help and whether it is set are
    /// kept: the value, and anything derived from it, never are.
    pub fn from_schema_entry(entry: &Json) -> Option<Self> {
        Some(Self {
            key: entry["key"].as_str()?.to_string(),
            label: entry["label"].as_str().unwrap_or_default().to_string(),
            description: entry["description"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            help: entry["help"].as_str().unwrap_or_default().to_string(),
            set: entry["set"].as_bool().unwrap_or(false),
        })
    }

    fn table(&self) -> &str {
        self.key.rsplit_once('.').map(|(t, _)| t).unwrap_or("")
    }
}

/// One line of the form.
#[derive(Debug, Clone)]
pub enum Row {
    Heading(String),
    Key(&'static KeySpec),
    Secret(SecretRow),
}

impl Row {
    fn focusable(&self) -> bool {
        !matches!(self, Row::Heading(_))
    }
}

/// A change not yet written.
#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    Set(TypedValue),
    /// Remove the key so its default applies.
    Unset,
}

/// What the section around the form customizes.
pub struct FormOptions {
    pub title: &'static str,
    /// Heading for the keys in `table`.
    pub group_title: fn(table: &str) -> String,
    /// A bool key that, while false, dims every other row.
    pub master: Option<&'static str>,
    /// Extra help paragraphs for a key, after its schema description.
    pub extra_help: fn(key: &str) -> Vec<&'static str>,
}

/// A schema-driven form.
pub struct SchemaForm {
    options: FormOptions,
    rows: Vec<Row>,
    /// Index into `rows`; always on a focusable row when there is one.
    cursor: usize,
    /// The text being typed into the focused row.
    pub editing: Option<TextInput>,
    pending: BTreeMap<&'static str, Pending>,
    /// Literal values in the file (`null` when absent).
    file: HashMap<&'static str, Json>,
    /// What each key resolves to with nothing in the file.
    defaults: HashMap<&'static str, Json>,
    pub feedback: Option<(FeedbackLevel, String)>,
}

/// Group `keys` (in schema order) and `secrets` into rows.
///
/// A secret goes before the first key sharing its table, else after the
/// last key, as the Quickshell panel places it. Groups follow the order
/// their first entry appears in.
fn build_rows(
    keys: &[&'static KeySpec],
    secrets: Vec<SecretRow>,
    group_title: fn(&str) -> String,
) -> Vec<Row> {
    let mut entries: Vec<Row> = keys.iter().map(|k| Row::Key(k)).collect();
    for secret in secrets {
        let at = entries
            .iter()
            .position(|r| matches!(r, Row::Key(k) if k.table == secret.table()))
            .unwrap_or(entries.len());
        entries.insert(at, Row::Secret(secret));
    }
    let table_of = |r: &Row| match r {
        Row::Key(k) => k.table.to_string(),
        Row::Secret(s) => s.table().to_string(),
        Row::Heading(_) => String::new(),
    };
    let mut order: Vec<String> = Vec::new();
    for r in &entries {
        let t = table_of(r);
        if !order.contains(&t) {
            order.push(t);
        }
    }
    let mut rows = Vec::new();
    for table in order {
        rows.push(Row::Heading(group_title(&table)));
        rows.extend(entries.iter().filter(|r| table_of(r) == table).cloned());
    }
    rows
}

/// A value as a short string for the form.
fn display(value: &Json) -> String {
    match value {
        Json::Bool(b) => (if *b { "yes" } else { "no" }).to_string(),
        Json::Null => "unset".to_string(),
        Json::String(s) if s.is_empty() => "empty".to_string(),
        Json::String(s) => s.clone(),
        // An f32 default widened to f64 prints as 0.009999999776482582.
        Json::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap_or_default();
            let text = format!("{f:.4}");
            let trimmed = text.trim_end_matches('0');
            if trimmed.ends_with('.') {
                format!("{trimmed}0")
            } else {
                trimmed.to_string()
            }
        }
        other => other.to_string(),
    }
}

/// Keys whose value is a credential: shown as set or not, never typed back.
fn sensitive(spec: &KeySpec) -> bool {
    spec.field.ends_with("api_key") || spec.field.ends_with("token")
}

/// Step for ←→ on a number: about a hundredth of its range, rounded to a
/// power of ten, so wide ranges are not stepped one by one.
fn int_step(min: i64, max: i64) -> i64 {
    let range = (max - min).max(1) as f64;
    if range <= 120.0 {
        return 1;
    }
    10f64.powi((range / 100.0).log10().floor() as i32) as i64
}

fn float_step(min: f64, max: f64) -> f64 {
    match max - min {
        r if r <= 1.0 => 0.05,
        r if r <= 10.0 => 0.1,
        _ => 1.0,
    }
}

impl SchemaForm {
    /// A form over `keys` (in schema order), with values read from `editor`.
    pub fn new(
        options: FormOptions,
        keys: Vec<&'static KeySpec>,
        secrets: Vec<SecretRow>,
        editor: &ConfigEditor,
    ) -> Self {
        let rows = build_rows(&keys, secrets, options.group_title);
        let defaults_cfg = Config::default();
        let defaults = keys
            .iter()
            .map(|k| {
                (
                    k.key,
                    schema::resolve(k.key, &defaults_cfg).unwrap_or(Json::Null),
                )
            })
            .collect();
        let mut form = Self {
            options,
            rows,
            cursor: 0,
            editing: None,
            pending: BTreeMap::new(),
            file: HashMap::new(),
            defaults,
            feedback: None,
        };
        form.read_file(editor);
        form.cursor = form.rows.iter().position(Row::focusable).unwrap_or(0);
        form
    }

    fn read_file(&mut self, editor: &ConfigEditor) {
        self.file = self
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Key(k) => Some((k.key, schema::file_value_found(&Found::Key(k), editor))),
                _ => None,
            })
            .collect();
    }

    #[cfg(test)]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn is_dirty(&self) -> bool {
        !self.pending.is_empty()
    }

    #[cfg(test)]
    pub fn pending(&self) -> &BTreeMap<&'static str, Pending> {
        &self.pending
    }

    /// The focused row.
    #[cfg(test)]
    pub fn focused(&self) -> Option<&Row> {
        self.rows.get(self.cursor)
    }

    /// Put the cursor on `key`'s row.
    #[cfg(test)]
    pub fn focus_key(&mut self, key: &str) -> bool {
        match self
            .rows
            .iter()
            .position(|r| matches!(r, Row::Key(k) if k.key == key))
        {
            Some(i) => {
                self.cursor = i;
                true
            }
            None => false,
        }
    }

    /// The value `key` has now: pending, else in the file, else its default.
    pub fn value(&self, spec: &KeySpec) -> Json {
        match self.pending.get(spec.key) {
            Some(Pending::Set(v)) => v.to_json(),
            Some(Pending::Unset) => self.defaults.get(spec.key).cloned().unwrap_or(Json::Null),
            None => match self.file.get(spec.key) {
                Some(v) if !v.is_null() => v.clone(),
                _ => self.defaults.get(spec.key).cloned().unwrap_or(Json::Null),
            },
        }
    }

    /// Is `key` taking its default (nothing in the file, nothing pending)?
    fn is_default(&self, spec: &KeySpec) -> bool {
        match self.pending.get(spec.key) {
            Some(Pending::Unset) => true,
            Some(Pending::Set(_)) => false,
            None => self.file.get(spec.key).is_none_or(Json::is_null),
        }
    }

    /// The row's value as the form shows it.
    pub fn display_value(&self, row: &Row) -> String {
        match row {
            Row::Heading(_) => String::new(),
            Row::Secret(s) => (if s.set { "set" } else { "not set" }).to_string(),
            Row::Key(spec) => {
                let value = self.value(spec);
                let mut text = if sensitive(spec) {
                    match &value {
                        Json::String(v) if !v.is_empty() => "set".to_string(),
                        _ => "not set".to_string(),
                    }
                } else {
                    display(&value)
                };
                if !spec.compiled() {
                    text.push_str(" (not in this build)");
                } else if self.pending.contains_key(spec.key) {
                    text.push_str(" *");
                } else if self.is_default(spec) {
                    text.push_str(" (default)");
                }
                text
            }
        }
    }

    fn master_off(&self) -> bool {
        let Some(master) = self.options.master else {
            return false;
        };
        let Some(spec) = schema::scalar_keys().find(|k| k.key == master) else {
            return false;
        };
        self.value(spec) == Json::Bool(false)
    }

    pub fn move_cursor(&mut self, delta: i32) {
        let focusable: Vec<usize> = (0..self.rows.len())
            .filter(|&i| self.rows[i].focusable())
            .collect();
        if focusable.is_empty() {
            return;
        }
        let at = focusable
            .iter()
            .position(|&i| i == self.cursor)
            .unwrap_or(0) as i32;
        let next = (at + delta).rem_euclid(focusable.len() as i32) as usize;
        self.cursor = focusable[next];
    }

    fn set(&mut self, spec: &'static KeySpec, pending: Pending) {
        // Back to what the file already says is no change at all.
        let same = match (&pending, self.file.get(spec.key)) {
            (Pending::Unset, None | Some(Json::Null)) => true,
            (Pending::Set(v), Some(file)) => &v.to_json() == file,
            _ => false,
        };
        if same {
            self.pending.remove(spec.key);
        } else {
            self.pending.insert(spec.key, pending);
        }
        self.feedback = None;
    }

    fn focused_spec(&self) -> Option<&'static KeySpec> {
        match self.rows.get(self.cursor) {
            Some(Row::Key(k)) => Some(k),
            _ => None,
        }
    }

    /// ←→: toggle, cycle or step the focused row.
    pub fn change(&mut self, delta: i32) {
        let Some(spec) = self.focused_spec() else {
            self.secret_hint();
            return;
        };
        let current = self.value(spec);
        let next = match spec.ty {
            KeyType::Bool => Some(TypedValue::Bool(!current.as_bool().unwrap_or(false))),
            KeyType::Enum { choices, .. } => {
                let now = current.as_str().unwrap_or_default();
                let mut options: Vec<&str> = choices.to_vec();
                if !now.is_empty() && !options.contains(&now) {
                    options.push(now);
                }
                let at = options.iter().position(|c| *c == now);
                let next = match at {
                    Some(i) => (i as i32 + delta).rem_euclid(options.len() as i32) as usize,
                    None if delta < 0 => options.len() - 1,
                    None => 0,
                };
                Some(TypedValue::Str(options[next].to_string()))
            }
            KeyType::Int { min, max } => {
                let now = current.as_i64().unwrap_or(min);
                Some(TypedValue::Int(
                    (now + delta as i64 * int_step(min, max)).clamp(min, max),
                ))
            }
            KeyType::Float { min, max } => {
                let now = current.as_f64().unwrap_or(min);
                let stepped = now + delta as f64 * float_step(min, max);
                let rounded = (stepped * 100.0).round() / 100.0;
                Some(TypedValue::Float(rounded.clamp(min, max)))
            }
            _ => {
                self.feedback = Some((FeedbackLevel::Warn, "Press Enter to type a value".into()));
                None
            }
        };
        if let Some(v) = next {
            self.set(spec, Pending::Set(v));
        }
    }

    fn secret_hint(&mut self) {
        if let Some(Row::Secret(s)) = self.rows.get(self.cursor) {
            self.feedback = Some((
                FeedbackLevel::Warn,
                format!(
                    "{} is set with a command; see the help on the right",
                    s.label
                ),
            ));
        }
    }

    /// Enter: type a value for the focused row (a bool toggles instead).
    pub fn start_edit(&mut self) {
        let Some(spec) = self.focused_spec() else {
            self.secret_hint();
            return;
        };
        if spec.ty == KeyType::Bool {
            self.change(1);
            return;
        }
        let initial = if sensitive(spec) {
            String::new()
        } else {
            match self.value(spec) {
                Json::Null => String::new(),
                Json::String(s) => s,
                other => other.to_string(),
            }
        };
        self.editing = Some(TextInput::new(initial));
    }

    /// Enter in the text input: an empty value goes back to the default,
    /// anything else must pass the schema's validation.
    pub fn commit_edit(&mut self) {
        let (Some(input), Some(spec)) = (self.editing.take(), self.focused_spec()) else {
            return;
        };
        let text = input.buffer().trim();
        if text.is_empty() && spec.key != "history.exclude_apps" {
            self.set(spec, Pending::Unset);
            return;
        }
        match schema::validate_value(spec, text) {
            Ok(v) => self.set(spec, Pending::Set(v)),
            Err(e) => self.feedback = Some((FeedbackLevel::Err, e.to_string())),
        }
    }

    /// Delete: back to the default.
    pub fn reset_focused(&mut self) {
        if let Some(spec) = self.focused_spec() {
            self.set(spec, Pending::Unset);
        }
    }

    /// Write the pending changes through `editor` (loaded fresh by the
    /// caller). On failure nothing is written and the changes stay pending.
    pub fn save_with_editor(&mut self, editor: &mut ConfigEditor) {
        if self.pending.is_empty() {
            self.feedback = Some((FeedbackLevel::Ok, "Nothing to save".into()));
            return;
        }
        let mut restart = Vec::new();
        for (key, change) in &self.pending {
            let Some(spec) = schema::scalar_keys().find(|k| k.key == *key) else {
                continue;
            };
            match change {
                Pending::Set(v) => schema::apply(editor, &Found::Key(spec), v),
                Pending::Unset => editor.unset(spec.table, spec.field),
            }
            if spec.restart_required {
                restart.push(spec.label);
            }
        }
        match editor.save() {
            Ok(()) => {
                self.pending.clear();
                self.read_file(editor);
                self.feedback = Some(if restart.is_empty() {
                    (FeedbackLevel::Ok, "Saved; takes effect now".into())
                } else {
                    (
                        FeedbackLevel::Warn,
                        format!(
                            "Saved. Needs a daemon restart: {}. Quitting restarts a running daemon.",
                            restart.join(", ")
                        ),
                    )
                });
            }
            Err(e) => {
                self.feedback = Some((FeedbackLevel::Err, format!("Not saved: {e}")));
            }
        }
    }

    /// Re-read the file and drop pending changes.
    pub fn revert(&mut self, editor: &ConfigEditor) {
        self.pending.clear();
        self.editing = None;
        self.read_file(editor);
        self.feedback = Some((FeedbackLevel::Ok, "Reverted".into()));
    }

    /// Keys while not typing. Returns true when the key was used.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if let Some(input) = self.editing.as_mut() {
            match input.handle_key(key) {
                TextInputResult::Continue => {}
                TextInputResult::Commit => self.commit_edit(),
                TextInputResult::Cancel => self.editing = None,
            }
            return true;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-10),
            KeyCode::PageDown => self.move_cursor(10),
            KeyCode::Left | KeyCode::Char('h') => self.change(-1),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => self.change(1),
            KeyCode::Enter | KeyCode::Char('i') => self.start_edit(),
            KeyCode::Delete | KeyCode::Backspace => self.reset_focused(),
            _ => return false,
        }
        true
    }

    fn help(&self) -> Vec<Line<'static>> {
        let heading = |t: String| {
            Line::from(Span::styled(
                t,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        };
        let dim = |t: String| Line::from(Span::styled(t, Style::default().fg(Color::Gray)));
        match self.rows.get(self.cursor) {
            Some(Row::Secret(s)) => {
                let mut out = vec![
                    heading(s.label.clone()),
                    Line::from(""),
                    Line::from(s.description.clone()),
                    Line::from(""),
                    Line::from(if s.set { "Set." } else { "Not set." }),
                    Line::from(""),
                ];
                out.extend(s.help.lines().map(|l| dim(l.to_string())));
                out
            }
            Some(Row::Key(spec)) => {
                let mut out = vec![
                    heading(spec.label.to_string()),
                    Line::from(""),
                    Line::from(spec.description.to_string()),
                ];
                for extra in (self.options.extra_help)(spec.key) {
                    out.push(Line::from(""));
                    out.push(Line::from(extra));
                }
                out.push(Line::from(""));
                match spec.ty {
                    KeyType::Enum { choices, open } => out.push(dim(format!(
                        "Choices: {}{}",
                        choices.join(", "),
                        if open { ", or type another" } else { "" }
                    ))),
                    KeyType::Int { min, max } => out.push(dim(format!("Range: {min} to {max}"))),
                    KeyType::Float { min, max } => out.push(dim(format!("Range: {min} to {max}"))),
                    _ => {}
                }
                if !sensitive(spec) {
                    let default = self.defaults.get(spec.key).cloned().unwrap_or(Json::Null);
                    out.push(dim(format!("Default: {}", display(&default))));
                }
                out.push(dim(format!("Key: {}", spec.key)));
                out.push(if spec.restart_required {
                    Line::from(Span::styled(
                        "Takes effect after a daemon restart",
                        Style::default().fg(Color::Yellow),
                    ))
                } else {
                    dim("Takes effect at once".to_string())
                });
                if let Some(feature) = spec.requires_feature.filter(|_| !spec.compiled()) {
                    out.push(Line::from(Span::styled(
                        format!("This build lacks the {feature} feature, so it has no effect."),
                        Style::default().fg(Color::Yellow),
                    )));
                }
                out
            }
            _ => Vec::new(),
        }
    }

    /// The form's rows, as the settings panel draws them.
    pub fn row_specs(&self) -> Vec<FormRowSpec> {
        let dim = self.master_off();
        self.rows
            .iter()
            .enumerate()
            .map(|(i, row)| match row {
                Row::Heading(t) => FormRowSpec::heading(t.clone()),
                Row::Key(spec) => {
                    let focused = i == self.cursor;
                    let value = match (&self.editing, focused) {
                        (Some(input), true) => input.caret_string(),
                        _ => self.display_value(row),
                    };
                    FormRowSpec::new(focused, spec.label, value)
                        .dimmed(dim && Some(spec.key) != self.options.master)
                }
                Row::Secret(s) => {
                    FormRowSpec::new(i == self.cursor, s.label.clone(), self.display_value(row))
                        .dimmed(dim)
                }
            })
            .collect()
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        let rows = self.row_specs();
        let feedback = self.feedback.as_ref().map(|(l, m)| (*l, m.as_str()));
        let hint = if self.editing.is_some() {
            " Enter set   Esc cancel   empty = default "
        } else {
            " ↑↓ field   ←→ change   Enter type   Del default   s save   r revert "
        };
        common::render_form_with_guidance_hint(
            f,
            area,
            self.options.title,
            self.is_dirty(),
            feedback,
            &rows,
            self.help(),
            hint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_display_short() {
        assert_eq!(display(&serde_json::json!(0.009999999776482582)), "0.01");
        assert_eq!(display(&serde_json::json!(0.004999999888241291)), "0.005");
        assert_eq!(display(&serde_json::json!(4.0)), "4.0");
        assert_eq!(display(&serde_json::json!(0.8)), "0.8");
        assert_eq!(display(&serde_json::json!(30)), "30");
        assert_eq!(display(&serde_json::json!("")), "empty");
        assert_eq!(display(&serde_json::json!(true)), "yes");
        assert_eq!(display(&Json::Null), "unset");
    }

    #[test]
    fn number_steps_scale_with_the_range() {
        assert_eq!(int_step(5, 600), 1);
        assert_eq!(int_step(0, 1440), 10);
        assert_eq!(int_step(1, 86400), 100);
        assert_eq!(int_step(2_000, 2_000_000), 10_000);
        assert_eq!(float_step(0.3, 1.0), 0.05);
        assert_eq!(float_step(0.1, 30.0), 1.0);
    }
}
