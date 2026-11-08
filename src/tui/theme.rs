//! Semantic styles for the configuration TUI.
//!
//! Every style is built from the terminal's own palette: the default
//! foreground and background, the sixteen ANSI colors, and text attributes.
//! No RGB values and no hard-coded white or black, so an Omarchy (or any
//! other) terminal theme recolors the whole interface, light or dark.
//! Muted text uses the dim attribute rather than ANSI "bright black", which
//! some themes set to the background color itself.
//!
//! Color is never the only signal: selection is reversed video, unsaved rows
//! carry a `●`, and status lines lead with `✓`, `✗` or `!`.

use ratatui::style::{Color, Modifier, Style};

/// Body text: the terminal's own foreground.
pub fn text() -> Style {
    Style::default()
}

/// Supporting text: labels, descriptions, hints, inactive chrome.
pub fn muted() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// Headings inside panes.
pub fn heading() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// The one accent: focus, active pane borders, key names, current section.
pub fn accent() -> Style {
    Style::default().fg(Color::Green)
}

/// Informational highlight, such as a recommendation.
pub fn info() -> Style {
    Style::default().fg(Color::Cyan)
}

/// Unsaved edits and warnings.
pub fn warn() -> Style {
    Style::default().fg(Color::Yellow)
}

/// Errors and failures.
pub fn error() -> Style {
    Style::default().fg(Color::Red)
}

/// Success confirmations.
pub fn ok() -> Style {
    Style::default().fg(Color::Green)
}

/// The focused row or cell: reversed video works on every palette.
pub fn selected() -> Style {
    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

/// Border of the pane that has keyboard focus.
pub fn border_focused() -> Style {
    accent()
}

/// Border of every other pane.
pub fn border() -> Style {
    muted()
}

/// Pane titles sit on the border; the focused pane's title is bold.
pub fn pane_title(focused: bool) -> Style {
    if focused {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

/// A key name in the footer and help, such as `s` or `Enter`.
pub fn key() -> Style {
    accent().add_modifier(Modifier::BOLD)
}
