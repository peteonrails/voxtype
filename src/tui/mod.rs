//! Terminal UI for `voxtype configure`.
//!
//! Renders an interactive view over voxtype settings. PR 2 covers the General
//! section (variant picker + daemon status); subsequent PRs add Hotkey, Audio,
//! Output, etc.

mod app;
mod general;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};
use std::time::Duration;

use app::{Action, App};

type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn run() -> anyhow::Result<()> {
    let mut terminal = enter_terminal()?;
    let result = event_loop(&mut terminal);
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

fn event_loop(terminal: &mut Tui) -> anyhow::Result<()> {
    let mut app = App::new();

    loop {
        terminal.draw(|f| general::render(f, &app))?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            // crossterm fires both Press and Release on some terminals; ignore
            // releases to keep nav single-step.
            if !matches!(
                key.kind,
                crossterm::event::KeyEventKind::Press | crossterm::event::KeyEventKind::Repeat
            ) {
                continue;
            }
            match general::handle_key(&mut app, key) {
                Action::Quit => return Ok(()),
                Action::SwitchVariant(variant) => {
                    // Drop out of the alternate screen so pkexec can prompt.
                    leave_terminal(terminal)?;
                    let outcome = run_pkexec_switch(variant);
                    *terminal = enter_terminal()?;
                    terminal.clear()?;
                    app.record_switch_attempt(variant, outcome);
                }
                Action::None => {}
            }
        }
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
