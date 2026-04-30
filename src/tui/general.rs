//! General settings screen: install info, daemon status, variant matrix.

use crate::setup::binary::{Acceleration, EngineFamily, InstallKind, Variant};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App, COLS, ROWS};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(banner_height(app)),
            Constraint::Length(8), // install/daemon info
            Constraint::Min(8),    // variant matrix
            Constraint::Length(2), // legend
            Constraint::Length(1), // section help
        ])
        .split(area);

    render_banner(f, chunks[0], app);
    render_info(f, chunks[1], app);

    // Side-by-side: variant matrix on the left, hint pane on the right.
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(chunks[2]);
    render_matrix(f, body[0], app);
    render_hint(f, body[1], app);

    render_legend(f, chunks[3]);
    render_help(f, chunks[4]);
}

fn banner_height(app: &App) -> u16 {
    let any = app.last_switch.is_some() || app.restart_needed || app.missing_model.is_some();
    if any {
        4
    } else {
        0
    }
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

    if let Some(missing) = &app.missing_model {
        lines.push(Line::from(Span::styled(
            format!(
                "⚠ Active {} model not downloaded: {}",
                missing.engine, missing.model
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            format!("  Run `{}` to fetch it.", missing.setup_command),
            Style::default().fg(Color::Gray),
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

    let rec = &inv.recommendation;
    let recommended = format!(
        "{}  /  {}",
        rec.whisper.display(),
        rec.onnx.display()
    );

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
        Line::from(vec![
            Span::raw("Recommended:   "),
            Span::styled("★ ", Style::default().fg(Color::Cyan)),
            Span::styled(recommended, Style::default().fg(Color::Cyan)),
            Span::styled("   (Whisper / ONNX)", Style::default().fg(Color::Gray)),
        ]),
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

    if is_recommended(variant, &app.inventory.recommendation) {
        format!("★ {}", glyph)
    } else {
        glyph.to_string()
    }
}

fn is_recommended(v: Variant, r: &crate::setup::binary::Recommendation) -> bool {
    v == r.whisper || v == r.onnx
}

fn render_hint(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("About");
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.inventory.install_kind == InstallKind::Source {
        return;
    }

    let (r, c) = app.cursor;
    let lines: Vec<Line> = match app.variant_at(r, c) {
        Some(variant) => variant_hint_lines(variant, app),
        None => na_hint_lines(r, c),
    };

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn variant_hint_lines<'a>(variant: Variant, app: &App) -> Vec<Line<'a>> {
    let hint = variant_hint(variant);
    let status = app.inventory.variants.iter().find(|s| s.variant == variant);

    let status_line = match status {
        Some(s) if s.active => Line::from(Span::styled(
            "● Currently active",
            Style::default().fg(Color::Green),
        )),
        Some(s) if !s.installed => Line::from(Span::styled(
            "· Not installed on this system",
            Style::default().fg(Color::Gray),
        )),
        Some(s) if !s.runs_on_this_cpu => Line::from(Span::styled(
            "⚠ Won't run: CPU lacks required instructions",
            Style::default().fg(Color::Yellow),
        )),
        Some(s) if !s.gpu_available => Line::from(Span::styled(
            "⚠ Won't accelerate: required GPU not detected",
            Style::default().fg(Color::Yellow),
        )),
        Some(_) => Line::from(Span::styled(
            "✓ Ready to switch (Enter)",
            Style::default().fg(Color::Cyan),
        )),
        None => Line::from(""),
    };

    let rec = &app.inventory.recommendation;
    let mut lines: Vec<Line> = Vec::new();
    if variant == rec.whisper {
        lines.push(Line::from(Span::styled(
            "★ Recommended for Whisper on this hardware",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            rec.whisper_reason.to_string(),
            Style::default().fg(Color::Cyan),
        )));
        lines.push(Line::from(""));
    }
    if variant == rec.onnx {
        lines.push(Line::from(Span::styled(
            "★ Recommended for ONNX engines on this hardware",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            rec.onnx_reason.to_string(),
            Style::default().fg(Color::Cyan),
        )));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        hint.headline.to_string(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for paragraph in hint.body {
        lines.push(Line::from(paragraph.to_string()));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(vec![
        Span::styled("Models:    ", Style::default().fg(Color::Gray)),
        Span::raw(hint.models.to_string()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Speed:     ", Style::default().fg(Color::Gray)),
        Span::raw(hint.speed.to_string()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Hardware:  ", Style::default().fg(Color::Gray)),
        Span::raw(hint.hardware.to_string()),
    ]));

    // Only show concrete model picks on the recommended cells, where the user
    // is most likely to act on them. On non-recommended cells the static
    // `models:` line above is enough.
    if variant == rec.whisper || variant == rec.onnx {
        let models = recommended_models(variant);
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Recommended models",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("  English:   ", Style::default().fg(Color::Gray)),
            Span::raw(models.english.to_string()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  European:  ", Style::default().fg(Color::Gray)),
            Span::raw(models.european.to_string()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Asian:     ", Style::default().fg(Color::Gray)),
            Span::raw(models.asian.to_string()),
        ]));
        if let Some(note) = models.note {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                note.to_string(),
                Style::default().fg(Color::Gray),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(status_line);
    lines
}

fn na_hint_lines<'a>(row: usize, col: usize) -> Vec<Line<'a>> {
    let family = ROWS.get(row).copied();
    let accel = COLS.get(col).copied();
    let suggestion = match (family, accel) {
        (Some(EngineFamily::Whisper), Some(Acceleration::Cuda)) => {
            "For NVIDIA GPU acceleration with Whisper, use Vulkan — it covers \
             NVIDIA, AMD, and Intel GPUs in a single binary."
        }
        (Some(EngineFamily::Whisper), Some(Acceleration::Migraphx)) => {
            "For AMD GPU acceleration with Whisper, use Vulkan — voxtype's \
             whisper.cpp build uses Vulkan instead of ROCm."
        }
        (Some(EngineFamily::Onnx), Some(Acceleration::Vulkan)) => {
            "ONNX Runtime does not ship a Vulkan execution provider. Use \
             ONNX (CUDA) for NVIDIA, ONNX (MIGraphX) for AMD, or ONNX (AVX2/AVX-512) \
             for CPU."
        }
        _ => "This combination is not built.",
    };
    vec![
        Line::from(Span::styled(
            "Not applicable",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(suggestion.to_string()),
    ]
}

struct VariantHint {
    headline: &'static str,
    body: &'static [&'static str],
    models: &'static str,
    speed: &'static str,
    hardware: &'static str,
}

/// Recommended models for a given variant, broken out by language family.
/// Shown in the hint pane only when the cursor is on a *recommended* cell —
/// the goal is to give users a jumping-off point ("if I switch to this
/// binary, what should I download?") rather than a complete model catalog.
struct ModelRecommendations {
    english: &'static str,
    european: &'static str,
    asian: &'static str,
    /// Optional advice tied to the acceleration tier (e.g. prefer int8 on AVX2).
    note: Option<&'static str>,
}

fn recommended_models(v: Variant) -> ModelRecommendations {
    match v {
        // ---- Whisper family ----
        Variant::WhisperAvx2 | Variant::WhisperNative => ModelRecommendations {
            english: "small.en  (or base.en for low-power CPUs)",
            european: "small  (covers FR, DE, IT, ES, NL, PL, PT and more)",
            asian: "medium  (CJK accuracy improves a lot at medium+)",
            note: Some(
                "AVX2-only CPU: large-v3 will run but isn't realtime. Stick to \
                 small/medium unless you have a GPU.",
            ),
        },
        Variant::WhisperAvx512 => ModelRecommendations {
            english: "large-v3-turbo  (fast and accurate)",
            european: "large-v3-turbo  (strong on most EU languages)",
            asian: "large-v3  (better CJK than turbo for the same size)",
            note: Some(
                "AVX-512 makes large-v3-turbo practical on CPU; large-v3 is \
                 slower but more accurate on non-English.",
            ),
        },
        Variant::WhisperVulkan => ModelRecommendations {
            english: "large-v3-turbo",
            european: "large-v3-turbo  (FR, DE, IT, ES, NL, PL, PT, etc.)",
            asian: "large-v3  (CJK; turbo is slightly weaker on Asian languages)",
            note: Some(
                "GPU acceleration removes the size penalty; pick whichever \
                 model gives you the accuracy you need.",
            ),
        },

        // ---- ONNX family ----
        Variant::OnnxAvx2 | Variant::OnnxNative => ModelRecommendations {
            english: "parakeet-tdt-0.6b-v3-int8  (quantized; ~50% faster on CPU)",
            european: "dolphin-base  (multi-language CTC, dictation-tuned)",
            asian: "sensevoice-small  (zh, en, ja, ko, yue in one model)",
            note: Some(
                "On AVX2-only CPUs the int8 Parakeet variant is the practical \
                 default. Omnilingual is also viable but heavier; Cohere is \
                 the heaviest at ~3 GB but ranks #1 on the Open ASR \
                 Leaderboard.",
            ),
        },
        Variant::OnnxAvx512 => ModelRecommendations {
            english: "parakeet-tdt-0.6b-v3  (top of the Open ASR Leaderboard)",
            european: "omnilingual-300m  (1600 languages, including all of EU)",
            asian: "sensevoice-small  (zh, en, ja, ko, yue)",
            note: Some(
                "AVX-512 lets you run full-precision Parakeet at real-time \
                 speed without a GPU.",
            ),
        },
        Variant::OnnxCuda12 | Variant::OnnxCuda13 | Variant::OnnxCuda => ModelRecommendations {
            english: "parakeet-tdt-0.6b-v3",
            european: "omnilingual-300m  (1600 languages)",
            asian: "sensevoice-small  (zh/en/ja/ko/yue) or paraformer-zh for Chinese-only",
            note: Some(
                "CUDA inference is so fast on Parakeet that English dictation \
                 is effectively instantaneous; use the largest model that fits \
                 your VRAM.",
            ),
        },
        Variant::OnnxMigraphx => ModelRecommendations {
            english: "parakeet-tdt-0.6b-v3",
            european: "omnilingual-300m  (1600 languages)",
            asian: "sensevoice-small  (zh, en, ja, ko, yue)",
            note: Some(
                "MIGraphX execution provider is new and may not register on all \
                 driver versions; if you see ORT registration errors, fall back \
                 to ONNX (AVX-512) on CPU.",
            ),
        },
    }
}

fn variant_hint(v: Variant) -> VariantHint {
    match v {
        Variant::WhisperAvx2 => VariantHint {
            headline: "Whisper on AVX2 CPUs",
            body: &[
                "Baseline Whisper build. Runs on any x86-64 CPU since ~2013 \
                 (Haswell/Excavator and newer). Pick this if your CPU lacks \
                 AVX-512 and you don't have a GPU worth using.",
            ],
            models: "tiny, base, small, medium, large-v3, large-v3-turbo (and .en variants)",
            speed: "Real-time on small/base; large-v3 is slow without a GPU",
            hardware: "Any x86-64 CPU with AVX2",
        },
        Variant::WhisperAvx512 => VariantHint {
            headline: "Whisper on AVX-512 CPUs",
            body: &[
                "Fastest CPU-only Whisper. Roughly 1.5-2x throughput over the \
                 AVX2 build on supported chips. Use this if you don't have a \
                 capable GPU but do have a recent Intel or AMD CPU.",
            ],
            models: "Same as AVX2; large-v3-turbo becomes practical for live use",
            speed: "Best CPU performance; ~1.5-2x AVX2",
            hardware: "Intel Tiger/Ice Lake+, AMD Zen 4+",
        },
        Variant::WhisperVulkan => VariantHint {
            headline: "Whisper with Vulkan GPU",
            body: &[
                "Vendor-agnostic GPU acceleration via Vulkan compute shaders. \
                 Works on NVIDIA, AMD, and Intel GPUs (including integrated \
                 graphics that support Vulkan).",
                "Best general-purpose pick for desktops and gaming laptops.",
            ],
            models: "All Whisper models; large-v3-turbo runs comfortably",
            speed: "5-10x CPU on a discrete GPU; falls back to CPU if Vulkan unavailable",
            hardware: "Any Vulkan 1.2 GPU; ~2 GB VRAM for large-v3",
        },
        Variant::WhisperNative => VariantHint {
            headline: "Whisper (source build)",
            body: &[
                "A locally compiled Whisper binary with whatever Cargo features \
                 you enabled. Reported when no specific tier suffix matches.",
            ],
            models: "Whatever your build supports",
            speed: "Depends on build flags (RUSTFLAGS, GPU features)",
            hardware: "Whatever you compiled for",
        },
        Variant::OnnxAvx2 => VariantHint {
            headline: "ONNX engines on AVX2 CPUs",
            body: &[
                "CPU inference for the ONNX Runtime engine family: Parakeet, \
                 Moonshine, SenseVoice, Paraformer, Dolphin, Omnilingual, and \
                 Cohere Transcribe.",
                "Pick this when you don't have a GPU but want a faster, more \
                 accurate alternative to Whisper.",
            ],
            models: "parakeet-tdt-0.6b-v3, moonshine-base/tiny, sense-voice-small, paraformer-zh, dolphin-base, omnilingual",
            speed: "Parakeet is ~2-3x faster than Whisper-large at higher accuracy",
            hardware: "Any x86-64 CPU with AVX2",
        },
        Variant::OnnxAvx512 => VariantHint {
            headline: "ONNX engines on AVX-512 CPUs",
            body: &[
                "Same engine set as ONNX (AVX2), built against a newer toolchain \
                 that takes advantage of AVX-512 where ONNX Runtime can use it.",
            ],
            models: "Same as ONNX (AVX2)",
            speed: "Modest gain over AVX2 build; ORT does runtime SIMD dispatch",
            hardware: "Intel Tiger/Ice Lake+, AMD Zen 4+",
        },
        Variant::OnnxCuda12 | Variant::OnnxCuda13 | Variant::OnnxCuda => VariantHint {
            headline: "ONNX engines on NVIDIA CUDA",
            body: &[
                "GPU inference via the CUDA execution provider. Best choice for \
                 anyone with a recent NVIDIA card running Parakeet or another \
                 ONNX engine.",
                "voxtype ships separate cu12 and cu13 binaries; pick the one \
                 matching your installed CUDA runtime. The unversioned variant \
                 is for source builds and pre-0.7.0 installs.",
                "Note: this binary bundles an ONNX Runtime built with AVX-512, \
                 so the CPU also needs AVX-512 to load it cleanly.",
            ],
            models: "Same as ONNX (AVX2)",
            speed: "10-20x AVX2 on capable GPUs; Parakeet faster than real-time even at large sizes",
            hardware: "NVIDIA GPU + matching CUDA 12.x or 13.x driver; AVX-512 CPU",
        },
        Variant::OnnxMigraphx => VariantHint {
            headline: "ONNX engines on AMD MIGraphX",
            body: &[
                "GPU inference for AMD GPUs via the MIGraphX execution provider \
                 (replaces the ROCm EP that was retired in voxtype 0.7.0).",
                "Note: this binary bundles an ONNX Runtime built with AVX-512, \
                 so the CPU also needs AVX-512 to load it cleanly. MIGraphX \
                 support is new — if the provider fails to register on your \
                 driver/card combo, fall back to ONNX (AVX-512) on CPU or \
                 switch the engine to Whisper (Vulkan).",
            ],
            models: "Same as ONNX (AVX2)",
            speed: "Comparable to CUDA on similarly-tier GPUs",
            hardware: "AMD GPU with MIGraphX-capable driver; AVX-512 CPU",
        },
        Variant::OnnxNative => VariantHint {
            headline: "ONNX engines (source build)",
            body: &[
                "Locally compiled ONNX engine binary with whatever Cargo \
                 features you enabled. Reported when no specific tier suffix \
                 matches.",
            ],
            models: "Whatever your build supports",
            speed: "Depends on build flags",
            hardware: "Whatever you compiled for",
        },
    }
}

fn render_legend(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled("★ recommended   ", Style::default().fg(Color::Cyan)),
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
        " ↑↓←→ navigate matrix   Enter switch   r refresh ",
        Style::default().fg(Color::Gray),
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
        Acceleration::Migraphx => "MIGraphX",
        Acceleration::Native => "native",
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    match key.code {
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
