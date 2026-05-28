//! Install voxtype XDG icon theme entries for the system tray.
//!
//! Installs `voxtype.png` and `voxtype-recording.png` at `hicolor/128x128/apps/`
//! (the native size of the bundled PNGs) plus scalable SVGs under
//! `hicolor/scalable/apps/`. Panels that support SVG use the scalable assets;
//! panels that fall back to raster get the correctly-sized 128×128 PNG.

use std::fs;
use std::path::{Path, PathBuf};

const ICON_IDLE_PNG: &[u8] = include_bytes!("../../assets/icon.png");
const ICON_RECORDING_PNG: &[u8] = include_bytes!("../../assets/icon-recording.png");
const ICON_IDLE_SVG: &[u8] = include_bytes!("../../assets/icon.svg");
const ICON_RECORDING_SVG: &[u8] = include_bytes!("../../assets/icon-recording.svg");

/// Install tray icons to `~/.local/share/icons/hicolor/` at the native PNG
/// size (128×128) plus scalable SVGs. Skips silently if icons are already up
/// to date.
pub fn install() -> anyhow::Result<()> {
    let base = hicolor_base()?;
    let mut any_changed = false;

    // PNG — install only at the native 128×128 size per the XDG icon spec.
    let png_dir = base.join("128x128/apps");
    fs::create_dir_all(&png_dir)?;
    any_changed |= write_if_changed(&png_dir.join("voxtype.png"), ICON_IDLE_PNG)?;
    any_changed |= write_if_changed(&png_dir.join("voxtype-recording.png"), ICON_RECORDING_PNG)?;

    // Scalable SVGs
    let scalable = base.join("scalable/apps");
    fs::create_dir_all(&scalable)?;
    any_changed |= write_if_changed(&scalable.join("voxtype.svg"), ICON_IDLE_SVG)?;
    any_changed |= write_if_changed(&scalable.join("voxtype-recording.svg"), ICON_RECORDING_SVG)?;

    if any_changed {
        tracing::info!("Installed tray icons");
        update_icon_cache(&base);
    }

    Ok(())
}

/// Write `data` to `path` only if the file is missing or has different content.
/// Returns true if the file was written.
fn write_if_changed(path: &Path, data: &[u8]) -> anyhow::Result<bool> {
    if let Ok(existing) = fs::read(path) {
        if existing == data {
            return Ok(false);
        }
    }
    fs::write(path, data)?;
    Ok(true)
}

/// Remove tray icons installed by `install()`.
///
/// Best-effort: logs errors and continues so all files are attempted even if
/// one removal fails. Always returns `Ok(())`.
pub fn uninstall() -> anyhow::Result<()> {
    let base = hicolor_base()?;
    remove_icons(
        &base.join("128x128/apps"),
        &["voxtype.png", "voxtype-recording.png"],
    );
    remove_icons(
        &base.join("scalable/apps"),
        &["voxtype.svg", "voxtype-recording.svg"],
    );
    update_icon_cache(&base);
    Ok(())
}

/// Best-effort removal of named icon files from `dir`. Logs warnings on error.
fn remove_icons(dir: &Path, names: &[&str]) {
    for name in names {
        let path = dir.join(name);
        if path.exists() {
            if let Err(e) = fs::remove_file(&path) {
                tracing::warn!(path = %path.display(), "Failed to remove tray icon: {e}");
            } else {
                tracing::info!(path = %path.display(), "Removed tray icon");
            }
        }
    }
}

fn hicolor_base() -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
    Ok(home.join(".local/share/icons/hicolor"))
}

/// Run gtk-update-icon-cache on the hicolor theme directory (best-effort).
fn update_icon_cache(hicolor: &Path) {
    let result = std::process::Command::new("gtk-update-icon-cache")
        .arg("-f")
        .arg("-t")
        .arg(hicolor)
        .status();
    match result {
        Ok(s) if s.success() => tracing::debug!("Updated icon cache at {}", hicolor.display()),
        Ok(_) => tracing::debug!("gtk-update-icon-cache returned non-zero (non-fatal)"),
        Err(_) => tracing::debug!("gtk-update-icon-cache not found — icon cache not updated"),
    }
}
