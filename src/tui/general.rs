//! General settings screen: install info, daemon status, variant matrix.

use crate::setup::binary::{Acceleration, EngineFamily, InstallKind, Variant};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App, COLS, ROWS};

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title bar
            Constraint::Length(banner_height(app)),
            Constraint::Length(7), // install/daemon info
            Constraint::Min(8),    // variant matrix
            Constraint::Length(2), // legend
            Constraint::Length(1), // help
        ])
        .split(f.area());

    render_title(f, chunks[0]);
    render_banner(f, chunks[1], app);
    render_info(f, chunks[2], app);
    render_matrix(f, chunks[3], app);
    render_legend(f, chunks[4]);
    render_help(f, chunks[5]);
}

fn banner_height(app: &App) -> u16 {
    if app.last_switch.is_some() || app.restart_needed {
        3
    } else {
        0
    }
}

fn render_title(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(
            " Voxtype Configuration ",
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  General"),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_banner(f: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let mut lines = Vec::new();

    if let Some(outcome) = &app.last_switch {
        let style = if outcome.success {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red)
        };
        let prefix = if outcome.success { "✓ " } else { "✗ " };
        lines.push(Line::from(Span::styled(
            format!("{}{}", prefix, outcome.message),
            style,
        )));
    }

    if app.restart_needed {
        lines.push(Line::from(Span::styled(
            "  Daemon restart required: systemctl --user restart voxtype",
            Style::default().fg(Color::Yellow),
        )));
    }

    let block = Block::default().borders(Borders::ALL).title("Status");
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), area);
}

fn render_info(f: &mut Frame, area: Rect, app: &App) {
    let inv = &app.inventory;
    let install_kind = match inv.install_kind {
        InstallKind::Package => "package",
        InstallKind::Source => "source",
    };

    let daemon_dot = if app.daemon_running {
        Span::styled("●", Style::default().fg(Color::Green))
    } else {
        Span::styled("●", Style::default().fg(Color::Red))
    };
    let daemon_text = if app.daemon_running {
        "running"
    } else {
        "stopped"
    };

    let active = inv
        .active_variant
        .map(|v| format!("{} ({})", v.display(), v.binary_name()))
        .unwrap_or_else(|| "unknown (symlink missing or unrecognized)".to_string());

    let lines = vec![
        Line::from(vec![
            Span::raw("Daemon:        "),
            daemon_dot,
            Span::raw(format!(" {}", daemon_text)),
        ]),
        Line::from(format!(
            "Install:       {} ({})",
            inv.binary_path.display(),
            install_kind
        )),
        Line::from(format!("Active:        {}", active)),
        Line::from(format!(
            "CPU:           AVX2={}  AVX-512={}",
            inv.cpu.avx2, inv.cpu.avx512
        )),
        Line::from(format!(
            "GPU:           NVIDIA={}  AMD={}",
            inv.gpus.nvidia, inv.gpus.amd
        )),
    ];

    let block = Block::default().borders(Borders::ALL).title("Install");
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_matrix(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("Variant");
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.inventory.install_kind == InstallKind::Source {
        let para = Paragraph::new(vec![
            Line::from("Source build detected."),
            Line::from(""),
            Line::from("Variant switching applies only to package installs"),
            Line::from("(/usr/lib/voxtype/voxtype-*). To enable a different"),
            Line::from("engine, rebuild with the appropriate Cargo features."),
            Line::from(""),
            Line::from(format!(
                "Compiled features: {}",
                if app.inventory.compiled_features.is_empty() {
                    "(none)".to_string()
                } else {
                    app.inventory.compiled_features.join(", ")
                }
            )),
        ])
        .wrap(Wrap { trim: true });
        f.render_widget(para, inner);
        return;
    }

    let mut lines = Vec::new();

    // Header row
    let mut header = vec![Span::raw(format!("{:<10}", ""))];
    for col in COLS {
        header.push(Span::styled(
            format!("{:<10}", accel_label(*col)),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(header));

    // One row per engine family
    for (r, family) in ROWS.iter().enumerate() {
        let mut spans = vec![Span::styled(
            format!("{:<10}", family_label(*family)),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        for (c, _accel) in COLS.iter().enumerate() {
            let cell = render_cell(app, r, c);
            let is_cursor = app.cursor == (r, c);
            let style = if is_cursor {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            spans.push(Span::styled(format!("{:<10}", cell), style));
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_cell(app: &App, row: usize, col: usize) -> String {
    let Some(variant) = app.variant_at(row, col) else {
        return "—".to_string();
    };

    let status = app
        .inventory
        .variants
        .iter()
        .find(|s| s.variant == variant);

    let glyph = match status {
        Some(s) if s.active => "● active",
        Some(s) if !s.installed => "·",
        Some(s) if !s.runs_on_this_cpu => "⚠ CPU",
        Some(s) if !s.gpu_available => "⚠ GPU",
        Some(_) => "✓",
        None => "·",
    };
    glyph.to_string()
}

fn render_legend(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::raw("● active   "),
        Span::raw("✓ ready   "),
        Span::raw("⚠ CPU/GPU mismatch   "),
        Span::raw("· not installed   "),
        Span::raw("— not applicable"),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_help(f: &mut Frame, area: Rect) {
    let line = Line::from(Span::styled(
        " ↑↓←→ navigate   Enter switch   r refresh   q quit ",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(line), area);
}

fn family_label(f: EngineFamily) -> &'static str {
    match f {
        EngineFamily::Whisper => "Whisper",
        EngineFamily::Onnx => "ONNX",
    }
}

fn accel_label(a: Acceleration) -> &'static str {
    match a {
        Acceleration::Avx2 => "AVX2",
        Acceleration::Avx512 => "AVX-512",
        Acceleration::Vulkan => "Vulkan",
        Acceleration::Cuda => "CUDA",
        Acceleration::Rocm => "ROCm",
        Acceleration::Native => "native",
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Char('r') => {
            app.refresh_inventory();
            Action::None
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_cursor(-1, 0);
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_cursor(1, 0);
            Action::None
        }
        KeyCode::Left | KeyCode::Char('h') => {
            app.move_cursor(0, -1);
            Action::None
        }
        KeyCode::Right | KeyCode::Char('l') => {
            app.move_cursor(0, 1);
            Action::None
        }
        KeyCode::Enter => match selected_actionable_variant(app) {
            Some(v) => Action::SwitchVariant(v),
            None => Action::None,
        },
        _ => Action::None,
    }
}

/// Variant under the cursor, but only if switching to it makes sense:
/// - exists in the matrix
/// - is installed
/// - runs on this CPU
/// - has a compatible GPU (if required)
/// - is not already active
fn selected_actionable_variant(app: &App) -> Option<Variant> {
    if app.inventory.install_kind == InstallKind::Source {
        return None;
    }
    let (r, c) = app.cursor;
    let v = app.variant_at(r, c)?;
    let status = app.inventory.variants.iter().find(|s| s.variant == v)?;
    if status.active || !status.installed || !status.runs_on_this_cpu || !status.gpu_available {
        return None;
    }
    Some(v)
}
