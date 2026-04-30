//! Terminal UI for `voxtype configure`.
//!
//! Renders an interactive view over voxtype settings. The General section
//! (variant picker + daemon status) is functional today; remaining sections
//! ship as placeholders and will be filled in over subsequent PRs.

mod advanced_section;
mod app;
mod audio;
mod common;
mod compositor_bindings;
mod config_editor;
mod engine;
mod general;
mod hotkey;
mod meeting_section;
mod notifications_section;
mod output_section;
mod section;
mod sidebar;
mod text_section;
mod vad_section;
mod waybar_section;

#[allow(unused_imports)]
pub(crate) use config_editor::{ConfigEditor, EditorError};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
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
    let mut last_general_refresh = std::time::Instant::now();
    let general_refresh_interval = Duration::from_secs(2);

    loop {
        terminal.draw(|f| draw(f, &app))?;

        if !event::poll(Duration::from_millis(250))? {
            // Idle tick. Refresh the General-screen state (daemon status,
            // active variant, inventory) so the green/red dot stays current
            // without the user pressing `r`.
            if app.current_section == Section::General
                && last_general_refresh.elapsed() >= general_refresh_interval
            {
                app.refresh_inventory();
                last_general_refresh = std::time::Instant::now();
            }
            continue;
        }
        match event::read()? {
            Event::Key(key) => {
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
            Event::Mouse(mouse) => {
                handle_mouse(&mut app, mouse);
            }
            _ => {}
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
    // While the active section is inline-editing a text field, swallow
    // global shortcuts so the user can type 'q', press Esc, etc. into the
    // input. The section's handle_key gets the key instead.
    if app.is_editing() {
        return None;
    }

    // Help overlay: any key dismisses it (including ?).
    if app.help_open {
        app.help_open = false;
        return Some(Action::None);
    }
    if matches!(key.code, KeyCode::Char('?')) {
        app.help_open = true;
        return Some(Action::None);
    }

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

fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    // Ignore mouse input while help overlay is open or a text field is editing.
    if app.help_open || app.is_editing() {
        return;
    }
    if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        return;
    }

    let col = mouse.column;
    let row = mouse.row;

    // Title bar occupies row 0; sidebar inner rows begin at absolute row 1.
    if col < sidebar::WIDTH && row >= 1 {
        let idx = (row - 1) as usize;
        if idx < Section::ALL.len() {
            app.sidebar_cursor = idx;
            app.open_hovered_section();
            app.focus_sidebar();
        }
        return;
    }

    if col >= sidebar::WIDTH {
        app.focus_content();
    }
}

fn handle_section_key(app: &mut App, key: KeyEvent) -> Action {
    match app.current_section {
        Section::General => general::handle_key(app, key),
        Section::Hotkey => hotkey::handle_key(app, key),
        Section::Audio => audio::handle_key(app, key),
        Section::Engine => engine::handle_key(app, key),
        Section::Output => output_section::handle_key(app, key),
        Section::Text => text_section::handle_key(app, key),
        Section::Vad => vad_section::handle_key(app, key),
        Section::Meeting => meeting_section::handle_key(app, key),
        Section::Notifications => notifications_section::handle_key(app, key),
        Section::Waybar => waybar_section::handle_key(app, key),
        Section::Advanced => advanced_section::handle_key(app, key),
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

    if app.help_open {
        render_help_overlay(f);
    }
}

fn render_help_overlay(f: &mut Frame) {
    let area = f.area();
    // Centered modal: ~70% width, ~85% height, capped at 78x30.
    let w = area.width.saturating_sub(8).min(78);
    let h = area.height.saturating_sub(4).min(30);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    // Clear under the modal so it overpaints whatever's behind.
    f.render_widget(ratatui::widgets::Clear, rect);

    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Voxtype Configuration — Help ");
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let bold = Style::default().add_modifier(ratatui::style::Modifier::BOLD);
    let dim = Style::default().fg(Color::Gray);

    let lines = vec![
        Line::from(Span::styled("Global", bold)),
        Line::from("  Tab          Toggle focus between sidebar and section"),
        Line::from("  Esc          Sidebar focus / quit from sidebar"),
        Line::from("  q, Ctrl-C    Quit"),
        Line::from("  ?            Toggle this help"),
        Line::from(""),
        Line::from(Span::styled("Sidebar", bold)),
        Line::from("  ↑↓ / jk      Navigate sections"),
        Line::from("  Enter, →, l  Open section / focus content"),
        Line::from(""),
        Line::from(Span::styled("Section forms", bold)),
        Line::from("  ↑↓ / jk      Navigate fields"),
        Line::from("  ←→ / hl      Cycle field value"),
        Line::from("  Space        Toggle / advance"),
        Line::from("  Enter, i     Edit text field"),
        Line::from("  s            Save changes to config.toml"),
        Line::from("  r            Revert unsaved changes"),
        Line::from(""),
        Line::from(Span::styled("Inline text editing", bold)),
        Line::from("  type         Insert at cursor"),
        Line::from("  ←→           Move cursor"),
        Line::from("  Home / End   Beginning / end of line"),
        Line::from("  Backspace    Delete previous char"),
        Line::from("  Delete       Delete next char"),
        Line::from("  Ctrl-W       Delete previous word"),
        Line::from("  Ctrl-U       Clear line"),
        Line::from("  Enter        Commit"),
        Line::from("  Esc, Ctrl-C  Cancel"),
        Line::from(""),
        Line::from(Span::styled("Press any key to dismiss.", dim)),
    ];
    f.render_widget(Paragraph::new(lines), inner);
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
    let line = if app.sidebar_focused {
        // Show the highlighted section's summary alongside the keymap so the
        // user sees what each section covers without opening it.
        let summary = Section::ALL
            .get(app.sidebar_cursor)
            .map(|s| s.summary())
            .unwrap_or("");
        Line::from(vec![
            Span::styled(
                " ↑↓  Enter open  Tab content  ? help  q quit  ",
                Style::default().fg(Color::Gray),
            ),
            Span::styled(
                format!("│  {}", summary),
                Style::default().fg(Color::Cyan),
            ),
        ])
    } else {
        Line::from(Span::styled(
            " Tab / Esc back to sidebar   ? help   q quit ",
            Style::default().fg(Color::Gray),
        ))
    };
    f.render_widget(Paragraph::new(line), area);
}

fn render_section(f: &mut Frame, area: Rect, app: &App) {
    match app.current_section {
        Section::General => general::render(f, area, app),
        Section::Hotkey => hotkey::render(f, area, app),
        Section::Audio => audio::render(f, area, app),
        Section::Engine => engine::render(f, area, app),
        Section::Output => output_section::render(f, area, app),
        Section::Text => text_section::render(f, area, app),
        Section::Vad => vad_section::render(f, area, app),
        Section::Meeting => meeting_section::render(f, area, app),
        Section::Notifications => notifications_section::render(f, area, app),
        Section::Waybar => waybar_section::render(f, area, app),
        Section::Advanced => advanced_section::render(f, area, app),
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
