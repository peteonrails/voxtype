//! The small subset of Omarchy Shell's popup tokens used by the shell layout.
//!
//! Keep this independent of Omarchy's QML modules: a standalone voxtype shell
//! cannot import `qs.Commons` from another Quickshell configuration. Hyprland
//! rounding is read live by QML; TOML and color aliases are resolved here.

use std::collections::BTreeMap;
use std::fs;

use serde::Serialize;

use super::theme;

/// Resolved shell chrome, consumed only by the opt-in `shell` layout.
#[derive(Debug, Clone, Serialize)]
pub struct ShellStyle {
    pub text: String,
    pub border_colors: Vec<String>,
    pub border_angle: f32,
    pub border_alpha: f32,
    pub border_width: f32,
    pub spacing_scale: f32,
    pub font_size: f32,
}

/// Load theme tokens with per-user shell overrides and palette fallbacks.
pub fn load(colors: &BTreeMap<String, String>) -> ShellStyle {
    let mut values = theme::omarchy_theme_dirs()
        .unwrap_or_default()
        .into_iter()
        .find_map(|dir| read_shell(dir.join("shell.toml")))
        .unwrap_or_default();
    if let Some(user) =
        dirs::home_dir().and_then(|home| read_shell(home.join(".config/omarchy/shell.toml")))
    {
        // Omarchy's user shell.toml overlays individual theme tokens, not
        // entire sections (a font override must not erase other font keys).
        for (section, value) in user {
            if let (Some(base), Some(overrides)) = (
                values.get_mut(&section).and_then(toml::Value::as_table_mut),
                value.as_table(),
            ) {
                base.extend(overrides.clone());
            } else {
                values.insert(section, value);
            }
        }
    }
    resolve(&values, colors)
}

fn read_shell(path: std::path::PathBuf) -> Option<toml::Table> {
    toml::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn value<'a>(values: &'a toml::Table, key: &str) -> Option<&'a toml::Value> {
    let (section, name) = key.split_once('.')?;
    values.get(section)?.get(name)
}

fn number(values: &toml::Table, key: &str, fallback: f32) -> f32 {
    value(values, key)
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|v| v as f64)))
        .filter(|v| v.is_finite() && *v >= 0.0 && *v <= f32::MAX as f64)
        .map(|v| v as f32)
        .unwrap_or(fallback)
}

// Shell border tokens can alias another surface or a Hyprland gradient.
// Bound recursion so a malformed theme cannot cycle indefinitely.
fn expand(values: &toml::Table, token: &str, depth: usize) -> Option<String> {
    if depth > 8 {
        return None;
    }
    match value(values, token).and_then(toml::Value::as_str) {
        Some(next) => expand(values, next.trim(), depth + 1),
        None => Some(token.to_string()),
    }
}

fn color(token: &str, colors: &BTreeMap<String, String>) -> Option<String> {
    let token = match token {
        "text" => "foreground",
        "urgent" => "error",
        "transparent" => return Some("#00000000".to_string()),
        other => other,
    };
    if let Some(c) = colors.get(token) {
        return Some(c.clone());
    }
    // Hyprland uses RRGGBBAA; Qt uses AARRGGBB. Shell hex colors use the
    // same trailing-alpha convention as Hyprland.
    let hex = token
        .strip_prefix('#')
        .or_else(|| token.strip_prefix("rgba(")?.strip_suffix(')'))
        .or_else(|| token.strip_prefix("rgb(")?.strip_suffix(')'))?;
    if !hex.is_ascii() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => Some(format!("#{hex}")),
        8 => Some(format!("#{}{}", &hex[6..], &hex[..6])),
        _ => None,
    }
}

fn resolve(values: &toml::Table, colors: &BTreeMap<String, String>) -> ShellStyle {
    let foreground = colors["foreground"].clone();
    let accent = colors["accent"].clone();
    let text = value(values, "popups.text")
        .and_then(toml::Value::as_str)
        .and_then(|v| expand(values, v, 0))
        .and_then(|v| color(&v, colors))
        .unwrap_or(foreground);
    let border = value(values, "popups.border")
        .and_then(toml::Value::as_str)
        .and_then(|v| expand(values, v, 0))
        .unwrap_or_else(|| "accent".to_string());
    let mut border_angle = 0.0;
    let mut border_colors = Vec::new();
    for part in border.split_whitespace() {
        if let Some(angle) = part.strip_suffix("deg").and_then(|v| v.parse::<f32>().ok()) {
            if angle.is_finite() {
                border_angle = angle.rem_euclid(360.0);
            }
        } else if let Some(c) = color(part, colors) {
            border_colors.push(c);
        }
    }
    if border_colors.is_empty() {
        border_colors.push(accent);
    }
    let font_size = number(values, "font.base-size", 12.0).max(1.0);
    let font_scale = font_size / 12.0;
    let scale_with_font = value(values, "spacing.scale-with-font")
        .and_then(toml::Value::as_bool)
        .unwrap_or(true);
    let spacing_scale =
        number(values, "spacing.scale", 1.0) * if scale_with_font { font_scale } else { 1.0 };
    ShellStyle {
        text,
        border_colors,
        border_angle,
        border_alpha: number(values, "popups.border-alpha", 1.0).clamp(0.0, 1.0),
        border_width: number(
            values,
            "popups.border-width",
            (2.0 * spacing_scale).round().max(1.0),
        ),
        spacing_scale,
        font_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("accent".into(), "#739FD2".into()),
            ("foreground".into(), "#E8DAC7".into()),
        ])
    }

    #[test]
    fn missing_shell_tokens_use_palette_and_shell_defaults() {
        let shell = resolve(&toml::Table::new(), &colors());
        assert_eq!(shell.text, "#E8DAC7");
        assert_eq!(shell.border_colors, ["#739FD2"]);
        assert_eq!(shell.border_width, 2.0);
        assert_eq!(shell.font_size, 12.0);
        assert_eq!(shell.spacing_scale, 1.0);
    }

    #[test]
    fn resolves_popup_gradient_alias_and_converts_hyprland_alpha() {
        let values = toml::from_str(
            r##"
            [hyprland]
            active-border = "rgba(739fd2ff) rgba(fa40a980) 45deg"
            [popups]
            border = "hyprland.active-border"
            border-width = 2
            border-alpha = 0.88
            text = "foreground"
            "##,
        )
        .unwrap();
        let shell = resolve(&values, &colors());
        assert_eq!(shell.border_colors, ["#ff739fd2", "#80fa40a9"]);
        assert_eq!(shell.border_angle, 45.0);
        assert_eq!(shell.border_alpha, 0.88);
        assert_eq!(shell.text, "#E8DAC7");
    }

    #[test]
    fn zero_border_and_font_scaled_spacing_are_preserved() {
        let values = toml::from_str(
            "[font]\nbase-size = 18\n[spacing]\nscale = 1.2\n[popups]\nborder-width = 0\n",
        )
        .unwrap();
        let shell = resolve(&values, &colors());
        assert_eq!(shell.border_width, 0.0);
        assert!((shell.spacing_scale - 1.8).abs() < 0.001);
    }

    #[test]
    fn malformed_tokens_and_alias_cycles_fall_back() {
        let values = toml::from_str(
            r#"
            [popups]
            border = "popups.border"
            text = "not-a-color"
            border-width = -2
            border-alpha = nan
            [font]
            base-size = inf
            "#,
        )
        .unwrap();
        let shell = resolve(&values, &colors());
        assert_eq!(shell.border_colors, ["#739FD2"]);
        assert_eq!(shell.text, "#E8DAC7");
        assert_eq!(shell.border_width, 2.0);
        assert_eq!(shell.border_alpha, 1.0);
        assert_eq!(shell.font_size, 12.0);
    }
}
