//! Terminal UI for `voxtype configure`.
//!
//! The page list on the left, the open page on the right, one status line
//! and a footer of keys. `s` saves every change and restarts voxtype so it
//! uses them. See `docs/TUI_DESIGN.md` for the design this follows.

mod advanced_section;
mod app;
mod audio;
mod basics;
mod build;
mod chrome;
mod common;
mod compositor_bindings;
mod config_editor;
mod engine;
mod hotkey;
mod meeting_section;
mod notifications_section;
mod osd_section;
mod output_section;
mod section;
mod sidebar;
mod text_section;
mod theme;
mod vad_section;
mod waybar_section;

// Exported beyond the crate so the binary's `voxtype config` handlers write
// through the same atomic-save-and-validate path the TUI uses, rather than
// growing a second config writer.
#[allow(unused_imports)]
pub use config_editor::{ConfigEditor, EditorError, OverlayChange};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    Frame, Terminal,
};
use std::io::{self, Stdout};
use std::time::Duration;

use app::{Action, App};
use section::Section;

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Entry point for the hidden `voxtype configure --probe-audio-devices`
/// flag: print detected input devices and exit. See
/// `audio::start_device_scan` for why this runs in its own process.
pub fn probe_audio_devices() {
    audio::print_input_devices();
}

/// Launch the TUI.
///
/// `config_path` is the `-c/--config` value; when set, every page reads and
/// writes that file instead of the default. Installed before the first page
/// loads, because a page that has already read the default would save back
/// to it (#595).
pub fn run(
    force_package_mode: bool,
    config_path: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    config_editor::set_tui_config_path(config_path);
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
    let mut last_refresh = std::time::Instant::now();

    loop {
        app.poll_background();
        terminal.draw(|f| draw(f, &app))?;

        if !event::poll(Duration::from_millis(250))? {
            // Keep the Build page and the Basics problem rows current while
            // they are on screen.
            if matches!(app.current_section, Section::Basics | Section::Build)
                && last_refresh.elapsed() >= Duration::from_secs(2)
            {
                app.refresh_inventory();
                last_refresh = std::time::Instant::now();
            }
            continue;
        }
        let action = match event::read()? {
            Event::Key(key) => {
                if !matches!(
                    key.kind,
                    crossterm::event::KeyEventKind::Press | crossterm::event::KeyEventKind::Repeat
                ) {
                    continue;
                }
                handle_key(&mut app, key)
            }
            Event::Mouse(mouse) => {
                let size = terminal.size()?;
                handle_mouse(&mut app, mouse, Rect::new(0, 0, size.width, size.height));
                Action::None
            }
            _ => continue,
        };
        if matches!(
            dispatch_action(terminal, &mut app, action)?,
            LoopControl::Quit
        ) {
            return Ok(());
        }
        // The last result stays up until something changes again.
        if app.unsaved_count() > 0 {
            app.notice = None;
        }
        app.capture_baselines();
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
        Action::Quit if app.unsaved_count() > 0 => {
            app.quit_pending = true;
            Ok(LoopControl::Continue)
        }
        Action::Quit | Action::ForceQuit => Ok(LoopControl::Quit),
        Action::Save => {
            save_and_apply(terminal, app)?;
            Ok(LoopControl::Continue)
        }
        Action::SaveAndQuit => {
            if save_and_apply(terminal, app)? {
                Ok(LoopControl::Quit)
            } else {
                Ok(LoopControl::Continue)
            }
        }
        Action::None => Ok(LoopControl::Continue),
        other => {
            run_followups(terminal, app, vec![other])?;
            Ok(LoopControl::Continue)
        }
    }
}

/// `s`: save every page, do what the saves need (download a model, switch
/// build), then restart voxtype so it uses the new settings.
fn save_and_apply(terminal: &mut Tui, app: &mut App) -> anyhow::Result<bool> {
    let (failed, followups) = app.save_all();
    // A page that failed keeps its edits and shows why on its status line.
    if let Some(section) = failed.first() {
        app.jump_to_section(*section);
        return Ok(false);
    }
    if followups.is_empty() {
        app.apply_saved();
    } else {
        run_followups(terminal, app, followups)?;
    }
    Ok(true)
}

/// Downloads and build switches run in the plain terminal so their
/// progress and password prompt are visible, then voxtype restarts.
fn run_followups(terminal: &mut Tui, app: &mut App, actions: Vec<Action>) -> anyhow::Result<()> {
    leave_terminal(terminal)?;
    let mut ok = true;
    for action in actions {
        match action {
            Action::SwitchVariant(variant) => {
                let outcome = run_pkexec_switch(variant);
                ok &= outcome.is_ok();
                app.record_switch_attempt(variant, outcome);
            }
            Action::DownloadModel { engine, model } => {
                let outcome = run_setup_model(&engine, &model);
                ok &= outcome.is_ok();
                app.record_download_attempt(&model, outcome);
            }
            Action::SwitchVariantAndDownload {
                variant,
                engine,
                model,
            } => {
                // The download writes to the models directory and needs no
                // switch, so it runs even when the switch fails.
                let switched = run_pkexec_switch(variant);
                let downloaded = run_setup_model(&engine, &model);
                ok &= switched.is_ok() && downloaded.is_ok();
                app.record_switch_attempt(variant, switched);
                if ok {
                    app.record_download_attempt(&model, downloaded);
                }
            }
            _ => {}
        }
    }
    *terminal = enter_terminal()?;
    terminal.clear()?;
    if ok {
        app.apply_saved();
    }
    Ok(())
}

/// Download a model in the plain terminal so its progress shows. Runs the
/// `voxtype` on PATH, which is the installed build the daemon uses; this
/// TUI may be a build without that engine.
fn run_setup_model(engine: &str, model: &str) -> Result<(), String> {
    let _ = engine;
    let status = std::process::Command::new("voxtype")
        .args(["setup", "--download", "--model", model])
        .status()
        .map_err(|e| format!("couldn't run voxtype setup --download ({})", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "the download stopped. Run: voxtype setup --download --model {}",
            model
        ))
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
        .map_err(|e| format!("couldn't run pkexec ({}); is polkit installed?", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "pkexec didn't finish. Run: sudo voxtype setup variant --to {}",
            variant.binary_name()
        ))
    }
}

fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    // While a value is being typed, every key goes to it.
    if app.is_editing() {
        return handle_section_key(app, key);
    }
    if app.quit_pending {
        app.quit_pending = false;
        return match key.code {
            // Quits only once everything saved; a failure stays open on
            // the page that failed.
            KeyCode::Enter | KeyCode::Char('s') | KeyCode::Char('y') => Action::SaveAndQuit,
            KeyCode::Char('d') | KeyCode::Char('D') => Action::ForceQuit,
            _ => Action::None,
        };
    }
    if app.help_open {
        let scroll = app.help_scroll.get();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => app.help_scroll.set(scroll.saturating_add(1)),
            KeyCode::Up | KeyCode::Char('k') => app.help_scroll.set(scroll.saturating_sub(1)),
            KeyCode::PageDown | KeyCode::Char(' ') => {
                app.help_scroll.set(scroll.saturating_add(10))
            }
            KeyCode::PageUp => app.help_scroll.set(scroll.saturating_sub(10)),
            _ => app.help_open = false,
        }
        return Action::None;
    }

    match (key.code, key.modifiers) {
        (KeyCode::Char('?'), _) => {
            app.help_open = true;
            app.help_scroll.set(0);
            Action::None
        }
        (KeyCode::Char('q'), KeyModifiers::NONE) => Action::Quit,
        (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => Action::Quit,
        (KeyCode::Char('s'), KeyModifiers::NONE) => {
            if app.unsaved_count() > 0 {
                Action::Save
            } else {
                Action::None
            }
        }
        (KeyCode::PageDown, _) => {
            let s = app.guidance_scroll.get();
            app.guidance_scroll.set(s.saturating_add(3));
            Action::None
        }
        (KeyCode::PageUp, _) => {
            let s = app.guidance_scroll.get();
            app.guidance_scroll.set(s.saturating_sub(3));
            Action::None
        }
        (KeyCode::Tab, _) | (KeyCode::BackTab, _) => {
            if app.sidebar_focused {
                app.focus_content();
            } else {
                app.focus_sidebar();
            }
            Action::None
        }
        (KeyCode::Esc, _) if app.sidebar_focused => Action::Quit,
        (KeyCode::Esc, _) => {
            app.focus_sidebar();
            Action::None
        }
        _ if app.sidebar_focused => handle_sidebar_key(app, key),
        _ => handle_section_key(app, key),
    }
}

fn handle_sidebar_key(app: &mut App, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_sidebar(-1);
            app.open_hovered_section();
            app.guidance_scroll.set(0);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_sidebar(1);
            app.open_hovered_section();
            app.guidance_scroll.set(0);
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            app.open_hovered_section();
            app.focus_content();
        }
        _ => {}
    }
    Action::None
}

/// Screen regions, computed once so drawing and mouse hit-testing agree.
struct ScreenLayout {
    list: Rect,
    page: Rect,
    status: Rect,
    footer: Rect,
}

/// Widest the page pane grows.
const PAGE_MAX_WIDTH: u16 = 96;

fn screen_layout(area: Rect) -> ScreenLayout {
    let [body, status, footer] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
    let [list, page] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar::WIDTH), Constraint::Min(0)])
        .areas(body);
    // A margin so the pane doesn't touch the terminal edge, and a cap so
    // settings and their values stay close together on wide terminals.
    let page = Rect {
        width: page.width.saturating_sub(1).min(PAGE_MAX_WIDTH),
        ..page
    };
    ScreenLayout {
        list,
        page,
        status,
        footer,
    }
}

fn handle_mouse(app: &mut App, mouse: MouseEvent, screen: Rect) {
    if app.help_open || app.quit_pending || app.is_editing() {
        return;
    }
    if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        return;
    }
    let layout = screen_layout(screen);
    let (col, row) = (mouse.column, mouse.row);
    let inside = |r: Rect| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height;

    if inside(layout.list) {
        if let Some(idx) = sidebar::item_at(layout.list, sidebar::focus_index(app), row) {
            app.sidebar_cursor = idx;
            app.open_hovered_section();
            app.guidance_scroll.set(0);
        }
        app.focus_sidebar();
    } else if inside(layout.page) {
        app.focus_content();
    }
}

fn handle_section_key(app: &mut App, key: KeyEvent) -> Action {
    let moving = matches!(
        key.code,
        KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k')
    );
    if !app.is_editing() && moving {
        app.guidance_scroll.set(0);
    }
    // Enter types a value; on a setting that can't be typed it changes it,
    // as → does, so Enter never does nothing.
    let advance = key.code == KeyCode::Enter && !app.is_editing();
    let before = advance.then(|| focused_value(app));
    let action = section_key(app, key);
    if advance
        && action == Action::None
        && !app.is_editing()
        && focused_value(app) == before.flatten()
    {
        return section_key(app, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    }
    action
}

/// The focused row's value on the open page, to tell whether a key did
/// anything.
fn focused_value(app: &App) -> Option<String> {
    let rows = app.section_rows(app.current_section)?;
    rows.into_iter().find(|r| r.focused).map(|r| r.value)
}

fn section_key(app: &mut App, key: KeyEvent) -> Action {
    // Pages save through `s` above; their own `s` handlers would save one
    // page without applying it.
    if !app.is_editing() && key.code == KeyCode::Char('s') {
        return Action::None;
    }
    match app.current_section {
        Section::Basics => basics::handle_key(app, key),
        Section::Build => build::handle_key(app, key),
        Section::Hotkey => hotkey::handle_key(app, key),
        Section::Audio => audio::handle_key(app, key),
        Section::Engine => engine::handle_key(app, key),
        Section::Output => output_section::handle_key(app, key),
        Section::Text => text_section::handle_key(app, key),
        Section::Vad => vad_section::handle_key(app, key),
        Section::Meeting => meeting_section::handle_key(app, key),
        Section::Notifications => notifications_section::handle_key(app, key),
        Section::Osd => osd_section::handle_key(app, key),
        Section::Waybar => waybar_section::handle_key(app, key),
        Section::Advanced => advanced_section::handle_key(app, key),
    }
}

fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < chrome::MIN_WIDTH || area.height < chrome::MIN_HEIGHT {
        chrome::render_too_small(f, area);
        return;
    }
    let layout = screen_layout(area);
    sidebar::render(f, layout.list, app);
    render_section(f, layout.page, app);
    chrome::render_status_line(f, layout.status, app);
    chrome::render_footer(f, layout.footer, app);

    if app.help_open {
        chrome::render_help_overlay(f, app);
    }
    if app.quit_pending {
        chrome::render_quit_prompt(f, app);
    }
}

fn render_section(f: &mut Frame, area: Rect, app: &App) {
    match app.current_section {
        Section::Basics => basics::render(f, area, app),
        Section::Build => build::render(f, area, app),
        Section::Hotkey => hotkey::render(f, area, app),
        Section::Audio => audio::render(f, area, app),
        Section::Engine => engine::render(f, area, app),
        Section::Output => output_section::render(f, area, app),
        Section::Text => text_section::render(f, area, app),
        Section::Vad => vad_section::render(f, area, app),
        Section::Meeting => meeting_section::render(f, area, app),
        Section::Notifications => notifications_section::render(f, area, app),
        Section::Osd => osd_section::render(f, area, app),
        Section::Waybar => waybar_section::render(f, area, app),
        Section::Advanced => advanced_section::render(f, area, app),
    }
}
