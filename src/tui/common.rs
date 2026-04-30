//! Shared rendering helpers for form-style sections (Hotkey, Audio, Output, …).

#![allow(dead_code)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// Minimal single-line text input. Owned by a section's state when a free-text
/// field is being edited; sections check whether `editing` is `Some` and route
/// keys to [`TextInput::handle_key`] while it is.
#[derive(Debug, Clone)]
pub struct TextInput {
    buffer: String,
    /// Byte offset within `buffer`. Always lands on a UTF-8 char boundary.
    cursor: usize,
    /// Original value at the time editing started — used to detect "no change"
    /// and reportable on Cancel.
    original: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextInputResult {
    /// Key was consumed but editing continues.
    Continue,
    /// User pressed Enter; commit `buffer()` to the underlying field.
    Commit,
    /// User pressed Esc; discard buffer.
    Cancel,
}

impl TextInput {
    pub fn new(initial: impl Into<String>) -> Self {
        let buffer: String = initial.into();
        let cursor = buffer.len();
        Self {
            original: buffer.clone(),
            buffer,
            cursor,
        }
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    pub fn changed(&self) -> bool {
        self.buffer != self.original
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> TextInputResult {
        match key.code {
            KeyCode::Enter => TextInputResult::Commit,
            KeyCode::Esc => TextInputResult::Cancel,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                TextInputResult::Cancel
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let prev = prev_char_boundary(&self.buffer, self.cursor);
                    self.buffer.replace_range(prev..self.cursor, "");
                    self.cursor = prev;
                }
                TextInputResult::Continue
            }
            KeyCode::Delete => {
                if self.cursor < self.buffer.len() {
                    let next = next_char_boundary(&self.buffer, self.cursor);
                    self.buffer.replace_range(self.cursor..next, "");
                }
                TextInputResult::Continue
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor = prev_char_boundary(&self.buffer, self.cursor);
                }
                TextInputResult::Continue
            }
            KeyCode::Right => {
                if self.cursor < self.buffer.len() {
                    self.cursor = next_char_boundary(&self.buffer, self.cursor);
                }
                TextInputResult::Continue
            }
            KeyCode::Home => {
                self.cursor = 0;
                TextInputResult::Continue
            }
            KeyCode::End => {
                self.cursor = self.buffer.len();
                TextInputResult::Continue
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = 0;
                TextInputResult::Continue
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = self.buffer.len();
                TextInputResult::Continue
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Clear the line.
                self.buffer.clear();
                self.cursor = 0;
                TextInputResult::Continue
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Delete the previous word.
                let prev_word = prev_word_boundary(&self.buffer, self.cursor);
                self.buffer.replace_range(prev_word..self.cursor, "");
                self.cursor = prev_word;
                TextInputResult::Continue
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let mut tmp = [0u8; 4];
                let s = c.encode_utf8(&mut tmp);
                self.buffer.insert_str(self.cursor, s);
                self.cursor += s.len();
                TextInputResult::Continue
            }
            _ => TextInputResult::Continue,
        }
    }

    /// Plain-text rendering of the buffer with a `│` caret inserted at the
    /// cursor position. Suitable for slotting into a form row's value column
    /// where we can't easily run multi-span styling.
    pub fn caret_string(&self) -> String {
        let mut out = String::with_capacity(self.buffer.len() + 1);
        out.push_str(&self.buffer[..self.cursor]);
        out.push('│');
        out.push_str(&self.buffer[self.cursor..]);
        out
    }

    /// Render the buffer with a visible cursor caret. Returned line is meant
    /// to slot into a form row's "value" column.
    pub fn render_inline(&self) -> Line<'static> {
        let (before, at, after) = split_at_cursor(&self.buffer, self.cursor);
        let caret_glyph = if at.is_empty() { " ".to_string() } else { at };
        Line::from(vec![
            Span::raw(before),
            Span::styled(
                caret_glyph,
                Style::default().bg(Color::White).fg(Color::Black),
            ),
            Span::raw(after),
        ])
    }
}

fn prev_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.saturating_sub(1);
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn next_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = (idx + 1).min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

fn prev_word_boundary(s: &str, idx: usize) -> usize {
    let bytes = s.as_bytes();
    let mut i = idx;
    // Skip trailing spaces.
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    // Skip non-space characters.
    while i > 0 && !bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn typing_appends_to_buffer() {
        let mut input = TextInput::new("");
        input.handle_key(key(KeyCode::Char('h')));
        input.handle_key(key(KeyCode::Char('i')));
        assert_eq!(input.buffer(), "hi");
    }

    #[test]
    fn backspace_deletes_prev_char() {
        let mut input = TextInput::new("hello");
        assert_eq!(input.handle_key(key(KeyCode::Backspace)), TextInputResult::Continue);
        assert_eq!(input.buffer(), "hell");
    }

    #[test]
    fn left_then_insert_inserts_mid_string() {
        let mut input = TextInput::new("ac");
        input.handle_key(key(KeyCode::Left));
        input.handle_key(key(KeyCode::Char('b')));
        assert_eq!(input.buffer(), "abc");
    }

    #[test]
    fn enter_signals_commit() {
        let mut input = TextInput::new("done");
        assert_eq!(input.handle_key(key(KeyCode::Enter)), TextInputResult::Commit);
    }

    #[test]
    fn esc_signals_cancel() {
        let mut input = TextInput::new("x");
        assert_eq!(input.handle_key(key(KeyCode::Esc)), TextInputResult::Cancel);
    }

    #[test]
    fn ctrl_u_clears() {
        let mut input = TextInput::new("hello");
        input.handle_key(ctrl('u'));
        assert_eq!(input.buffer(), "");
    }

    #[test]
    fn ctrl_w_deletes_prev_word() {
        let mut input = TextInput::new("hello world");
        input.handle_key(ctrl('w'));
        assert_eq!(input.buffer(), "hello ");
    }

    #[test]
    fn changed_tracks_buffer_vs_original() {
        let mut input = TextInput::new("abc");
        assert!(!input.changed());
        input.handle_key(key(KeyCode::Char('d')));
        assert!(input.changed());
    }
}

fn split_at_cursor(s: &str, idx: usize) -> (String, String, String) {
    if idx >= s.len() {
        return (s.to_string(), String::new(), String::new());
    }
    let mut next = idx;
    while next < s.len() && !s.is_char_boundary(next + 1) {
        next += 1;
    }
    next = (next + 1).min(s.len());
    while next < s.len() && !s.is_char_boundary(next) {
        next += 1;
    }
    (
        s[..idx].to_string(),
        s[idx..next].to_string(),
        s[next..].to_string(),
    )
}

#[derive(Debug, Clone, Copy)]
pub enum FeedbackLevel {
    Ok,
    Err,
}

pub fn render_feedback(f: &mut Frame, area: Rect, level: FeedbackLevel, message: &str) {
    let style = match level {
        FeedbackLevel::Ok => Style::default().fg(Color::Green),
        FeedbackLevel::Err => Style::default().fg(Color::Red),
    };
    let prefix = match level {
        FeedbackLevel::Ok => "✓ ",
        FeedbackLevel::Err => "✗ ",
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{}{}", prefix, message),
            style,
        ))),
        area,
    );
}

pub fn render_section_header(f: &mut Frame, area: Rect, title: &str, dirty: bool) {
    let dirty_span = if dirty {
        Span::styled("  • unsaved", Style::default().fg(Color::Yellow))
    } else {
        Span::raw("")
    };
    let line = Line::from(vec![
        Span::styled(
            title.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        dirty_span,
    ]);
    f.render_widget(Paragraph::new(vec![line, Line::from("")]), area);
}

pub fn render_bottom_hint(f: &mut Frame, area: Rect, dirty: bool) {
    let dirty_marker = if dirty {
        Span::styled("  ●", Style::default().fg(Color::Yellow))
    } else {
        Span::raw("")
    };
    let line = Line::from(vec![
        Span::styled(
            " ↑↓ field   ←→ change   s save   r revert ",
            Style::default().fg(Color::Gray),
        ),
        dirty_marker,
    ]);
    f.render_widget(Paragraph::new(line), area);
}

/// Single form row: focused or unfocused, with a label-and-value layout that
/// matches the rest of the form sections.
pub fn form_row<'a>(focused: bool, label: &str, value: &str) -> Line<'a> {
    form_row_dimmed(focused, false, label, value)
}

/// Form row that supports a `dimmed` variant for fields disabled by another
/// toggle (e.g. the rest of the Hotkey form when the evdev listener is off).
pub fn form_row_dimmed<'a>(
    focused: bool,
    dimmed: bool,
    label: &str,
    value: &str,
) -> Line<'a> {
    let dim_color = Color::DarkGray;
    let label_style = if dimmed {
        Style::default().fg(dim_color)
    } else if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let value_style = if dimmed {
        Style::default().fg(dim_color)
    } else if focused {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    } else {
        Style::default().fg(Color::White)
    };
    let prefix = if focused { "▸ " } else { "  " };
    Line::from(vec![
        Span::styled(format!("{}{:<32}", prefix, label), label_style),
        Span::styled(format!(" ◂ {} ▸", value), value_style),
    ])
}

/// Specification for a row in a two-pane form.
pub struct FormRowSpec {
    pub focused: bool,
    pub dimmed: bool,
    pub label: String,
    pub value: String,
}

impl FormRowSpec {
    pub fn new(focused: bool, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            focused,
            dimmed: false,
            label: label.into(),
            value: value.into(),
        }
    }

    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }
}

/// Render a section using the General-style two-panel layout: a form panel on
/// the left (rows, save/revert hints) and a guidance panel on the right that
/// shows context-sensitive help for the focused row.
///
/// Layout (vertical):
///   1 row  feedback (only present if `feedback` is Some)
///   2 rows section title + dirty marker
///   N rows two columns: form (Settings) on left, guidance (About) on right
///   1 row  bottom hint
pub fn render_form_with_guidance(
    f: &mut Frame,
    area: Rect,
    title: &str,
    dirty: bool,
    feedback: Option<(FeedbackLevel, &str)>,
    rows: &[FormRowSpec],
    guidance: Vec<Line<'_>>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if feedback.is_some() { 2 } else { 0 }),
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(area);

    if let Some((lvl, msg)) = feedback {
        render_feedback(f, chunks[0], lvl, msg);
    }
    render_section_header(f, chunks[1], title, dirty);

    // Two columns: Settings on the left, About on the right.
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[2]);

    render_settings_panel(f, body[0], rows);
    render_guidance_panel(f, body[1], guidance);

    render_bottom_hint(f, chunks[3], dirty);
}

fn render_settings_panel(f: &mut Frame, area: Rect, rows: &[FormRowSpec]) {
    let block = Block::default().borders(Borders::ALL).title("Settings");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines: Vec<Line> = rows
        .iter()
        .map(|r| form_row_dimmed(r.focused, r.dimmed, &r.label, &r.value))
        .collect();
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_guidance_panel(f: &mut Frame, area: Rect, lines: Vec<Line<'_>>) {
    let block = Block::default().borders(Borders::ALL).title("About");
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
