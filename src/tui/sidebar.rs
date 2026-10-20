//! The page list down the left side: the product name, then the pages,
//! with a blank line between groups. No box and no headings.

use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::app::App;
use super::common;
use super::section::Section;
use super::theme;

/// Width of the list column: the longest page name, the `›` marker and a
/// space on either side.
pub const WIDTH: u16 = 21;

/// Lines above the first page: the product name and a blank line.
const TOP: u16 = 2;

/// One line of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Gap,
    Item(usize),
}

/// The list's lines for `height` rows of pages. The gaps between groups go
/// when the terminal is too short for them.
pub fn rows(height: u16) -> Vec<Row> {
    let mut with_gaps = Vec::new();
    for (idx, section) in Section::ALL.iter().enumerate() {
        if idx > 0 && Section::ALL[idx - 1].group() != section.group() {
            with_gaps.push(Row::Gap);
        }
        with_gaps.push(Row::Item(idx));
    }
    if with_gaps.len() <= height as usize {
        with_gaps
    } else {
        (0..Section::ALL.len()).map(Row::Item).collect()
    }
}

/// The lines that fit in `height`, scrolled so page `focus` is visible.
pub fn window(height: u16, focus: usize) -> Vec<Row> {
    let all = rows(height);
    let at = all.iter().position(|r| *r == Row::Item(focus)).unwrap_or(0);
    let offset = common::scroll_offset(at, all.len(), height as usize);
    all.into_iter().skip(offset).take(height as usize).collect()
}

/// Page index under screen row `row` of a list drawn in `area`.
pub fn item_at(area: Rect, focus: usize, row: u16) -> Option<usize> {
    let line = row.checked_sub(area.y + TOP)?;
    match window(area.height.saturating_sub(TOP), focus).get(line as usize) {
        Some(Row::Item(idx)) => Some(*idx),
        _ => None,
    }
}

/// The page the list keeps in view: the one under the cursor while the
/// list has focus, otherwise the open one.
pub fn focus_index(app: &App) -> usize {
    if app.sidebar_focused {
        app.sidebar_cursor
    } else {
        Section::ALL
            .iter()
            .position(|s| *s == app.current_section)
            .unwrap_or(0)
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let width = area.width as usize;
    let mut lines = vec![
        Line::from(vec![
            Span::raw(" "),
            Span::styled("voxtype", theme::heading()),
        ]),
        Line::from(""),
    ];
    for row in window(area.height.saturating_sub(TOP), focus_index(app)) {
        lines.push(match row {
            Row::Gap => Line::from(""),
            Row::Item(idx) => item_line(app, idx, width),
        });
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn item_line(app: &App, idx: usize, width: usize) -> Line<'static> {
    let section = Section::ALL[idx];
    let open = section == app.current_section;
    let hovered = app.sidebar_focused && idx == app.sidebar_cursor;
    let label = common::truncate(section.label(), width.saturating_sub(4));
    let padded = format!(" {:<w$} ", label, w = width.saturating_sub(4));
    let marker = Span::styled(
        if open { " ›" } else { "  " },
        theme::accent().add_modifier(Modifier::BOLD),
    );
    let style = if hovered {
        theme::selected()
    } else if open {
        theme::accent().add_modifier(Modifier::BOLD)
    } else {
        theme::text()
    };
    Line::from(vec![marker, Span::styled(padded, style)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_separate_groups_when_they_fit() {
        let all = rows(40);
        assert_eq!(all[0], Row::Item(0));
        assert_eq!(all[1], Row::Gap);
        assert_eq!(
            all.iter().filter(|r| matches!(r, Row::Item(_))).count(),
            Section::ALL.len()
        );
        assert_ne!(all.last(), Some(&Row::Gap));
    }

    #[test]
    fn short_terminals_drop_gaps_but_keep_every_page() {
        let short = rows(Section::ALL.len() as u16);
        assert_eq!(short.len(), Section::ALL.len());
        assert!(short.iter().all(|r| matches!(r, Row::Item(_))));
    }

    #[test]
    fn clicks_land_on_pages_not_gaps() {
        let area = Rect::new(0, 0, WIDTH, 30);
        assert_eq!(item_at(area, 0, 0), None);
        assert_eq!(item_at(area, 0, TOP), Some(0));
        assert_eq!(item_at(area, 0, TOP + 1), None);
        assert_eq!(item_at(area, 0, TOP + 2), Some(1));
    }

    #[test]
    fn a_short_list_scrolls_to_keep_the_focused_page_visible() {
        let last = Section::ALL.len() - 1;
        let shown = window(10, last);
        assert_eq!(shown.len(), 10);
        assert!(shown.contains(&Row::Item(last)));
    }
}
