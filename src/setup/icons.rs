//! Install voxtype XDG icon theme entries for the system tray.
//!
//! Installs `voxtype.png` (blue, idle state) and `voxtype-recording.png`
//! (red, recording/transcribing state) to the hicolor icon theme so that
//! SNI-compatible panels (KDE Plasma, waybar, etc.) can display them.

use std::fs;
use std::path::{Path, PathBuf};

const ICON_IDLE: &[u8] = include_bytes!("../../assets/icon.png");
const ICON_RECORDING: &[u8] = include_bytes!("../../assets/icon-recording.png");

/// Install tray icons to `~/.local/share/icons/hicolor/128x128/apps/` and
/// update the icon cache. Prints a summary of what was installed.
/// Skips silently if icons are already up to date.
pub async fn install() -> anyhow::Result<()> {
    let icon_dir = icon_dir()?;
    fs::create_dir_all(&icon_dir)?;

    let idle_path = icon_dir.join("voxtype.png");
    let recording_path = icon_dir.join("voxtype-recording.png");

    let idle_changed = write_if_changed(&idle_path, ICON_IDLE)?;
    let recording_changed = write_if_changed(&recording_path, ICON_RECORDING)?;

    if idle_changed || recording_changed {
        println!("Installed tray icons:");
        if idle_changed {
            println!("  {} (idle state)", idle_path.display());
        }
        if recording_changed {
            println!("  {} (recording state)", recording_path.display());
        }
        update_icon_cache(&icon_dir);
    }

    Ok(())
}

/// Write `data` to `path` only if the file is missing or has different content.
/// Returns true if the file was written.
fn write_if_changed(path: &std::path::Path, data: &[u8]) -> anyhow::Result<bool> {
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
    let icon_dir = icon_dir()?;

    for name in ["voxtype.png", "voxtype-recording.png"] {
        let path = icon_dir.join(name);
        if path.exists() {
            fs::remove_file(&path)?;
            println!("Removed {}", path.display());
        }
    }

    update_icon_cache(&icon_dir);

    Ok(())
}

fn icon_dir() -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
    Ok(home.join(".local/share/icons/hicolor/128x128/apps"))
}

/// Run gtk-update-icon-cache on the hicolor theme directory (best-effort).
fn update_icon_cache(app_dir: &Path) {
    // Walk up two levels: 128x128/apps -> 128x128 -> hicolor
    let hicolor = match app_dir.parent().and_then(|p| p.parent()) {
        Some(d) => d.to_owned(),
        None => return,
    };
    let result = std::process::Command::new("gtk-update-icon-cache")
        .arg("-f")
        .arg("-t")
        .arg(&hicolor)
        .status();
    match result {
        Ok(s) if s.success() => tracing::debug!("Updated icon cache at {}", hicolor.display()),
        Ok(_) => tracing::debug!("gtk-update-icon-cache returned non-zero (non-fatal)"),
        Err(_) => tracing::debug!("gtk-update-icon-cache not found — icon cache not updated"),
    }
}
