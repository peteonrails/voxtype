//! Quickshell OSD style/package resolution.
//!
//! Rust owns TOML parsing, package discovery, and Omarchy palette mapping.
//! The Quickshell host consumes the resolved runtime JSON through
//! `VOXTYPE_OSD_STYLE_FILE` so QML never needs to parse user config files.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::VoxtypeError;
use crate::osd::config::{
    OsdConfig, OsdFrameConfig, OsdLayout, OsdPackageManifest, OsdPaletteSource, OsdPosition,
    OsdVisualConfig,
};
use crate::osd::theme;
use crate::osd::visual::{Color, Palette};

const PACKAGE_MANIFEST: &str = "voxtype-osd.toml";

/// Fully resolved style data consumed by Quickshell QML.
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeOsdStyle {
    pub style: String,
    pub palette: OsdPaletteSource,
    pub layout: OsdLayout,
    pub position: OsdPosition,
    pub margin_px: u32,
    pub top_margin: f32,
    pub package_dir: Option<PathBuf>,
    pub asset_root: Option<PathBuf>,
    pub custom_qml: Option<PathBuf>,
    pub colors: BTreeMap<String, String>,
    pub frame: OsdFrameConfig,
    pub visual: OsdVisualConfig,
}

/// Resolve an OSD style from config and optional CLI/env override.
pub fn resolve_runtime_style(
    osd: &OsdConfig,
    style_override: Option<&str>,
) -> Result<RuntimeOsdStyle, VoxtypeError> {
    let style_name = style_override
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&osd.style)
        .trim()
        .to_string();
    let package_dir = resolve_package_dir(&style_name, osd.plugin_path.as_deref())?;
    let manifest = match package_dir.as_ref() {
        Some(dir) => load_manifest(dir)?,
        None => None,
    };

    // Merge priority: explicit user config wins, then fields the manifest
    // explicitly sets, then built-in defaults. A manifest that omits a
    // field must not reset the user's choice.
    let manifest_palette = manifest.as_ref().and_then(|m| m.palette);
    let palette_source = osd
        .palette
        .or(manifest_palette)
        .unwrap_or(OsdPaletteSource::Omarchy);
    let layout = manifest
        .as_ref()
        .and_then(|m| m.layout)
        .unwrap_or(osd.layout);
    let frame = manifest
        .as_ref()
        .and_then(|m| m.frame.clone())
        .unwrap_or_else(|| osd.frame.clone());
    let visual = manifest
        .as_ref()
        .and_then(|m| m.visual.clone())
        .unwrap_or_else(|| osd.visual.clone());
    let custom_qml = manifest
        .as_ref()
        .and_then(|m| m.qml_entry.as_ref())
        .and_then(|entry| package_dir.as_ref().map(|dir| dir.join(entry)));
    if let Some(qml) = custom_qml.as_ref() {
        if !qml.is_file() {
            return Err(VoxtypeError::Config(format!(
                "OSD package qml_entry not found: {}\n  Fix qml_entry in {} or remove it to use the built-in renderer.",
                qml.display(),
                package_dir
                    .as_ref()
                    .map(|d| d.join(PACKAGE_MANIFEST).display().to_string())
                    .unwrap_or_else(|| PACKAGE_MANIFEST.to_string()),
            )));
        }
    }
    let asset_root = package_dir.as_ref().map(|dir| dir.join("assets"));
    let mut colors = colors_for_palette(palette_source);
    if palette_source == OsdPaletteSource::Package {
        if let Some(manifest) = manifest.as_ref() {
            colors.extend(manifest.colors.clone());
        }
    }

    Ok(RuntimeOsdStyle {
        style: style_name,
        palette: palette_source,
        layout,
        position: osd.position,
        margin_px: osd.margin_px,
        top_margin: osd.top_margin,
        package_dir,
        asset_root,
        custom_qml,
        colors,
        frame,
        visual,
    })
}

/// Write the runtime JSON consumed by Quickshell and return its path.
pub fn write_runtime_style(style: &RuntimeOsdStyle) -> Result<PathBuf, VoxtypeError> {
    let dir = runtime_dir();
    fs::create_dir_all(&dir).map_err(|e| {
        VoxtypeError::Config(format!(
            "Failed to create OSD runtime directory {}: {}",
            dir.display(),
            e
        ))
    })?;
    let path = dir.join("quickshell-style.json");
    let json = serde_json::to_string_pretty(style).map_err(|e| {
        VoxtypeError::Config(format!("Failed to serialize Quickshell OSD style: {}", e))
    })?;
    fs::write(&path, json).map_err(|e| {
        VoxtypeError::Config(format!(
            "Failed to write Quickshell OSD style {}: {}",
            path.display(),
            e
        ))
    })?;
    Ok(path)
}

fn runtime_dir() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join("voxtype")
}

fn resolve_package_dir(
    style: &str,
    plugin_path: Option<&Path>,
) -> Result<Option<PathBuf>, VoxtypeError> {
    if let Some(path) = plugin_path {
        let path = expand_tilde(path);
        if is_package_dir(&path) {
            return Ok(Some(path));
        }
        return Err(VoxtypeError::Config(format!(
            "[osd] plugin_path {} is not an OSD package directory (no {} found).\n  Point plugin_path at a directory containing {}, or remove it from config.toml.",
            path.display(),
            PACKAGE_MANIFEST,
            PACKAGE_MANIFEST,
        )));
    }
    if style == "default" || style.is_empty() {
        return Ok(None);
    }
    let direct = expand_tilde(Path::new(style));
    if is_package_dir(&direct) {
        return Ok(Some(direct));
    }
    let candidates = candidate_package_dirs(style);
    if let Some(found) = candidates.iter().find(|p| is_package_dir(p)) {
        return Ok(Some(found.clone()));
    }
    let mut searched: Vec<String> = vec![direct.display().to_string()];
    searched.extend(candidates.iter().map(|p| p.display().to_string()));
    Err(VoxtypeError::Config(format!(
        "OSD style '{}' not found. Searched:\n    {}\n  Install the style package in one of those directories, or set [osd] style = \"default\".",
        style,
        searched.join("\n    "),
    )))
}

/// Expand a leading `~` component to the user's home directory so config
/// values like `plugin_path = "~/.config/voxtype/osd/my-style"` work.
fn expand_tilde(path: &Path) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix("~") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    path.to_path_buf()
}

fn is_package_dir(path: &Path) -> bool {
    path.is_dir() && path.join(PACKAGE_MANIFEST).is_file()
}

fn candidate_package_dirs(name: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            dirs.push(PathBuf::from(xdg).join("voxtype/osd").join(name));
        }
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".config/voxtype/osd").join(name));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            dirs.push(PathBuf::from(xdg).join("voxtype/osd").join(name));
        }
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/voxtype/osd").join(name));
    }
    dirs
}

fn load_manifest(dir: &Path) -> Result<Option<OsdPackageManifest>, VoxtypeError> {
    let path = dir.join(PACKAGE_MANIFEST);
    if !path.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| VoxtypeError::Config(format!("Failed to read {}: {}", path.display(), e)))?;
    toml::from_str::<OsdPackageManifest>(&content)
        .map(Some)
        .map_err(|e| VoxtypeError::Config(format!("Invalid {}: {}", path.display(), e)))
}

fn colors_for_palette(source: OsdPaletteSource) -> BTreeMap<String, String> {
    let palette = match source {
        OsdPaletteSource::Fallback | OsdPaletteSource::Custom | OsdPaletteSource::Package => {
            Palette::fallback()
        }
        OsdPaletteSource::Omarchy => theme::load_palette(),
    };
    semantic_colors(palette)
}

fn semantic_colors(p: Palette) -> BTreeMap<String, String> {
    let mut colors = BTreeMap::new();
    colors.insert("accent".to_string(), color_to_css(p.accent));
    colors.insert("background".to_string(), color_to_css(p.background));
    colors.insert("surface".to_string(), color_to_css(p.background));
    colors.insert("foreground".to_string(), color_to_css(p.foreground));
    colors.insert(
        "muted".to_string(),
        color_to_css(p.foreground.with_alpha(0.65)),
    );
    colors.insert("success".to_string(), color_to_css(p.meter_low));
    colors.insert("warning".to_string(), color_to_css(p.meter_mid));
    colors.insert("error".to_string(), color_to_css(p.meter_high));
    colors.insert("recording".to_string(), color_to_css(p.meter_high));
    colors.insert("streaming".to_string(), color_to_css(p.accent));
    colors.insert("transcribing".to_string(), color_to_css(p.meter_mid));
    colors.insert(
        "idle".to_string(),
        color_to_css(p.foreground.with_alpha(0.75)),
    );
    colors
}

fn color_to_css(c: Color) -> String {
    let r = (c.r.clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (c.g.clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (c.b.clamp(0.0, 1.0) * 255.0).round() as u8;
    if c.a >= 0.999 {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("rgba({r}, {g}, {b}, {:.3})", c.a.clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osd::config::{OsdLayerKind, OsdPaletteSource};
    use tempfile::tempdir;

    #[test]
    fn default_style_has_no_package_and_uses_omarchy_palette() {
        let style = resolve_runtime_style(&OsdConfig::default(), None).unwrap();
        assert_eq!(style.style, "default");
        assert_eq!(style.palette, OsdPaletteSource::Omarchy);
        assert!(style.package_dir.is_none());
        assert!(style.custom_qml.is_none());
        assert!(style.colors.contains_key("accent"));
    }

    #[test]
    fn explicit_package_manifest_can_supply_qml_and_visual() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join(PACKAGE_MANIFEST),
            r#"
                name = "bars-plus"
                version = "1.0.0"
                palette = "fallback"
                layout = "wide"
                qml_entry = "CustomOsd.qml"

                [frame]
                background = "none"
                border = "accent"

                [[visual.layers]]
                type = "bars"
                source = "rms"
                color = "accent"
                order = 7
            "#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("CustomOsd.qml"),
            "import QtQuick\nItem {}\n",
        )
        .unwrap();

        let cfg = OsdConfig {
            plugin_path: Some(tmp.path().to_path_buf()),
            ..OsdConfig::default()
        };
        let style = resolve_runtime_style(&cfg, None).unwrap();
        assert_eq!(style.palette, OsdPaletteSource::Fallback);
        assert_eq!(style.layout, OsdLayout::Wide);
        assert_eq!(style.frame.background, "none");
        assert_eq!(style.frame.border, "accent");
        assert_eq!(
            style.custom_qml.as_deref(),
            Some(tmp.path().join("CustomOsd.qml").as_path())
        );
        assert_eq!(style.visual.layers[0].kind, OsdLayerKind::Bars);
    }

    #[test]
    fn package_palette_merges_manifest_colors() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join(PACKAGE_MANIFEST),
            r##"
                name = "colored"
                version = "1.0.0"
                palette = "package"

                [colors]
                accent = "#123456"
                background = "rgba(1, 2, 3, 0.5)"
            "##,
        )
        .unwrap();

        let cfg = OsdConfig {
            plugin_path: Some(tmp.path().to_path_buf()),
            ..OsdConfig::default()
        };
        let style = resolve_runtime_style(&cfg, None).unwrap();
        assert_eq!(style.palette, OsdPaletteSource::Package);
        assert_eq!(
            style.colors.get("accent").map(String::as_str),
            Some("#123456")
        );
        assert_eq!(
            style.colors.get("background").map(String::as_str),
            Some("rgba(1, 2, 3, 0.5)")
        );
        assert!(style.colors.contains_key("foreground"));
    }

    #[test]
    fn explicit_omarchy_palette_overrides_package_manifest() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join(PACKAGE_MANIFEST),
            r##"
                name = "colored"
                version = "1.0.0"
                palette = "package"

                [colors]
                accent = "#123456"
            "##,
        )
        .unwrap();

        let mut cfg = OsdConfig {
            plugin_path: Some(tmp.path().to_path_buf()),
            palette: Some(OsdPaletteSource::Omarchy),
            ..OsdConfig::default()
        };
        let style = resolve_runtime_style(&cfg, None).unwrap();
        assert_eq!(style.palette, OsdPaletteSource::Omarchy);
        assert_ne!(
            style.colors.get("accent").map(String::as_str),
            Some("#123456")
        );

        cfg.palette = None;
        let style = resolve_runtime_style(&cfg, None).unwrap();
        assert_eq!(style.palette, OsdPaletteSource::Package);
        assert_eq!(
            style.colors.get("accent").map(String::as_str),
            Some("#123456")
        );
    }

    #[test]
    fn aegis_hud_example_package_resolves_to_custom_qml() {
        let package_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/osd-packages/aegis-hud");
        let qml_entry = package_dir.join("AegisHud.qml");

        assert!(package_dir.join(PACKAGE_MANIFEST).is_file());
        assert!(qml_entry.is_file());

        let style_name = package_dir.to_string_lossy().to_string();
        let style = resolve_runtime_style(&OsdConfig::default(), Some(&style_name)).unwrap();

        assert_eq!(style.palette, OsdPaletteSource::Package);
        assert_eq!(style.layout, OsdLayout::Custom);
        assert_eq!(style.package_dir.as_deref(), Some(package_dir.as_path()));
        assert_eq!(style.custom_qml.as_deref(), Some(qml_entry.as_path()));
        assert_eq!(
            style.colors.get("accent").map(String::as_str),
            Some("#38D8FF")
        );
        assert_eq!(
            style.colors.get("background").map(String::as_str),
            Some("rgba(3, 11, 18, 0.72)")
        );
        assert_eq!(style.frame.background, "none");
        assert_eq!(style.frame.border, "none");
    }

    #[test]
    fn minimal_manifest_preserves_user_layout_frame_and_visual() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join(PACKAGE_MANIFEST),
            r##"
                name = "colors-only"
                version = "1.0.0"

                [colors]
                accent = "#123456"
            "##,
        )
        .unwrap();

        let mut user_visual = OsdVisualConfig::default();
        user_visual.layers.truncate(1);
        let cfg = OsdConfig {
            plugin_path: Some(tmp.path().to_path_buf()),
            layout: OsdLayout::Orb,
            frame: crate::osd::config::OsdFrameConfig {
                background: "none".to_string(),
                ..Default::default()
            },
            visual: user_visual,
            ..OsdConfig::default()
        };
        let style = resolve_runtime_style(&cfg, None).unwrap();
        assert_eq!(style.layout, OsdLayout::Orb);
        assert_eq!(style.frame.background, "none");
        assert_eq!(style.visual.layers.len(), 1);
    }

    #[test]
    fn unknown_style_name_is_an_error_not_a_silent_fallback() {
        let cfg = OsdConfig {
            style: "definitely-not-installed".to_string(),
            ..OsdConfig::default()
        };
        let err = resolve_runtime_style(&cfg, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("definitely-not-installed"), "got: {msg}");
        assert!(msg.contains("Searched"), "got: {msg}");
    }

    #[test]
    fn invalid_plugin_path_is_an_error() {
        let tmp = tempdir().unwrap();
        let cfg = OsdConfig {
            plugin_path: Some(tmp.path().join("nope")),
            ..OsdConfig::default()
        };
        let err = resolve_runtime_style(&cfg, None).unwrap_err();
        assert!(err.to_string().contains(PACKAGE_MANIFEST));
    }

    #[test]
    fn missing_qml_entry_is_an_error() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join(PACKAGE_MANIFEST),
            r#"
                name = "broken"
                version = "1.0.0"
                qml_entry = "Missing.qml"
            "#,
        )
        .unwrap();

        let cfg = OsdConfig {
            plugin_path: Some(tmp.path().to_path_buf()),
            ..OsdConfig::default()
        };
        let err = resolve_runtime_style(&cfg, None).unwrap_err();
        assert!(err.to_string().contains("Missing.qml"));
    }

    #[test]
    fn expand_tilde_resolves_home() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde(Path::new("~/x/y")), home.join("x/y"));
        assert_eq!(expand_tilde(Path::new("/abs/x")), PathBuf::from("/abs/x"));
    }

    #[test]
    fn css_color_serialization_preserves_alpha() {
        assert_eq!(color_to_css(Color::rgb(1.0, 0.0, 0.5)), "#FF0080");
        assert_eq!(
            color_to_css(Color::rgba(0.1, 0.2, 0.3, 0.5)),
            "rgba(26, 51, 77, 0.500)"
        );
    }
}
