//! Everything around the pages: the status line, the key footer, the help
//! overlay, the quit question and the too-small screen.

use ratatui::{
    layout::{Alignment, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
    Frame,
};

use super::app::App;
use super::common::{self, first_that_fits, str_width, truncate, FeedbackLevel};
use super::section::Section;
use super::theme;

/// Smallest terminal the layout is designed for. Below it the TUI asks for
/// more room instead of drawing a broken screen.
pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 16;

/// Spacing between footer key groups.
const FOOTER_GAP: &str = "   ";

/// Most hints the footer shows; the rest are under `?`.
const FOOTER_MAX: usize = 6;

pub fn render_too_small(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled("Terminal too small", theme::heading())),
        Line::from(Span::styled(
            format!(
                "voxtype configure needs {}x{}; this is {}x{}.",
                MIN_WIDTH, MIN_HEIGHT, area.width, area.height
            ),
            theme::muted(),
        )),
        Line::from(Span::styled("q quits.", theme::muted())),
    ];
    let y = area.y + area.height.saturating_sub(lines.len() as u16) / 2;
    let rect = Rect {
        x: area.x,
        y,
        width: area.width,
        height: (lines.len() as u16).min(area.height),
    };
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        rect,
    );
}

/// What the status line says, by priority: the last save or action result,
/// the open page's own message, the unsaved count, then what voxtype is
/// running and which file this edits.
pub fn status_text(app: &App, width: usize) -> (Option<FeedbackLevel>, String) {
    if let Some((level, message)) = &app.notice {
        return (Some(*level), message.clone());
    }
    if let Some((level, message)) = app.page_message() {
        return (Some(level), message);
    }
    let unsaved = app.unsaved_count();
    if unsaved > 0 {
        let count = if unsaved == 1 {
            "1 unsaved change".to_string()
        } else {
            format!("{} unsaved changes", unsaved)
        };
        let applies = super::app::edits_daemon_config();
        let options = if applies {
            vec![
                format!("{} · s saves and restarts voxtype", count),
                format!("{} · s saves", count),
            ]
        } else {
            vec![format!("{} · s saves", count)]
        };
        return (
            None,
            format!("● {}", first_that_fits(&options, width.saturating_sub(2))),
        );
    }
    let path = common::display_config_path();
    let daemon = match crate::daemon_status::running_version() {
        crate::daemon_status::DaemonVersion::Running(v) => format!("voxtype {} is running", v),
        crate::daemon_status::DaemonVersion::NotRunning => "voxtype isn't running".to_string(),
        crate::daemon_status::DaemonVersion::Unknown => "voxtype is running".to_string(),
    };
    let room = width.saturating_sub(str_width(&daemon) + 3);
    let short_path = common::truncate_middle(&path, room.max(14));
    (
        None,
        first_that_fits(
            &[
                format!("{} · {}", daemon, path),
                format!("{} · {}", daemon, short_path),
                daemon,
            ],
            width,
        ),
    )
}

/// The one status line under the page.
pub fn render_status_line(f: &mut Frame, area: Rect, app: &App) {
    let area = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    };
    let width = area.width as usize;
    let (level, text) = status_text(app, width);
    let line = match level {
        Some(level) => {
            let (glyph, style) = match level {
                FeedbackLevel::Ok => ("✓ ", theme::ok()),
                FeedbackLevel::Err => ("✗ ", theme::error()),
                FeedbackLevel::Warn => ("! ", theme::warn()),
            };
            Line::from(vec![
                Span::styled(glyph, style.add_modifier(Modifier::BOLD)),
                Span::styled(truncate(&text, width.saturating_sub(2)), style),
            ])
        }
        None if text.starts_with('●') => Line::from(Span::styled(text, theme::warn())),
        None => Line::from(Span::styled(text, theme::muted())),
    };
    f.render_widget(Paragraph::new(line), area);
}

/// Key hints for whatever has focus right now, most important first.
pub fn footer_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    if app.quit_pending {
        return vec![
            ("Enter", "save and quit"),
            ("d", "discard and quit"),
            ("Esc", "keep editing"),
        ];
    }
    if app.help_open {
        return vec![("↑↓", "scroll"), ("Esc", "close")];
    }
    if app.is_editing() {
        return vec![("Enter", "done"), ("Esc", "cancel"), ("Ctrl-U", "clear")];
    }
    let mut hints = if app.sidebar_focused {
        vec![("↑↓", "move"), ("Enter", "open")]
    } else if app.current_section == Section::Build {
        vec![("↑↓", "move"), ("Enter", "switch")]
    } else if app.current_section == Section::Basics && super::basics::on_problem_row(app) {
        vec![("↑↓", "move"), ("Enter", "fix")]
    } else {
        vec![("↑↓", "move"), ("←→", "change"), ("Enter", "edit")]
    };
    if app.unsaved_count() > 0 {
        hints.push(("s", "save"));
    }
    if !app.sidebar_focused {
        hints.push(("Esc", "back"));
    }
    hints.push(("?", "keys"));
    if app.sidebar_focused {
        hints.push(("q", "quit"));
    }
    hints.truncate(FOOTER_MAX);
    hints
}

/// The bottom line of key hints, dropped from the end when narrow.
pub fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let width = area.width as usize;
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    for (i, (key, desc)) in footer_hints(app).into_iter().enumerate() {
        let gap = if i == 0 { "" } else { FOOTER_GAP };
        let w = str_width(gap) + str_width(key) + 1 + str_width(desc);
        if used + w > width {
            break;
        }
        spans.push(Span::raw(gap));
        spans.push(Span::styled(key, theme::key()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(desc, theme::muted()));
        used += w;
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Asked on quit only when something is unsaved. The only dialog.
pub fn render_quit_prompt(f: &mut Frame, app: &App) {
    let area = f.area();
    let width = 52.min(area.width.saturating_sub(4));
    let inner_width = width.saturating_sub(6) as usize;
    let unsaved = app.unsaved_count();
    let names: Vec<&str> = app.dirty_sections().iter().map(|s| s.label()).collect();
    let what = if unsaved == 1 {
        format!("1 change on {}", names.join(", "))
    } else {
        format!("{} changes on {}", unsaved, names.join(", "))
    };
    let save = if super::app::edits_daemon_config() {
        "Save, restart voxtype, quit"
    } else {
        "Save and quit"
    };
    let mut lines = vec![Line::from("")];
    for l in common::wrap_words(&what, inner_width) {
        lines.push(Line::from(Span::styled(l, theme::warn())));
    }
    lines.push(Line::from(""));
    lines.push(prompt_option("Enter", save, inner_width));
    lines.push(prompt_option("d", "Discard them and quit", inner_width));
    lines.push(prompt_option("Esc", "Keep editing", inner_width));
    lines.push(Line::from(""));
    let rect = centered(area, width, lines.len() as u16 + 2);
    f.render_widget(Clear, rect);
    let block = common::pane("Unsaved changes", true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let inner = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    f.render_widget(Paragraph::new(lines), inner);
}

fn prompt_option(key: &str, desc: &str, width: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<7}", key), theme::key()),
        Span::raw(truncate(desc, width.saturating_sub(7))),
    ])
}

fn help_lines() -> Vec<Line<'static>> {
    let group = |title: &str| Line::from(Span::styled(title.to_string(), theme::heading()));
    let entry = |keys: &str, desc: &str| {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<13}", keys), theme::key()),
            Span::raw(desc.to_string()),
        ])
    };
    let mut lines = vec![
        group("Pages"),
        entry("↑↓ j k", "Move"),
        entry("Enter → l", "Open the page"),
        entry("Tab", "Switch between the list and the page"),
        entry("q Esc", "Quit (asks first if anything is unsaved)"),
        Line::from(""),
        group("Settings"),
        entry("↑↓ j k", "Move"),
        entry("←→ h l Space", "Change"),
        entry("Enter i", "Type a value, or change one that can't be typed"),
        entry("PgUp PgDn", "Scroll the explanation"),
        entry("Esc", "Back to the list"),
        Line::from(""),
        group("Saving"),
        entry("s", "Save every change and restart voxtype to use them"),
        entry("r", "Put this page back to what is saved"),
        Line::from(""),
        group("Typing a value"),
        entry("Enter", "Done"),
        entry("Esc Ctrl-C", "Cancel"),
        entry("Ctrl-W", "Delete the previous word"),
        entry("Ctrl-U", "Clear"),
        Line::from(""),
        group("Text page"),
        entry("Enter", "Edit a replacement, or add one"),
        entry("d Delete", "Remove the replacement under the cursor"),
        Line::from(""),
        group("Marks"),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<13}", "●"), theme::warn()),
            Span::raw("Changed, not saved yet"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<13}", "!"), theme::warn()),
            Span::raw("Something to fix; Enter fixes it"),
        ]),
        Line::from(""),
        group("What each page holds"),
    ];
    for section in Section::ALL {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<19}", section.label()), theme::text()),
            Span::styled(section.summary().to_string(), theme::muted()),
        ]));
    }
    lines
}

pub fn render_help_overlay(f: &mut Frame, app: &App) {
    let area = f.area();
    let lines = help_lines();
    let width = 76.min(area.width.saturating_sub(4));
    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let rect = centered(area, width, height);
    f.render_widget(Clear, rect);

    let mut block = common::pane("Keys", true);
    let inner = block.inner(rect);
    let max_scroll = (lines.len() as u16).saturating_sub(inner.height);
    let scroll = app.help_scroll.get().min(max_scroll);
    app.help_scroll.set(scroll);
    if max_scroll > 0 {
        let hint = if scroll < max_scroll {
            " ↓ more "
        } else {
            " ↑ back "
        };
        block = block.title_bottom(Line::from(Span::styled(hint, theme::muted())).right_aligned());
    }
    f.render_widget(block, rect);
    let inner = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_fits_the_overlay_width_at_80_columns() {
        for line in help_lines() {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(str_width(&text) <= 72, "{text:?}");
        }
    }
}
