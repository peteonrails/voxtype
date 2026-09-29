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
/// System-wide package directory populated by distro packages; user paths
/// are searched first so a user copy always shadows a shipped package.
const SYSTEM_PACKAGE_DIR: &str = "/usr/share/voxtype/osd";

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
    let path = runtime_style_path();
    write_style_file(&path, &style_json(style)?)?;
    Ok(path)
}

/// Path of the runtime JSON consumed by Quickshell.
pub fn runtime_style_path() -> PathBuf {
    runtime_dir().join("quickshell-style.json")
}

/// Serialize a resolved style to the JSON written for Quickshell.
pub fn style_json(style: &RuntimeOsdStyle) -> Result<String, VoxtypeError> {
    serde_json::to_string_pretty(style).map_err(|e| {
        VoxtypeError::Config(format!("Failed to serialize Quickshell OSD style: {}", e))
    })
}

/// Rewrite the runtime JSON only when `style` serializes differently from
/// `last_json`; updates `last_json` and reports whether a write happened.
pub fn rewrite_runtime_style_if_changed(
    path: &Path,
    style: &RuntimeOsdStyle,
    last_json: &mut String,
) -> Result<bool, VoxtypeError> {
    let json = style_json(style)?;
    if json == *last_json {
        return Ok(false);
    }
    write_style_file(path, &json)?;
    *last_json = json;
    Ok(true)
}

/// Start live Omarchy theme following for a resolved style.
///
/// Returns `None` when the palette source is not Omarchy (a pinned package
/// or custom palette never gets a watcher) or when no Omarchy install is
/// present.
pub fn follow_omarchy_theme<F>(
    style: &RuntimeOsdStyle,
    on_change: F,
) -> Option<theme::ThemeWatchHandle>
where
    F: FnMut() + Send + 'static,
{
    if style.palette != OsdPaletteSource::Omarchy {
        return None;
    }
    theme::watch_current_theme(on_change)
}

/// Atomic write: the Quickshell FileView reloads this file on change, so a
/// reader must never observe a partially written JSON. Write to a sibling
/// temp file and rename it into place. Creates the parent directory when
/// missing (XDG_RUNTIME_DIR contents can be cleaned under a live follower).
fn write_style_file(path: &Path, json: &str) -> Result<(), VoxtypeError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| {
            VoxtypeError::Config(format!(
                "Failed to create OSD runtime directory {}: {}",
                dir.display(),
                e
            ))
        })?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp.{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let write_err = |e: std::io::Error| {
        VoxtypeError::Config(format!(
            "Failed to write Quickshell OSD style {}: {}",
            path.display(),
            e
        ))
    };
    fs::write(&tmp, json).map_err(write_err)?;
    fs::rename(&tmp, path).map_err(write_err)
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
    if let Some(found) = find_package_dir(&candidates) {
        return Ok(Some(found));
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
    candidate_package_dirs_with_system(name, Path::new(SYSTEM_PACKAGE_DIR))
}

fn candidate_package_dirs_with_system(name: &str, system_dir: &Path) -> Vec<PathBuf> {
    package_roots_with_system(system_dir)
        .into_iter()
        .map(|root| root.join(name))
        .collect()
}

/// Package search roots in priority order: user config, user data, then the
/// system directory, so a user copy always shadows a shipped package. Shared
/// by per-name resolution and [`list_installed_styles`] so the two can't
/// disagree about where packages live.
fn package_roots_with_system(system_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            roots.push(PathBuf::from(xdg).join("voxtype/osd"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".config/voxtype/osd"));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            roots.push(PathBuf::from(xdg).join("voxtype/osd"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local/share/voxtype/osd"));
    }
    roots.push(system_dir.to_path_buf());
    roots
}

/// An OSD style `[osd] style` can select, discovered on disk or built in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstalledStyle {
    /// The value to set `[osd] style` to.
    pub name: String,
    /// Package directory; `None` for the built-in default renderer.
    pub dir: Option<PathBuf>,
    /// The package manifest's `description`, when it sets one.
    pub description: Option<String>,
}

/// Every style installed right now: the built-in `default`, then each valid
/// package directory across the search roots in name order. A name present
/// in several roots is listed once with the highest-priority copy, matching
/// how [`resolve_runtime_style`] resolves it.
pub fn list_installed_styles() -> Vec<InstalledStyle> {
    list_installed_styles_in(&package_roots_with_system(Path::new(SYSTEM_PACKAGE_DIR)))
}

fn list_installed_styles_in(roots: &[PathBuf]) -> Vec<InstalledStyle> {
    // "default" never resolves to a package (resolve_package_dir returns the
    // built-in renderer before searching), so a package dir named "default"
    // is unreachable and deliberately not listed.
    let mut seen: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::from(["default".to_string()]);
    let mut packages: Vec<InstalledStyle> = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !is_package_dir(&dir) {
                continue;
            }
            let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !seen.insert(name.to_string()) {
                continue;
            }
            // A manifest that fails to parse would error at selection time,
            // but the package is still installed; list it without a
            // description rather than hiding it.
            let description = load_manifest(&dir)
                .ok()
                .flatten()
                .and_then(|m| m.description);
            packages.push(InstalledStyle {
                name: name.to_string(),
                dir: Some(dir),
                description,
            });
        }
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    let mut styles = vec![InstalledStyle {
        name: "default".to_string(),
        dir: None,
        description: Some("Built-in recipe renderer".to_string()),
    }];
    styles.extend(packages);
    styles
}

fn find_package_dir(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| is_package_dir(p)).cloned()
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
        assert!(
            msg.contains("/usr/share/voxtype/osd/definitely-not-installed"),
            "got: {msg}"
        );
    }

    #[test]
    fn system_package_dir_is_last_candidate_after_user_paths() {
        let system = Path::new("/fake/system/osd");
        let dirs = candidate_package_dirs_with_system("neon", system);
        assert_eq!(dirs.last(), Some(&system.join("neon")));
        let home_config = dirs::home_dir().unwrap().join(".config/voxtype/osd/neon");
        let user_idx = dirs.iter().position(|p| *p == home_config).unwrap();
        assert!(user_idx < dirs.len() - 1, "user path must precede system");

        assert_eq!(
            candidate_package_dirs("neon").last(),
            Some(&PathBuf::from("/usr/share/voxtype/osd/neon"))
        );
    }

    #[test]
    fn package_only_in_system_dir_resolves() {
        let tmp = tempdir().unwrap();
        let user = tmp.path().join("user/osd/neon");
        let system = tmp.path().join("system/osd/neon");
        fs::create_dir_all(&system).unwrap();
        fs::write(
            system.join(PACKAGE_MANIFEST),
            "name = \"neon\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        assert_eq!(find_package_dir(&[user, system.clone()]), Some(system));
    }

    #[test]
    fn list_installed_styles_dedupes_shadowed_packages() {
        let tmp = tempdir().unwrap();
        let user_root = tmp.path().join("user/osd");
        let system_root = tmp.path().join("system/osd");
        for (root, desc) in [(&user_root, "User copy"), (&system_root, "Shipped copy")] {
            let dir = root.join("neon");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join(PACKAGE_MANIFEST),
                format!("name = \"neon\"\nversion = \"1.0.0\"\ndescription = \"{desc}\"\n"),
            )
            .unwrap();
        }
        let aurora = system_root.join("aurora");
        fs::create_dir_all(&aurora).unwrap();
        fs::write(
            aurora.join(PACKAGE_MANIFEST),
            "name = \"aurora\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        // A plain directory without a manifest is not a package.
        fs::create_dir_all(system_root.join("not-a-package")).unwrap();

        let styles = list_installed_styles_in(&[user_root.clone(), system_root]);
        let names: Vec<&str> = styles.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["default", "aurora", "neon"]);

        let neon = styles.iter().find(|s| s.name == "neon").unwrap();
        assert_eq!(
            neon.dir.as_deref(),
            Some(user_root.join("neon").as_path()),
            "the user copy must shadow the system copy"
        );
        assert_eq!(neon.description.as_deref(), Some("User copy"));

        let aurora = styles.iter().find(|s| s.name == "aurora").unwrap();
        assert_eq!(aurora.description, None);
    }

    #[test]
    fn list_installed_styles_with_no_packages_is_just_default() {
        let tmp = tempdir().unwrap();
        let styles = list_installed_styles_in(&[tmp.path().join("missing/osd")]);
        assert_eq!(styles.len(), 1);
        assert_eq!(styles[0].name, "default");
        assert_eq!(styles[0].dir, None);
    }

    #[test]
    fn user_package_shadows_system_package() {
        let tmp = tempdir().unwrap();
        let user = tmp.path().join("user/osd/neon");
        let system = tmp.path().join("system/osd/neon");
        for dir in [&user, &system] {
            fs::create_dir_all(dir).unwrap();
            fs::write(
                dir.join(PACKAGE_MANIFEST),
                "name = \"neon\"\nversion = \"1.0.0\"\n",
            )
            .unwrap();
        }

        assert_eq!(find_package_dir(&[user.clone(), system]), Some(user));
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
    fn rewrite_skips_write_when_style_is_unchanged() {
        let style = resolve_runtime_style(&OsdConfig::default(), None).unwrap();
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("quickshell-style.json");

        let mut last = String::new();
        assert!(rewrite_runtime_style_if_changed(&path, &style, &mut last).unwrap());
        assert!(path.is_file());

        // Deleting the file proves the no-op: an unchanged style must not
        // recreate it.
        fs::remove_file(&path).unwrap();
        assert!(!rewrite_runtime_style_if_changed(&path, &style, &mut last).unwrap());
        assert!(!path.exists());

        let mut changed = style.clone();
        changed.margin_px += 1;
        assert!(rewrite_runtime_style_if_changed(&path, &changed, &mut last).unwrap());
        assert!(path.is_file());
        assert_eq!(fs::read_to_string(&path).unwrap(), last);
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_behind() {
        let style = resolve_runtime_style(&OsdConfig::default(), None).unwrap();
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("quickshell-style.json");
        write_style_file(&path, &style_json(&style).unwrap()).unwrap();
        let entries: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries,
            vec![std::ffi::OsString::from("quickshell-style.json")]
        );
    }

    #[test]
    fn non_omarchy_palette_gets_no_theme_watcher() {
        let mut style = resolve_runtime_style(&OsdConfig::default(), None).unwrap();
        for source in [
            OsdPaletteSource::Fallback,
            OsdPaletteSource::Custom,
            OsdPaletteSource::Package,
        ] {
            style.palette = source;
            assert!(
                follow_omarchy_theme(&style, || {}).is_none(),
                "palette {source:?} must not start a theme watcher"
            );
        }
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
