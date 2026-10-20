//! Shared pieces for pages: text input, the form renderer, text helpers.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph, Wrap},
    Frame,
};

use super::app::App;
use super::theme;

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
    #[allow(dead_code)]
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

    #[allow(dead_code)]
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
        out.push(CARET);
        out.push_str(&self.buffer[self.cursor..]);
        out
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
        assert_eq!(
            input.handle_key(key(KeyCode::Backspace)),
            TextInputResult::Continue
        );
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
        assert_eq!(
            input.handle_key(key(KeyCode::Enter)),
            TextInputResult::Commit
        );
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackLevel {
    Ok,
    Err,
    Warn,
}

/// The caret glyph [`TextInput::caret_string`] inserts. A row whose value
/// contains it is mid-edit, so it never becomes the saved baseline.
pub const CARET: char = '│';

/// What a boolean setting reads as everywhere in the TUI.
pub fn on_off(b: bool) -> String {
    (if b { "On" } else { "Off" }).to_string()
}

/// What an optional setting with no value reads as everywhere in the TUI.
pub const NOT_SET: &str = "Not set";

/// Lines the explanation under the settings keeps when the settings would
/// otherwise fill the pane.
const DESCRIPTION_MIN_HEIGHT: u16 = 3;

/// Widest the explanation runs, so it stays readable on wide terminals.
const DESCRIPTION_MAX_WIDTH: u16 = 76;

/// Gap between choices shown side by side. Each choice already carries a
/// space on either side, so the marked one reads as a chip.
const CHOICE_GAP: &str = "";

/// One setting on a page.
#[derive(Debug, Clone)]
pub struct FormRowSpec {
    pub focused: bool,
    pub dimmed: bool,
    pub label: String,
    pub value: String,
    /// Every value the setting can take, when there are few enough to show
    /// side by side. `value` is the current one.
    pub choices: Option<Vec<String>>,
    /// A problem to fix rather than a setting: drawn in the warning color.
    pub attention: bool,
}

impl FormRowSpec {
    pub fn new(focused: bool, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            focused,
            dimmed: false,
            label: label.into(),
            value: value.into(),
            choices: None,
            attention: false,
        }
    }

    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }

    /// Mark the row as a problem to fix.
    pub fn attention(mut self) -> Self {
        self.attention = true;
        self
    }

    /// Show all of `choices` on the row, with the current value marked.
    pub fn choices<S: AsRef<str>>(mut self, choices: &[S]) -> Self {
        self.choices = Some(choices.iter().map(|c| c.as_ref().to_string()).collect());
        self
    }

    /// The choices to show inline: the explicit ones, or On and Off for a
    /// switch. `None` for free values and while typing.
    fn inline_choices(&self) -> Option<Vec<String>> {
        if self.value.contains(CARET) {
            return None;
        }
        match &self.choices {
            Some(c) if c.contains(&self.value) => Some(c.clone()),
            Some(_) => None,
            None if self.value == "On" || self.value == "Off" => {
                Some(vec!["On".to_string(), "Off".to_string()])
            }
            None => None,
        }
    }
}

/// The config file every section saves to, with the home directory shown as
/// `~` so it fits in a status line.
pub fn display_config_path() -> String {
    let Some(path) = super::config_editor::tui_config_path() else {
        return "config.toml".to_string();
    };
    let shown = path.display().to_string();
    match dirs::home_dir() {
        Some(home) => match path.strip_prefix(&home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => shown,
        },
        None => shown,
    }
}

/// Display width of `s` in terminal cells.
pub fn str_width(s: &str) -> usize {
    Span::raw(s).width()
}

/// Cut `s` to at most `max` cells, ending in `…` when anything was cut.
pub fn truncate(s: &str, max: usize) -> String {
    if str_width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = str_width(c.encode_utf8(&mut [0u8; 4]));
        if used + w > max - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Cut `s` to at most `max` cells by dropping the middle, which keeps both
/// the start and the file name of a long path readable.
pub fn truncate_middle(s: &str, max: usize) -> String {
    if str_width(s) <= max || max < 5 {
        return truncate(s, max);
    }
    let chars: Vec<char> = s.chars().collect();
    let keep = max - 1;
    let head = keep / 2;
    let tail = keep - head;
    let start: String = chars[..head.min(chars.len())].iter().collect();
    let end: String = chars[chars.len().saturating_sub(tail)..].iter().collect();
    format!("{}…{}", start, end)
}

/// Greedy word wrap to `width` cells.
pub fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if str_width(line) + 1 + str_width(word) <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// The first message that fits in `width` cells, or the last one cut to fit.
/// Lets a line drop whole clauses on narrow terminals instead of ending
/// mid-word.
pub fn first_that_fits(candidates: &[String], width: usize) -> String {
    candidates
        .iter()
        .find(|c| str_width(c) <= width)
        .cloned()
        .unwrap_or_else(|| truncate(candidates.last().map_or("", |c| c.as_str()), width))
}

/// A rounded pane. The focused pane takes the accent border so it is clear
/// where the arrow keys go.
pub fn pane<'a>(title: impl Into<String>, focused: bool) -> Block<'a> {
    let title: String = title.into();
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .padding(Padding::horizontal(1))
        .border_style(if focused {
            theme::border_focused()
        } else {
            theme::border()
        })
        .title(Span::styled(
            format!(" {} ", title),
            theme::pane_title(focused),
        ))
}

/// Width of the label column: as wide as the longest label, but leaving the
/// value at least a third of the row.
fn label_column_width(rows: &[FormRowSpec], inner_width: usize) -> usize {
    let longest = rows.iter().map(|r| str_width(&r.label)).max().unwrap_or(0);
    let value_room = (inner_width / 3).max(16);
    let cap = inner_width.saturating_sub(ROW_CHROME + value_room).max(12);
    longest.min(cap)
}

/// Cells a row spends outside its label and value: the cursor and its
/// space, and the space, unsaved marker and space between label and value.
const ROW_CHROME: usize = 2 + 3;

/// First visible row so the focused row stays on screen, keeping one row of
/// context below it when there is more to show.
pub fn scroll_offset(focused: usize, total: usize, visible: usize) -> usize {
    if visible == 0 || total <= visible {
        return 0;
    }
    let want_bottom = (focused + 2).min(total);
    want_bottom.saturating_sub(visible)
}

/// The value cell of a row. Choices that fit are all shown, the current one
/// marked; otherwise the value alone, between `‹ ›` on the focused row so it
/// is clear the arrows change it.
fn value_spans(row: &FormRowSpec, row_focused: bool, room: usize) -> Vec<Span<'static>> {
    let editing = row.value.contains(CARET);
    if row.attention {
        let style = theme::warn().add_modifier(Modifier::BOLD);
        return vec![Span::styled(
            truncate(&format!("! {}", row.value), room),
            if row_focused {
                style.add_modifier(Modifier::REVERSED)
            } else {
                style
            },
        )];
    }
    if row.dimmed {
        return vec![Span::styled(
            truncate(&format!(" {}", row.value), room),
            theme::muted(),
        )];
    }
    if let Some(choices) = row.inline_choices() {
        let width: usize = choices.iter().map(|c| str_width(c) + 2).sum::<usize>()
            + str_width(CHOICE_GAP) * choices.len().saturating_sub(1);
        if width <= room {
            let mut spans = Vec::new();
            for (i, choice) in choices.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw(CHOICE_GAP));
                }
                let style = if *choice != row.value {
                    theme::muted()
                } else if row_focused {
                    theme::selected()
                } else {
                    theme::accent().add_modifier(Modifier::BOLD)
                };
                spans.push(Span::styled(format!(" {} ", choice), style));
            }
            return spans;
        }
    }
    let value_style = if row.value == NOT_SET {
        theme::muted()
    } else {
        theme::text()
    };
    // A row that is an action rather than a value has nothing to cycle.
    if row_focused && !editing && !row.value.is_empty() {
        let inner = truncate(&row.value, room.saturating_sub(4));
        return vec![
            Span::styled("‹ ", theme::accent()),
            Span::styled(inner, value_style.add_modifier(Modifier::BOLD)),
            Span::styled(" ›", theme::accent()),
        ];
    }
    vec![Span::styled(
        truncate(&format!(" {}", row.value), room),
        if editing {
            theme::text().add_modifier(Modifier::BOLD)
        } else {
            value_style
        },
    )]
}

/// One row: cursor, label, unsaved marker, value. Only the focused row of a
/// focused page shows the cursor, so there is one selection on screen.
fn form_line(
    row: &FormRowSpec,
    modified: bool,
    pane_focused: bool,
    label_width: usize,
    width: usize,
) -> Line<'static> {
    let row_focused = row.focused && pane_focused;
    let label = format!("{:<w$}", truncate(&row.label, label_width), w = label_width);
    let label_style = if row_focused {
        theme::accent().add_modifier(Modifier::BOLD)
    } else if row.dimmed {
        theme::muted()
    } else {
        theme::text()
    };
    let mut spans = vec![
        Span::styled(
            if row_focused { "› " } else { "  " },
            theme::accent().add_modifier(Modifier::BOLD),
        ),
        Span::styled(label, label_style),
        Span::raw(" "),
        Span::styled(
            if modified { "●" } else { " " },
            theme::warn().add_modifier(Modifier::BOLD),
        ),
    ];
    let room = width.saturating_sub(ROW_CHROME + label_width);
    spans.extend(value_spans(row, row_focused, room));
    Line::from(spans)
}

/// Lines `lines` will occupy once wrapped to `width` cells.
fn wrapped_height(lines: &[Line<'_>], width: u16) -> u16 {
    let width = width.max(1) as usize;
    lines
        .iter()
        .map(|l| (l.width().max(1)).div_ceil(width) as u16)
        .sum()
}

/// The explanation for the focused setting, without the heading that
/// repeats its label and the blank line after it.
fn description_body(guidance: Vec<Line<'_>>) -> Vec<Line<'_>> {
    let is_heading =
        |l: &Line| l.spans.len() == 1 && l.spans[0].style.add_modifier.contains(Modifier::BOLD);
    let is_blank = |l: &Line| l.spans.iter().all(|s| s.content.trim().is_empty());
    let mut lines = guidance.into_iter().peekable();
    if lines.peek().is_some_and(is_heading) {
        lines.next();
        while lines.peek().is_some_and(is_blank) {
            lines.next();
        }
    }
    lines.collect()
}

/// Render a page: one pane titled with the page name, the settings, then a
/// blank line and the explanation of the setting under the cursor.
///
/// The settings scroll to keep the focused row visible, and the
/// explanation scrolls with PgUp/PgDn. A changed row carries a `●`; the
/// page's message goes to the status line under the pane.
pub fn render_form_with_guidance(
    f: &mut Frame,
    area: Rect,
    app: &App,
    feedback: Option<(FeedbackLevel, &str)>,
    rows: &[FormRowSpec],
    guidance: Vec<Line<'_>>,
) {
    app.set_page_message(feedback);
    let section = app.current_section;
    let focused = app.content_focused();
    let mut block = pane(section.label(), focused);
    let inner = block.inner(area);

    let description = description_body(guidance);
    let desc_width = inner.width.min(DESCRIPTION_MAX_WIDTH);
    let desc_needed = wrapped_height(&description, desc_width);
    // The settings get what they need unless that would squeeze the
    // explanation below a few lines; one blank line sits between the two.
    let desc_floor = desc_needed.min(DESCRIPTION_MIN_HEIGHT);
    let rows_height = (rows.len() as u16)
        .min(inner.height.saturating_sub(desc_floor + 1))
        .max(inner.height.min(1));

    let visible = rows_height as usize;
    let focused_idx = rows.iter().position(|r| r.focused).unwrap_or(0);
    let offset = scroll_offset(focused_idx, rows.len(), visible);
    let below = rows.len().saturating_sub(offset + visible);
    if offset > 0 || below > 0 {
        let hint = match (offset > 0, below > 0) {
            (true, true) => format!(" ↑↓ {} more ", offset + below),
            (true, false) => format!(" ↑ {} more ", offset),
            _ => format!(" ↓ {} more ", below),
        };
        block = block.title_bottom(Line::from(Span::styled(hint, theme::muted())));
    }

    let modified = app.modified_rows(section, rows);
    let width = inner.width as usize;
    let label_width = label_column_width(rows, width);
    let lines: Vec<Line> = rows
        .iter()
        .zip(modified.iter())
        .skip(offset)
        .take(visible)
        .map(|(row, m)| form_line(row, *m, focused, label_width, width))
        .collect();

    let desc_area = Rect {
        x: inner.x + 2,
        y: inner.y + rows_height + 1,
        width: desc_width.saturating_sub(2),
        height: inner.height.saturating_sub(rows_height + 1),
    };
    if desc_area.height > 0 && !description.is_empty() {
        let total = wrapped_height(&description, desc_area.width);
        let max_scroll = total.saturating_sub(desc_area.height);
        let scroll = app.guidance_scroll.get().min(max_scroll);
        app.guidance_scroll.set(scroll);
        if max_scroll > 0 {
            let hint = if scroll < max_scroll {
                " PgDn more "
            } else {
                " PgUp back "
            };
            block =
                block.title_bottom(Line::from(Span::styled(hint, theme::muted())).right_aligned());
        }
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new(description)
                .style(theme::muted())
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            desc_area,
        );
    } else {
        f.render_widget(block, area);
    }
    let rows_area = Rect {
        height: rows_height,
        ..inner
    };
    f.render_widget(Paragraph::new(lines), rows_area);
}

/// Shown in place of a page whose settings could not be read.
pub fn render_load_error(f: &mut Frame, area: Rect, app: &App) {
    let block = pane(app.current_section.label(), app.content_focused());
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!("✗ Could not read {}", display_config_path()),
                theme::error(),
            )),
            Line::from(""),
            Line::from(
                "Run `voxtype config` to see what is wrong with it, fix it, then reopen this.",
            ),
        ])
        .wrap(Wrap { trim: true }),
        inner,
    );
}

#[cfg(test)]
mod render_tests {
    use super::*;

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn row(focused: bool, label: &str, value: &str) -> FormRowSpec {
        FormRowSpec::new(focused, label, value)
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn truncate_middle_keeps_both_ends_of_a_path() {
        let cut = truncate_middle("/home/someone/.config/voxtype/config.toml", 20);
        assert_eq!(str_width(&cut), 20);
        assert!(cut.starts_with("/home"));
        assert!(cut.ends_with(".toml"));
        assert!(cut.contains('…'));
    }

    #[test]
    fn first_that_fits_drops_whole_clauses() {
        let options = [
            "Not saved yet: s writes the file, r reverts".to_string(),
            "Not saved yet".to_string(),
        ];
        assert_eq!(first_that_fits(&options, 80), options[0]);
        assert_eq!(first_that_fits(&options, 20), "Not saved yet");
        assert_eq!(first_that_fits(&options, 5), "Not …");
    }

    #[test]
    fn scrolling_keeps_the_focused_row_and_one_after_it_visible() {
        assert_eq!(scroll_offset(0, 15, 10), 0);
        assert_eq!(scroll_offset(8, 15, 10), 0);
        // Row 9 is the last visible one; one row of context follows it.
        assert_eq!(scroll_offset(9, 15, 10), 1);
        assert_eq!(scroll_offset(14, 15, 10), 5);
        assert_eq!(scroll_offset(3, 5, 10), 0);
    }

    #[test]
    fn long_labels_leave_the_value_a_third_of_the_row() {
        let rows = [row(false, &"x".repeat(60), "v")];
        let width = label_column_width(&rows, 60);
        assert!(60 - width >= 20, "label took {} of 60", width);
        let short = [row(false, "Mode", "v")];
        assert_eq!(label_column_width(&short, 60), 4);
    }

    #[test]
    fn only_the_focused_row_of_a_focused_page_shows_the_cursor() {
        let focused = row(true, "Key", "HOME");
        let other = row(false, "Cancel", "ESC");
        let text = line_text(&form_line(&focused, false, true, 6, 40));
        assert!(text.starts_with("› Key"), "{text:?}");
        assert!(text.contains("‹ HOME ›"), "{text:?}");
        assert!(!line_text(&form_line(&other, false, true, 6, 40)).contains('‹'));
        // Focus in the page list: nothing in the page is highlighted.
        let unfocused = line_text(&form_line(&focused, false, false, 6, 40));
        assert!(!unfocused.contains('›') && !unfocused.contains('‹'));
    }

    #[test]
    fn few_valued_settings_show_every_choice() {
        let r = row(false, "Mode", "Toggle").choices(&["Hold to talk", "Toggle"]);
        let text = line_text(&form_line(&r, false, true, 6, 60));
        assert!(text.contains(" Hold to talk  Toggle "), "{text:?}");
        // Switches get On and Off without asking.
        let s = row(false, "Sounds", "Off");
        assert!(line_text(&form_line(&s, false, true, 6, 60)).contains(" On  Off "));
    }

    #[test]
    fn choices_that_do_not_fit_fall_back_to_the_value() {
        let r = row(true, "Mode", "Toggle").choices(&["Hold to talk", "Toggle"]);
        let text = line_text(&form_line(&r, false, true, 6, 24));
        assert!(text.contains("‹ Toggle ›"), "{text:?}");
    }

    #[test]
    fn unsaved_rows_carry_a_marker_that_is_not_only_color() {
        let r = row(false, "Mode", "x");
        assert!(line_text(&form_line(&r, true, true, 6, 40)).contains("Mode   ●"));
        assert!(!line_text(&form_line(&r, false, true, 6, 40)).contains('●'));
    }

    #[test]
    fn rows_being_typed_into_show_the_caret_not_the_arrows() {
        let r = row(true, "Key", &format!("HO{}ME", CARET));
        let text = line_text(&form_line(&r, false, true, 6, 40));
        assert!(!text.contains('‹'));
        assert!(text.contains("HO│ME"));
    }

    #[test]
    fn the_heading_that_repeats_the_label_is_dropped() {
        let lines = vec![
            Line::from(Span::styled("Mode", theme::heading())),
            Line::from(""),
            Line::from("Body."),
        ];
        let body = description_body(lines);
        assert_eq!(body.len(), 1);
        assert_eq!(line_text(&body[0]), "Body.");
    }
}
