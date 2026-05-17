//! Omarchy theme integration.
//!
//! On startup, both OSD frontends read the active Omarchy theme and map it
//! to a [`Palette`] used by the renderer. The active theme lives at
//! `~/.config/omarchy/current/theme/colors.toml`, which is a TOML file with
//! a flat structure: `background`, `foreground`, `accent`, plus the ANSI
//! palette `color0`..=`color15`.
//!
//! Mapping:
//!
//! - `accent` → waveform fill
//! - `background` → window background (alpha kept from fallback)
//! - `foreground` → held-peak tick
//! - `color2` (ANSI green) → meter low zone
//! - `color3` (ANSI yellow) → meter mid zone
//! - `color1` (ANSI red) → meter high zone
//!
//! Themes whose ANSI red/green/yellow are off-spec (e.g. the "aether" theme
//! maps red to a tan) inherit the theme designer's choice — that's the
//! point of theming.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

use crate::osd::visual::{Color, Palette};

/// Canonical Omarchy "current theme" directory.
pub fn omarchy_theme_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push(".config/omarchy/current/theme");
    Some(p)
}

#[derive(Deserialize, Default)]
struct OmarchyColors {
    background: Option<String>,
    foreground: Option<String>,
    accent: Option<String>,
    color1: Option<String>,
    color2: Option<String>,
    color3: Option<String>,
}

/// Parse a `#RRGGBB` hex color into a [`Color`] with full alpha.
fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&s[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&s[4..6], 16).ok()? as f32 / 255.0;
    Some(Color::rgb(r, g, b))
}

/// Load the palette from the active Omarchy theme.
///
/// Falls back to [`Palette::fallback`] when the theme directory is missing,
/// the colors file is unreadable, or the TOML doesn't parse. Per-field
/// fallbacks apply too: a theme that only defines `accent` keeps the
/// fallback values for everything else.
pub fn load_palette() -> Palette {
    let Some(dir) = omarchy_theme_dir() else {
        return Palette::fallback();
    };
    let path = dir.join("colors.toml");
    let content = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Palette::fallback(),
    };
    let parsed: OmarchyColors = match toml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Palette::fallback(),
    };

    palette_from(parsed)
}

fn palette_from(c: OmarchyColors) -> Palette {
    let fb = Palette::fallback();
    let bg_alpha = fb.background.a;
    Palette {
        background: c
            .background
            .as_deref()
            .and_then(parse_hex)
            .map(|c| c.with_alpha(bg_alpha))
            .unwrap_or(fb.background),
        accent: c.accent.as_deref().and_then(parse_hex).unwrap_or(fb.accent),
        meter_low: c
            .color2
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fb.meter_low),
        meter_mid: c
            .color3
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fb.meter_mid),
        meter_high: c
            .color1
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fb.meter_high),
        foreground: c
            .foreground
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fb.foreground),
    }
}

/// Theme watcher: snapshots the palette at construction.
///
/// Real `notify`-based reload-on-change would let us re-render with the new
/// theme when the user switches Omarchy themes. Out of scope for this
/// commit; users can re-launch the OSD after switching.
pub struct ThemeWatcher {
    palette: Palette,
}

impl ThemeWatcher {
    pub fn new() -> Self {
        Self {
            palette: load_palette(),
        }
    }

    /// Current palette. Cheap to call every frame.
    pub fn palette(&self) -> Palette {
        self.palette
    }
}

impl Default for ThemeWatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_dir_resolves_under_home() {
        std::env::set_var("HOME", "/tmp/fakehome");
        let p = omarchy_theme_dir().unwrap();
        assert!(p.ends_with(".config/omarchy/current/theme"));
    }

    #[test]
    fn missing_theme_dir_yields_fallback() {
        std::env::set_var("HOME", "/tmp/this-dir-should-not-exist-voxtype-test");
        assert_eq!(load_palette(), Palette::fallback());
    }

    #[test]
    fn parse_hex_basic() {
        let c = parse_hex("#6E89C2").unwrap();
        assert!((c.r - 0x6E as f32 / 255.0).abs() < 1e-6);
        assert!((c.g - 0x89 as f32 / 255.0).abs() < 1e-6);
        assert!((c.b - 0xC2 as f32 / 255.0).abs() < 1e-6);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn parse_hex_no_hash_prefix() {
        let c = parse_hex("121515").unwrap();
        assert!((c.r - 0x12 as f32 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn parse_hex_rejects_short_or_invalid() {
        assert!(parse_hex("#FFF").is_none());
        assert!(parse_hex("#ZZZZZZ").is_none());
        assert!(parse_hex("").is_none());
    }

    #[test]
    fn palette_from_aether_sample() {
        // Real values from ~/.config/omarchy/themes/aether/colors.toml
        let toml_src = r##"
            accent = "#6E89C2"
            background = "#121515"
            foreground = "#FCFBF8"
            color1 = "#A48364"
            color2 = "#F8E7AE"
            color3 = "#FEE88B"
        "##;
        let c: OmarchyColors = toml::from_str(toml_src).unwrap();
        let p = palette_from(c);
        assert_eq!(p.accent, parse_hex("#6E89C2").unwrap());
        // Background keeps the fallback alpha (translucent OSD).
        let fb_alpha = Palette::fallback().background.a;
        assert!((p.background.a - fb_alpha).abs() < 1e-6);
        assert_eq!(p.meter_high, parse_hex("#A48364").unwrap());
        assert_eq!(p.meter_low, parse_hex("#F8E7AE").unwrap());
        assert_eq!(p.meter_mid, parse_hex("#FEE88B").unwrap());
    }

    #[test]
    fn palette_from_partial_inherits_fallback() {
        // Only accent defined; everything else stays as fallback.
        let toml_src = r##"accent = "#6E89C2""##;
        let c: OmarchyColors = toml::from_str(toml_src).unwrap();
        let p = palette_from(c);
        let fb = Palette::fallback();
        assert_eq!(p.accent, parse_hex("#6E89C2").unwrap());
        assert_eq!(p.background, fb.background);
        assert_eq!(p.meter_low, fb.meter_low);
    }

    #[test]
    fn watcher_uses_loaded_palette() {
        // We can't predict the user's theme here, but at minimum the watcher
        // should hold whatever load_palette() returned at construction.
        let w = ThemeWatcher::new();
        assert_eq!(w.palette(), load_palette());
    }
}
