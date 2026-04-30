//! Left-hand sidebar navigation.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::app::App;
use super::section::Section;

/// Width of the sidebar column. Wide enough for "Notifications" + a marker
/// without wrapping.
pub const WIDTH: u16 = 18;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .style(Style::default());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    for section in Section::ALL {
        let active = *section == app.current_section;
        let focused_in_sidebar = app.sidebar_focused;
        let style = if active && focused_in_sidebar {
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else if active {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if focused_in_sidebar
            && hovered_section(app).map(|s| s == *section).unwrap_or(false)
        {
            Style::default().bg(Color::DarkGray).fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        };
        let marker = if active { "▶ " } else { "  " };
        lines.push(Line::from(vec![Span::styled(
            format!("{}{}", marker, section.label()),
            style,
        )]));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn hovered_section(app: &App) -> Option<Section> {
    Section::ALL.get(app.sidebar_cursor).copied()
}
