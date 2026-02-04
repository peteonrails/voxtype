//! macOS App Bundle creation and Login Items setup
//!
//! Creates a proper macOS app bundle for voxtype and manages Login Items.
//! This is preferred over launchd for the daemon because:
//! - App bundles can be granted Accessibility, Input Monitoring, and Microphone permissions
//! - Login Items inherit these permissions correctly (launchd services don't get mic access)

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use super::{get_voxtype_path, print_failure, print_info, print_success, print_warning};

const APP_NAME: &str = "Voxtype.app";
const BUNDLE_ID: &str = "io.voxtype.daemon";

/// Get the path to the app bundle
pub fn app_bundle_path() -> PathBuf {
    PathBuf::from("/Applications").join(APP_NAME)
}

/// Get the path to the binary inside the app bundle
pub fn app_binary_path() -> PathBuf {
    app_bundle_path()
        .join("Contents")
        .join("MacOS")
        .join("voxtype")
}

/// Get the path to the logs directory
fn logs_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Library/Logs/voxtype"))
}

/// Generate Info.plist content
fn generate_info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>voxtype</string>
    <key>CFBundleIdentifier</key>
    <string>{bundle_id}</string>
    <key>CFBundleName</key>
    <string>Voxtype</string>
    <key>CFBundleDisplayName</key>
    <string>Voxtype</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>Voxtype needs microphone access for speech-to-text transcription.</string>
    <key>NSAppleEventsUsageDescription</key>
    <string>Voxtype needs accessibility access to type transcribed text.</string>
</dict>
</plist>
"#,
        bundle_id = BUNDLE_ID,
        version = version,
    )
}

/// Generate wrapper script that runs the daemon and menubar
fn generate_wrapper_script() -> String {
    let logs = logs_dir().unwrap_or_else(|| PathBuf::from("/tmp/voxtype"));
    format!(
        r#"#!/bin/bash
# Voxtype app wrapper - starts daemon and menu bar

# Kill any existing instances
pkill -9 -f "voxtype daemon" 2>/dev/null
pkill -9 -f "voxtype menubar" 2>/dev/null
rm -f /tmp/voxtype/voxtype.lock

# Create logs directory
mkdir -p "{logs}"

# Get the directory where this script is located
DIR="$(cd "$(dirname "$0")" && pwd)"

# Start daemon in background with logging
"$DIR/voxtype-bin" daemon >> "{logs}/stdout.log" 2>> "{logs}/stderr.log" &

# Start menubar (foreground keeps app alive and shows menu bar icon)
exec "$DIR/voxtype-bin" menubar
"#,
        logs = logs.display()
    )
}

/// Create the app bundle
pub fn create_app_bundle() -> anyhow::Result<()> {
    let app_path = app_bundle_path();
    let contents_path = app_path.join("Contents");
    let macos_path = contents_path.join("MacOS");

    // Create directory structure
    fs::create_dir_all(&macos_path)?;

    // Get version from current binary
    let version = env!("CARGO_PKG_VERSION");

    // Write Info.plist
    fs::write(contents_path.join("Info.plist"), generate_info_plist(version))?;

    // Copy the current voxtype binary
    let source_binary = get_voxtype_path();
    let dest_binary = macos_path.join("voxtype-bin");
    fs::copy(&source_binary, &dest_binary)?;

    // Make binary executable
    let mut perms = fs::metadata(&dest_binary)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&dest_binary, perms)?;

    // Create wrapper script as main executable
    let wrapper_path = macos_path.join("voxtype");
    fs::write(&wrapper_path, generate_wrapper_script())?;
    let mut perms = fs::metadata(&wrapper_path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&wrapper_path, perms)?;

    // Code sign the app bundle (ad-hoc)
    let _ = Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-", app_path.to_str().unwrap()])
        .output();

    Ok(())
}

/// Add app to Login Items
pub fn add_to_login_items() -> anyhow::Result<bool> {
    let app_path = app_bundle_path();
    let script = format!(
        r#"tell application "System Events"
    if not (exists login item "Voxtype") then
        make login item at end with properties {{path:"{}", hidden:true}}
    end if
end tell"#,
        app_path.display()
    );

    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()?;

    Ok(output.status.success())
}

/// Remove app from Login Items
pub fn remove_from_login_items() -> anyhow::Result<bool> {
    let script = r#"tell application "System Events"
    if exists login item "Voxtype" then
        delete login item "Voxtype"
    end if
end tell"#;

    let output = Command::new("osascript")
        .args(["-e", script])
        .output()?;

    Ok(output.status.success())
}

/// Check if app is in Login Items
pub fn is_in_login_items() -> bool {
    let script = r#"tell application "System Events"
    return exists login item "Voxtype"
end tell"#;

    Command::new("osascript")
        .args(["-e", script])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// Remove the app bundle
pub fn remove_app_bundle() -> anyhow::Result<()> {
    let app_path = app_bundle_path();
    if app_path.exists() {
        fs::remove_dir_all(&app_path)?;
    }
    Ok(())
}

/// Open System Settings to the relevant privacy pane
pub fn open_privacy_settings(pane: &str) -> anyhow::Result<()> {
    let url = match pane {
        "accessibility" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
        "input" => "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent",
        "microphone" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
        "login" => "x-apple.systempreferences:com.apple.LoginItems-Settings.extension",
        _ => return Err(anyhow::anyhow!("Unknown pane: {}", pane)),
    };

    Command::new("open").arg(url).spawn()?;
    Ok(())
}

/// Install the app bundle and set up Login Items
pub async fn install() -> anyhow::Result<()> {
    println!("Installing Voxtype.app...\n");

    // Create logs directory
    if let Some(logs) = logs_dir() {
        fs::create_dir_all(&logs)?;
        print_success(&format!("Logs directory: {:?}", logs));
    }

    // Create app bundle
    create_app_bundle()?;
    print_success(&format!("Created: {:?}", app_bundle_path()));

    // Add to Login Items
    if add_to_login_items()? {
        print_success("Added to Login Items");
    } else {
        print_warning("Could not add to Login Items automatically");
        print_info("Add manually: System Settings > General > Login Items");
    }

    println!("\n---");
    println!("\x1b[32m✓ Installation complete!\x1b[0m");
    println!();
    println!("\x1b[1mIMPORTANT: Grant permissions to Voxtype.app:\x1b[0m");
    println!();
    println!("  1. System Settings > Privacy & Security > \x1b[1mAccessibility\x1b[0m");
    println!("     Add and enable Voxtype");
    println!();
    println!("  2. System Settings > Privacy & Security > \x1b[1mInput Monitoring\x1b[0m");
    println!("     Add and enable Voxtype");
    println!();
    println!("  3. System Settings > Privacy & Security > \x1b[1mMicrophone\x1b[0m");
    println!("     Voxtype should appear after first use - enable it");
    println!();
    println!("To start now:");
    println!("  open /Applications/Voxtype.app");
    println!();
    println!("Voxtype will start automatically on login.");

    Ok(())
}

/// Uninstall the app bundle and remove from Login Items
pub async fn uninstall() -> anyhow::Result<()> {
    println!("Uninstalling Voxtype.app...\n");

    // Stop any running instance
    let _ = Command::new("pkill")
        .args(["-9", "-f", "Voxtype.app"])
        .status();

    // Remove from Login Items
    if remove_from_login_items()? {
        print_success("Removed from Login Items");
    }

    // Remove app bundle
    if app_bundle_path().exists() {
        remove_app_bundle()?;
        print_success("Removed Voxtype.app");
    } else {
        print_info("Voxtype.app was not installed");
    }

    println!("\n---");
    println!("\x1b[32m✓ Uninstallation complete!\x1b[0m");

    Ok(())
}

/// Show installation status
pub async fn status() -> anyhow::Result<()> {
    println!("Voxtype.app Status\n");
    println!("==================\n");

    // Check app bundle
    if app_bundle_path().exists() {
        print_success(&format!("App installed: {:?}", app_bundle_path()));
    } else {
        print_failure("Voxtype.app not installed");
        print_info("Install with: voxtype setup app-bundle");
        return Ok(());
    }

    // Check Login Items
    if is_in_login_items() {
        print_success("In Login Items (will start on login)");
    } else {
        print_warning("Not in Login Items");
        print_info("Add with: voxtype setup app-bundle");
    }

    // Check if running
    let output = Command::new("pgrep")
        .args(["-f", "Voxtype.app"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let pid = String::from_utf8_lossy(&out.stdout);
            print_success(&format!("Running (PID: {})", pid.trim()));
        }
        _ => {
            print_info("Not currently running");
            print_info("Start with: open /Applications/Voxtype.app");
        }
    }

    // Show log locations
    if let Some(logs) = logs_dir() {
        println!("\nLogs:");
        let stdout_log = logs.join("stdout.log");
        let stderr_log = logs.join("stderr.log");

        if stdout_log.exists() {
            let size = fs::metadata(&stdout_log).map(|m| m.len()).unwrap_or(0);
            println!("  stdout: {:?} ({} bytes)", stdout_log, size);
        }
        if stderr_log.exists() {
            let size = fs::metadata(&stderr_log).map(|m| m.len()).unwrap_or(0);
            println!("  stderr: {:?} ({} bytes)", stderr_log, size);
        }
    }

    Ok(())
}
