//! Terminal UI for `voxtype configure`.
//!
//! Renders an interactive view over voxtype settings. The General section
//! (variant picker + daemon status) is functional today; remaining sections
//! ship as placeholders and will be filled in over subsequent PRs.

mod app;
mod config_editor;
mod general;
mod hotkey;
mod section;
mod sidebar;
mod stub;

#[allow(unused_imports)]
pub(crate) use config_editor::{ConfigEditor, EditorError};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    backend::CrosstermBackend,
    Frame, Terminal,
};
use std::io::{self, Stdout};
use std::time::Duration;

use app::{Action, App};
use section::Section;

type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn run(force_package_mode: bool) -> anyhow::Result<()> {
    let mut terminal = enter_terminal()?;
    let result = event_loop(&mut terminal, force_package_mode);
    leave_terminal(&mut terminal)?;
    result
}

fn enter_terminal() -> anyhow::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn leave_terminal(terminal: &mut Tui) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn event_loop(terminal: &mut Tui, force_package_mode: bool) -> anyhow::Result<()> {
    let mut app = App::new(force_package_mode);

    loop {
        terminal.draw(|f| draw(f, &app))?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if !matches!(
                key.kind,
                crossterm::event::KeyEventKind::Press | crossterm::event::KeyEventKind::Repeat
            ) {
                continue;
            }

            // Global shortcuts handled before delegating to the focused pane.
            if let Some(action) = handle_global_key(&mut app, key) {
                match dispatch_action(terminal, &mut app, action)? {
                    LoopControl::Continue => continue,
                    LoopControl::Quit => return Ok(()),
                }
            }

            let action = if app.sidebar_focused {
                handle_sidebar_key(&mut app, key)
            } else {
                handle_section_key(&mut app, key)
            };

            match dispatch_action(terminal, &mut app, action)? {
                LoopControl::Continue => {}
                LoopControl::Quit => return Ok(()),
            }
        }
    }
}

enum LoopControl {
    Continue,
    Quit,
}

fn dispatch_action(
    terminal: &mut Tui,
    app: &mut App,
    action: Action,
) -> anyhow::Result<LoopControl> {
    match action {
        Action::Quit => Ok(LoopControl::Quit),
        Action::SwitchVariant(variant) => {
            // Drop out of the alternate screen so pkexec can prompt.
            leave_terminal(terminal)?;
            let outcome = run_pkexec_switch(variant);
            *terminal = enter_terminal()?;
            terminal.clear()?;
            app.record_switch_attempt(variant, outcome);
            Ok(LoopControl::Continue)
        }
        Action::None => Ok(LoopControl::Continue),
    }
}

fn handle_global_key(app: &mut App, key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Action::Quit),
        (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
        (KeyCode::Tab, _) => {
            if app.sidebar_focused {
                app.focus_content();
            } else {
                app.focus_sidebar();
            }
            Some(Action::None)
        }
        (KeyCode::Esc, _) => {
            if !app.sidebar_focused {
                // First Esc returns focus to sidebar, second quits.
                app.focus_sidebar();
                Some(Action::None)
            } else {
                Some(Action::Quit)
            }
        }
        _ => None,
    }
}

fn handle_sidebar_key(app: &mut App, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_sidebar(-1);
            app.open_hovered_section();
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_sidebar(1);
            app.open_hovered_section();
            Action::None
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            app.open_hovered_section();
            app.focus_content();
            Action::None
        }
        _ => Action::None,
    }
}

fn handle_section_key(app: &mut App, key: KeyEvent) -> Action {
    match app.current_section {
        Section::General => general::handle_key(app, key),
        Section::Hotkey => hotkey::handle_key(app, key),
        // Stub sections accept no input today.
        _ => Action::None,
    }
}

fn draw(f: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title bar
            Constraint::Min(0),    // body (sidebar + content)
            Constraint::Length(1), // footer / help
        ])
        .split(f.area());

    render_title(f, outer[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar::WIDTH), Constraint::Min(0)])
        .split(outer[1]);

    sidebar::render(f, body[0], app);
    render_section(f, body[1], app);

    render_footer(f, outer[2], app);
}

fn render_title(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::raw(" Voxtype Configuration"),
        Span::styled(
            "  ·  ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            "edit settings without leaving the terminal",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let hint = if app.sidebar_focused {
        " ↑↓ navigate sections   Enter / → open   Tab focus content   q quit "
    } else {
        " Tab / Esc back to sidebar   q quit "
    };
    let line = Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray)));
    f.render_widget(Paragraph::new(line), area);
}

fn render_section(f: &mut Frame, area: Rect, app: &App) {
    match app.current_section {
        Section::General => general::render(f, area, app),
        Section::Hotkey => hotkey::render(f, area, app),
        other => stub::render(f, area, other),
    }
}

fn run_pkexec_switch(variant: crate::setup::binary::Variant) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {}", e))?;
    let status = std::process::Command::new("pkexec")
        .arg(exe)
        .arg("setup")
        .arg("variant")
        .arg("--to")
        .arg(variant.binary_name())
        .status()
        .map_err(|e| format!("failed to launch pkexec: {} (is polkit installed?)", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("pkexec exited with {}", status))
    }
}
