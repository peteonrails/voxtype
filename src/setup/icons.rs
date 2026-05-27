//! Install voxtype XDG icon theme entries for the system tray.
//!
//! Installs `voxtype.png` and `voxtype-recording.png` at all standard hicolor
//! pixel sizes (16, 22, 24, 32, 48, 128, 256) plus scalable SVGs under
//! `hicolor/scalable/apps/`. Panels choose the best-fit size they need.

use std::fs;
use std::path::{Path, PathBuf};

const ICON_IDLE_PNG: &[u8] = include_bytes!("../../assets/icon.png");
const ICON_RECORDING_PNG: &[u8] = include_bytes!("../../assets/icon-recording.png");
const ICON_IDLE_SVG: &[u8] = include_bytes!("../../assets/icon.svg");
const ICON_RECORDING_SVG: &[u8] = include_bytes!("../../assets/icon-recording.svg");

/// Standard hicolor pixel sizes to install.
const PNG_SIZES: &[u32] = &[16, 22, 24, 32, 48, 128, 256];

/// Install tray icons to `~/.local/share/icons/hicolor/` at all standard sizes
/// plus scalable SVGs. Skips silently if icons are already up to date.
pub async fn install() -> anyhow::Result<()> {
    let base = hicolor_base()?;
    let mut any_changed = false;

    // PNG sizes — install the same 128px PNG into every size directory;
    // panels that request a smaller size will scale it down.
    for &size in PNG_SIZES {
        let dir = base.join(format!("{size}x{size}/apps"));
        fs::create_dir_all(&dir)?;
        any_changed |= write_if_changed(&dir.join("voxtype.png"), ICON_IDLE_PNG)?;
        any_changed |= write_if_changed(&dir.join("voxtype-recording.png"), ICON_RECORDING_PNG)?;
    }

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
pub async fn uninstall() -> anyhow::Result<()> {
    let base = hicolor_base()?;

    for &size in PNG_SIZES {
        let dir = base.join(format!("{size}x{size}/apps"));
        for name in ["voxtype.png", "voxtype-recording.png"] {
            let path = dir.join(name);
            if path.exists() {
                fs::remove_file(&path)?;
                tracing::info!(path = %path.display(), "Removed tray icon");
            }
        }
    }

    let scalable = base.join("scalable/apps");
    for name in ["voxtype.svg", "voxtype-recording.svg"] {
        let path = scalable.join(name);
        if path.exists() {
            fs::remove_file(&path)?;
            tracing::info!(path = %path.display(), "Removed tray icon");
        }
    }

    update_icon_cache(&base);

    Ok(())
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
