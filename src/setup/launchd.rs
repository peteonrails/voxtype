//! macOS LaunchAgent service installation
//!
//! Provides commands to install, uninstall, and check the status of
//! voxtype as a launchd user service on macOS.

use super::{get_voxtype_path, print_failure, print_info, print_success, print_warning};
use std::fs;
use std::path::PathBuf;

const PLIST_FILENAME: &str = "io.voxtype.daemon.plist";

/// Get the path to the LaunchAgents directory
fn launch_agents_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Library/LaunchAgents"))
}

/// Get the path to the plist file
fn plist_path() -> Option<PathBuf> {
    launch_agents_dir().map(|dir| dir.join(PLIST_FILENAME))
}

/// Get the path to the logs directory
fn logs_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Library/Logs/voxtype"))
}

/// Generate the launchd plist content
fn generate_plist() -> String {
    let voxtype_path = get_voxtype_path();
    let logs_dir = logs_dir().unwrap_or_else(|| PathBuf::from("/tmp"));

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>io.voxtype.daemon</string>

    <key>ProgramArguments</key>
    <array>
        <string>{voxtype_path}</string>
        <string>daemon</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>{stdout}</string>

    <key>StandardErrorPath</key>
    <string>{stderr}</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin</string>
    </dict>

    <key>ProcessType</key>
    <string>Interactive</string>

    <key>Nice</key>
    <integer>-10</integer>
</dict>
</plist>
"#,
        voxtype_path = voxtype_path,
        stdout = logs_dir.join("stdout.log").display(),
        stderr = logs_dir.join("stderr.log").display(),
    )
}

/// Install the LaunchAgent
pub async fn install() -> anyhow::Result<()> {
    println!("Installing Voxtype LaunchAgent...\n");

    // Check if we're on macOS
    if !cfg!(target_os = "macos") {
        print_failure("This command is only available on macOS");
        print_info("On Linux, use: voxtype setup systemd");
        anyhow::bail!("Not on macOS");
    }

    // Ensure LaunchAgents directory exists
    let launch_dir = launch_agents_dir().ok_or_else(|| anyhow::anyhow!("Could not determine LaunchAgents directory"))?;
    fs::create_dir_all(&launch_dir)?;

    // Ensure logs directory exists
    if let Some(logs) = logs_dir() {
        fs::create_dir_all(&logs)?;
        print_success(&format!("Logs directory: {:?}", logs));
    }

    // Generate and write the plist
    let plist = plist_path().ok_or_else(|| anyhow::anyhow!("Could not determine plist path"))?;
    let content = generate_plist();
    fs::write(&plist, &content)?;
    print_success(&format!("Created: {:?}", plist));

    // Load the service
    let status = std::process::Command::new("launchctl")
        .args(["load", plist.to_str().unwrap_or("")])
        .status();

    match status {
        Ok(s) if s.success() => {
            print_success("LaunchAgent loaded");
        }
        _ => {
            print_warning("Could not load LaunchAgent automatically");
            print_info("Try: launchctl load ~/Library/LaunchAgents/io.voxtype.daemon.plist");
        }
    }

    println!("\n---");
    println!("\x1b[32m✓ Installation complete!\x1b[0m");
    println!();
    println!("Voxtype will now start automatically on login.");
    println!();
    println!("Useful commands:");
    println!("  launchctl start io.voxtype.daemon   - Start now");
    println!("  launchctl stop io.voxtype.daemon    - Stop");
    println!("  launchctl unload ~/Library/LaunchAgents/io.voxtype.daemon.plist - Disable");
    println!();
    println!("Logs:");
    if let Some(logs) = logs_dir() {
        println!("  tail -f {:?}/stdout.log", logs);
        println!("  tail -f {:?}/stderr.log", logs);
    }

    Ok(())
}

/// Uninstall the LaunchAgent
pub async fn uninstall() -> anyhow::Result<()> {
    println!("Uninstalling Voxtype LaunchAgent...\n");

    let plist = plist_path().ok_or_else(|| anyhow::anyhow!("Could not determine plist path"))?;

    if !plist.exists() {
        print_info("LaunchAgent not installed");
        return Ok(());
    }

    // Unload the service first
    let _ = std::process::Command::new("launchctl")
        .args(["unload", plist.to_str().unwrap_or("")])
        .status();

    // Remove the plist file
    fs::remove_file(&plist)?;
    print_success("LaunchAgent removed");

    println!("\n---");
    println!("\x1b[32m✓ Uninstallation complete!\x1b[0m");

    Ok(())
}

/// Show LaunchAgent status
pub async fn status() -> anyhow::Result<()> {
    println!("Voxtype LaunchAgent Status\n");
    println!("==========================\n");

    let plist = plist_path().ok_or_else(|| anyhow::anyhow!("Could not determine plist path"))?;

    // Check if plist exists
    if plist.exists() {
        print_success(&format!("Plist installed: {:?}", plist));
    } else {
        print_failure("LaunchAgent not installed");
        print_info("Install with: voxtype setup launchd");
        return Ok(());
    }

    // Check if service is running
    let output = std::process::Command::new("launchctl")
        .args(["list", "io.voxtype.daemon"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.contains("io.voxtype.daemon") {
                print_success("Service is running");

                // Parse PID if available
                for line in stdout.lines() {
                    if let Some(pid) = line.split_whitespace().next() {
                        if pid != "-" {
                            println!("  PID: {}", pid);
                        }
                    }
                }
            } else {
                print_warning("Service is loaded but not running");
            }
        }
        _ => {
            print_warning("Service is not loaded");
            print_info("Start with: launchctl load ~/Library/LaunchAgents/io.voxtype.daemon.plist");
        }
    }

    // Show log locations
    if let Some(logs) = logs_dir() {
        println!("\nLogs:");
        let stdout_log = logs.join("stdout.log");
        let stderr_log = logs.join("stderr.log");

        if stdout_log.exists() {
            let size = fs::metadata(&stdout_log)
                .map(|m| m.len())
                .unwrap_or(0);
            println!("  stdout: {:?} ({} bytes)", stdout_log, size);
        }
        if stderr_log.exists() {
            let size = fs::metadata(&stderr_log)
                .map(|m| m.len())
                .unwrap_or(0);
            println!("  stderr: {:?} ({} bytes)", stderr_log, size);
        }
    }

    Ok(())
}

/// Regenerate the LaunchAgent plist file (e.g., after binary path change)
/// Returns true if the file was updated
pub fn regenerate_plist() -> anyhow::Result<bool> {
    let plist = match plist_path() {
        Some(p) if p.exists() => p,
        _ => return Ok(false),
    };

    let content = generate_plist();
    fs::write(&plist, &content)?;

    Ok(true)
}
