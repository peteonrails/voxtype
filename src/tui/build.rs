//! Build page: which voxtype build the `voxtype` command runs.
//!
//! Package installs ship several builds of the same version: Whisper or the
//! ONNX engines (Parakeet and the rest), each for the processor or for a
//! graphics card. This page lists the ones on disk, says which is in use
//! and which suits this machine, and switches with Enter.

use crate::setup::binary::{Acceleration, EngineFamily, InstallKind, Inventory, Variant};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, str_width, truncate};
use super::theme;

/// The builds worth listing, in display order.
const ORDER: &[Variant] = &[
    Variant::WhisperAvx2,
    Variant::WhisperAvx512,
    Variant::WhisperVulkan,
    Variant::WhisperBaseline,
    Variant::OnnxAvx2,
    Variant::OnnxAvx512,
    Variant::OnnxCuda12,
    Variant::OnnxCuda13,
    Variant::OnnxCuda,
    Variant::OnnxMigraphx,
];

/// Which engines a build runs, in words.
pub fn engines_name(family: EngineFamily) -> &'static str {
    match family {
        EngineFamily::Whisper => "Whisper",
        EngineFamily::Onnx => "Parakeet and others",
    }
}

/// What a build runs on, in words.
pub fn hardware_name(v: Variant) -> &'static str {
    match v {
        Variant::WhisperBaseline => "Older CPUs",
        Variant::WhisperAvx2 | Variant::OnnxAvx2 => "CPU",
        Variant::WhisperAvx512 | Variant::OnnxAvx512 => "CPU with AVX-512",
        Variant::WhisperVulkan => "GPU (Vulkan)",
        Variant::OnnxCuda12 => "NVIDIA GPU (CUDA 12)",
        Variant::OnnxCuda13 => "NVIDIA GPU (CUDA 13)",
        Variant::OnnxCuda => "NVIDIA GPU (CUDA)",
        Variant::OnnxMigraphx => "AMD GPU (MIGraphX)",
        Variant::WhisperNative | Variant::OnnxNative => "This machine",
    }
}

/// A build's name in messages: "Whisper on GPU (Vulkan)".
pub fn variant_name(v: Variant) -> String {
    format!("{} on {}", engines_name(v.family()), hardware_name(v))
}

/// The builds shown: every one installed, plus a recommended one that
/// isn't, so the better choice is never invisible.
pub fn listed(inv: &Inventory) -> Vec<Variant> {
    ORDER
        .iter()
        .copied()
        .filter(|v| {
            let installed = inv
                .variants
                .iter()
                .any(|s| s.variant == *v && (s.installed || s.active));
            installed || *v == inv.recommendation.whisper || *v == inv.recommendation.onnx
        })
        .collect()
}

pub fn initial_cursor(inv: &Inventory) -> usize {
    let list = listed(inv);
    inv.active_variant
        .and_then(|a| list.iter().position(|v| *v == a))
        .unwrap_or(0)
}

/// Why a build can't be switched to, in words; `None` when it can.
fn blocker(app: &App, v: Variant) -> Option<&'static str> {
    let Some(s) = app.inventory.variants.iter().find(|s| s.variant == v) else {
        return Some("not installed");
    };
    if s.active {
        Some("in use")
    } else if !s.installed {
        Some("not installed")
    } else if !s.runs_on_this_cpu {
        Some("needs a newer CPU")
    } else if !s.gpu_available {
        Some(match v.acceleration() {
            Acceleration::Cuda => "no NVIDIA GPU found",
            Acceleration::Migraphx => "no AMD GPU found",
            _ => "no GPU found",
        })
    } else {
        None
    }
}

fn is_recommended(app: &App, v: Variant) -> bool {
    let r = &app.inventory.recommendation;
    v == r.whisper || v == r.onnx
}

fn summary(v: Variant) -> &'static str {
    match v {
        Variant::WhisperAvx2 => "Whisper on the processor. Works on any CPU from the last decade.",
        Variant::WhisperAvx512 => {
            "Whisper on the processor, faster on CPUs with AVX-512 (Intel Ice Lake, AMD Zen 4 and later)."
        }
        Variant::WhisperVulkan => {
            "Whisper on the graphics card through Vulkan. Works with NVIDIA, AMD and Intel GPUs."
        }
        Variant::WhisperBaseline => "Whisper for older processors without AVX2. The slowest choice.",
        Variant::OnnxAvx2 => {
            "Parakeet, Moonshine, SenseVoice, Paraformer, Dolphin, Omnilingual and Cohere, on the processor."
        }
        Variant::OnnxAvx512 => {
            "Parakeet and the other ONNX engines on the processor, a little faster with AVX-512."
        }
        Variant::OnnxCuda12 | Variant::OnnxCuda13 | Variant::OnnxCuda => {
            "Parakeet and the other ONNX engines on an NVIDIA graphics card. Pick the CUDA version nvidia-smi reports."
        }
        Variant::OnnxMigraphx => {
            "Parakeet and the other ONNX engines on an AMD graphics card through MIGraphX."
        }
        Variant::WhisperNative | Variant::OnnxNative => "A build made on this machine.",
    }
}

fn render_source_build(f: &mut Frame, area: Rect, app: &App) {
    let block = common::pane("Build", app.content_focused());
    let inner = block.inner(area);
    f.render_widget(block, area);
    let features = if app.inventory.compiled_features.is_empty() {
        "Whisper only".to_string()
    } else {
        app.inventory.compiled_features.join(", ")
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("Built from source with  ", theme::muted()),
            Span::raw(features),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "There is no other build to switch to. To run another engine, rebuild with it, \
             for example: cargo build --release --features parakeet",
            theme::muted(),
        )),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    app.set_page_message(None);
    if app.inventory.install_kind == InstallKind::Source {
        render_source_build(f, area, app);
        return;
    }
    let focused = app.content_focused();
    let block = common::pane("Build", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let list = listed(&app.inventory);
    let width = inner.width as usize;
    let engines_w = list
        .iter()
        .map(|v| str_width(engines_name(v.family())))
        .max()
        .unwrap_or(0);
    let hardware_w = list
        .iter()
        .map(|v| str_width(hardware_name(*v)))
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line> = Vec::new();
    for (i, v) in list.iter().enumerate() {
        let row_focused = focused && i == app.build_cursor;
        let label_style = if row_focused {
            theme::accent().add_modifier(Modifier::BOLD)
        } else {
            theme::text()
        };
        let mut spans = vec![
            Span::styled(
                if row_focused { "› " } else { "  " },
                theme::accent().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<w$}", engines_name(v.family()), w = engines_w),
                label_style,
            ),
            Span::raw("   "),
            Span::styled(
                format!("{:<w$}", hardware_name(*v), w = hardware_w),
                label_style,
            ),
            Span::raw("   "),
        ];
        let used = 2 + engines_w + 3 + hardware_w + 3;
        let room = width.saturating_sub(used);
        // Words when they fit, else the glyph alone; the explanation below
        // always says it in full.
        let (full, glyph, style) = match blocker(app, *v) {
            Some("in use") => (
                "● in use".to_string(),
                "●",
                theme::accent().add_modifier(Modifier::BOLD),
            ),
            why if is_recommended(app, *v) => (
                match why {
                    Some(why) => format!("★ recommended, {}", why),
                    None => "★ recommended".to_string(),
                },
                "★",
                theme::info(),
            ),
            Some(why) => (why.to_string(), "·", theme::muted()),
            None => (String::new(), "", theme::muted()),
        };
        let shown = if str_width(&full) <= room {
            full
        } else {
            truncate(glyph, room)
        };
        spans.push(Span::styled(shown, style));
        lines.push(Line::from(spans));
    }

    let rows_height = (lines.len() as u16).min(inner.height);
    f.render_widget(
        Paragraph::new(lines),
        Rect {
            height: rows_height,
            ..inner
        },
    );

    let Some(v) = list.get(app.build_cursor).copied() else {
        return;
    };
    let mut about: Vec<Line> = vec![Line::from(summary(v))];
    let rec = &app.inventory.recommendation;
    if v == rec.whisper || v == rec.onnx {
        let reason = if v == rec.whisper {
            rec.whisper_reason
        } else {
            rec.onnx_reason
        };
        about.push(Line::from(Span::styled(
            format!("★ Recommended for this machine: {}", reason),
            theme::info(),
        )));
    }
    about.push(Line::from(match blocker(app, v) {
        None => "Enter switches to it. It asks for your password, then restarts voxtype.",
        Some("in use") => "● This is the build voxtype runs now.",
        Some("not installed") => {
            "Not installed. Install the voxtype package for your distribution to get every build."
        }
        Some("needs a newer CPU") => "It needs a newer CPU than this one.",
        Some(_) => "It needs a graphics card this machine doesn't have.",
    }));
    let desc = Rect {
        x: inner.x + 2,
        y: inner.y + rows_height + 1,
        width: inner.width.min(76).saturating_sub(2),
        height: inner.height.saturating_sub(rows_height + 1),
    };
    if desc.height > 0 {
        f.render_widget(
            Paragraph::new(about)
                .style(theme::muted())
                .wrap(Wrap { trim: true }),
            desc,
        );
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let len = listed(&app.inventory).len();
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.build_cursor = app.build_cursor.saturating_sub(1);
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.build_cursor = (app.build_cursor + 1).min(len.saturating_sub(1));
            Action::None
        }
        KeyCode::Enter => match selected_switchable(app) {
            Some(v) => Action::SwitchVariant(v),
            None => Action::None,
        },
        _ => Action::None,
    }
}

/// The build under the cursor, when switching to it can work.
pub fn selected_switchable(app: &App) -> Option<Variant> {
    if app.inventory.install_kind == InstallKind::Source {
        return None;
    }
    let v = listed(&app.inventory).get(app.build_cursor).copied()?;
    blocker(app, v).is_none().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_plain_words() {
        assert_eq!(
            variant_name(Variant::WhisperVulkan),
            "Whisper on GPU (Vulkan)"
        );
        assert_eq!(
            variant_name(Variant::OnnxCuda12),
            "Parakeet and others on NVIDIA GPU (CUDA 12)"
        );
    }

    #[test]
    fn every_listed_build_has_a_name_and_summary() {
        for v in ORDER {
            assert!(!hardware_name(*v).is_empty());
            assert!(summary(*v).ends_with('.'));
        }
    }
}
