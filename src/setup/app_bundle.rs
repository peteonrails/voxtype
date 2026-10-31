//! macOS App Bundle creation and Login Items setup
//!
//! Creates a proper macOS app bundle for voxtype and manages Login Items.
//! This is preferred over launchd for the daemon because:
//! - App bundles can be granted Accessibility, Input Monitoring, and Microphone permissions
//! - Login Items inherit these permissions correctly (launchd services don't get mic access)

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{get_voxtype_path, print_failure, print_info, print_success, print_warning};

const APP_NAME: &str = "Voxtype.app";
pub const BUNDLE_ID: &str = "io.voxtype.daemon";

/// Get the path to the app bundle
///
/// Prefers an existing install (system-wide, then per-user). For a fresh install it uses
/// `/Applications`, falling back to `~/Applications` for standard (non-admin) accounts
/// that cannot write there.
pub fn app_bundle_path() -> PathBuf {
    resolve_bundle_path(
        Path::new("/Applications"),
        dirs::home_dir().map(|h| h.join("Applications")).as_deref(),
    )
}

fn resolve_bundle_path(system_dir: &Path, user_dir: Option<&Path>) -> PathBuf {
    let system_bundle = system_dir.join(APP_NAME);
    let Some(user_dir) = user_dir else {
        return system_bundle;
    };
    let user_bundle = user_dir.join(APP_NAME);
    if system_bundle.exists() {
        system_bundle
    } else if user_bundle.exists() || !is_writable(system_dir) {
        user_bundle
    } else {
        system_bundle
    }
}

fn is_writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(dir.as_os_str().as_bytes())
        // SAFETY: the pointer is a valid NUL-terminated string for the duration of the call
        .map(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
        .unwrap_or(false)
}

/// Get the path to the binary inside the app bundle
pub fn app_binary_path() -> PathBuf {
    app_bundle_path()
        .join("Contents")
        .join("MacOS")
        .join("voxtype-bin")
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
    <string>voxtype-bin</string>
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
    <key>NSInputMonitoringUsageDescription</key>
    <string>Voxtype monitors keyboard input to detect your push-to-talk hotkey.</string>
</dict>
</plist>
"#,
        bundle_id = BUNDLE_ID,
        version = version,
    )
}

/// Create the app bundle
pub fn create_app_bundle() -> anyhow::Result<()> {
    let app_path = app_bundle_path();
    let contents_path = app_path.join("Contents");
    let macos_path = contents_path.join("MacOS");

    // Create directory structure
    fs::create_dir_all(&macos_path).map_err(|e| {
        anyhow::anyhow!("Cannot create app bundle at {}: {}", app_path.display(), e)
    })?;

    // Get version from current binary
    let version = env!("CARGO_PKG_VERSION");

    // Write Info.plist
    fs::write(
        contents_path.join("Info.plist"),
        generate_info_plist(version),
    )?;

    // Copy the current voxtype binary (handle self-copy case)
    let source_binary = get_voxtype_path();
    let dest_binary = macos_path.join("voxtype-bin");
    let source_canon =
        fs::canonicalize(&source_binary).unwrap_or_else(|_| PathBuf::from(&source_binary));
    let dest_canon = fs::canonicalize(&dest_binary).unwrap_or_else(|_| dest_binary.clone());

    let binary_replaced = source_canon != dest_canon;
    if binary_replaced {
        // Copy via temp file for atomicity (prevents corruption if interrupted)
        let temp_binary = macos_path.join("voxtype-bin.tmp");
        fs::copy(&source_binary, &temp_binary)?;
        fs::rename(&temp_binary, &dest_binary)?;
    }

    // Make binary executable
    let mut perms = fs::metadata(&dest_binary)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&dest_binary, perms)?;

    // Remove legacy wrapper script if present (replaced by direct binary launch)
    let wrapper_path = macos_path.join("voxtype");
    let _ = fs::remove_file(&wrapper_path);

    // Sign the Mach-O binary individually first so it gets proper code page hashes,
    // then sign the whole bundle (--deep alone doesn't always hash inner binaries correctly)
    let _ = Command::new("codesign")
        .args(["--force", "--sign", "-", dest_binary.to_str().unwrap()])
        .output();

    let _ = Command::new("codesign")
        .args([
            "--force",
            "--deep",
            "--sign",
            "-",
            app_path.to_str().unwrap(),
        ])
        .output();

    // Reset TCC entries only when the binary changed, so macOS re-prompts for the
    // new code signature. Skip on self-copy to preserve existing permissions.
    if binary_replaced {
        let _ = Command::new("tccutil")
            .args(["reset", "Accessibility", BUNDLE_ID])
            .output();
        let _ = Command::new("tccutil")
            .args(["reset", "ListenEvent", BUNDLE_ID])
            .output();
    }

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

    let output = Command::new("osascript").args(["-e", &script]).output()?;

    Ok(output.status.success())
}

/// Remove app from Login Items
pub fn remove_from_login_items() -> anyhow::Result<bool> {
    let script = r#"tell application "System Events"
    if exists login item "Voxtype" then
        delete login item "Voxtype"
    end if
end tell"#;

    let output = Command::new("osascript").args(["-e", script]).output()?;

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
        "accessibility" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        "input" => "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent",
        "microphone" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
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

    // Launch the app
    let launched = Command::new("open")
        .arg(app_bundle_path().as_os_str())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if launched {
        print_success("Launched Voxtype.app");
    } else {
        print_warning("Could not launch automatically");
        print_info(&format!(
            "Start manually: open {}",
            app_bundle_path().display()
        ));
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
    let output = Command::new("pgrep").args(["-f", "Voxtype.app"]).output();

    match output {
        Ok(out) if out.status.success() => {
            let pid = String::from_utf8_lossy(&out.stdout);
            print_success(&format!("Running (PID: {})", pid.trim()));
        }
        _ => {
            print_info("Not currently running");
            print_info(&format!("Start with: open {}", app_bundle_path().display()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_user_dir_when_system_dir_not_writable() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("Applications");
        let missing = tmp.path().join("no-such-system-dir");
        assert_eq!(
            resolve_bundle_path(&missing, Some(&user)),
            user.join(APP_NAME)
        );
    }

    #[test]
    fn uses_system_dir_when_writable() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user");
        assert_eq!(
            resolve_bundle_path(tmp.path(), Some(&user)),
            tmp.path().join(APP_NAME)
        );
    }

    #[test]
    fn existing_user_bundle_wins_over_writable_system_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user");
        fs::create_dir_all(user.join(APP_NAME)).unwrap();
        assert_eq!(
            resolve_bundle_path(tmp.path(), Some(&user)),
            user.join(APP_NAME)
        );
    }

    #[test]
    fn existing_system_bundle_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user");
        fs::create_dir_all(user.join(APP_NAME)).unwrap();
        fs::create_dir_all(tmp.path().join(APP_NAME)).unwrap();
        assert_eq!(
            resolve_bundle_path(tmp.path(), Some(&user)),
            tmp.path().join(APP_NAME)
        );
    }
}
