//! Systemd user service management for voxtype

use super::{get_voxtype_service_path, print_failure, print_info, print_success};
use std::path::PathBuf;
use tokio::process::Command;

const SERVICE_NAME: &str = "voxtype.service";

/// Get the systemd user service directory
fn service_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.config_dir().join("systemd/user"))
        .unwrap_or_else(|| PathBuf::from("~/.config/systemd/user"))
}

/// Get the path to the voxtype service file
fn service_path() -> PathBuf {
    service_dir().join(SERVICE_NAME)
}

/// Generate the systemd service file content
fn generate_service_file() -> String {
    let voxtype_path = get_voxtype_service_path();

    format!(
        r#"[Unit]
Description=Voxtype push-to-talk voice-to-text daemon
Documentation=https://voxtype.io
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart={voxtype_path} daemon
Restart=on-failure
RestartSec=5

# Ensure we have access to the display
Environment=XDG_RUNTIME_DIR=%t

[Install]
WantedBy=graphical-session.target
"#
    )
}

/// Install the systemd user service
pub async fn install() -> anyhow::Result<()> {
    println!("Installing voxtype systemd service...\n");

    let service_dir = service_dir();
    let service_path = service_path();

    // Create directory if needed
    std::fs::create_dir_all(&service_dir)?;
    print_success(&format!("Service directory: {:?}", service_dir));

    // Write service file
    let content = generate_service_file();
    std::fs::write(&service_path, &content)?;
    print_success(&format!("Created: {:?}", service_path));

    // Reload systemd
    println!("\nReloading systemd...");
    let reload = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()
        .await?;

    if reload.success() {
        print_success("Systemd daemon reloaded");
    } else {
        print_failure("Failed to reload systemd daemon");
        return Ok(());
    }

    // Enable the service
    println!("\nEnabling service...");
    let enable = Command::new("systemctl")
        .args(["--user", "enable", SERVICE_NAME])
        .status()
        .await?;

    if enable.success() {
        print_success("Service enabled (will start on login)");
    } else {
        print_failure("Failed to enable service");
    }

    // Start the service
    println!("\nStarting service...");
    let start = Command::new("systemctl")
        .args(["--user", "start", SERVICE_NAME])
        .status()
        .await?;

    if start.success() {
        print_success("Service started");
    } else {
        print_failure("Failed to start service");
        println!("    Check logs with: journalctl --user -u voxtype");
    }

    // Show status
    println!("\n---");
    println!("Service installed successfully!\n");
    println!("Useful commands:");
    println!("  systemctl --user status voxtype   # Check status");
    println!("  systemctl --user restart voxtype  # Restart");
    println!("  systemctl --user stop voxtype     # Stop");
    println!("  journalctl --user -u voxtype -f   # View logs");

    Ok(())
}

/// Uninstall the systemd user service
pub async fn uninstall() -> anyhow::Result<()> {
    println!("Uninstalling voxtype systemd service...\n");

    let service_path = service_path();

    // Stop the service if running
    println!("Stopping service...");
    let _ = Command::new("systemctl")
        .args(["--user", "stop", SERVICE_NAME])
        .status()
        .await;
    print_success("Service stopped");

    // Disable the service
    println!("\nDisabling service...");
    let _ = Command::new("systemctl")
        .args(["--user", "disable", SERVICE_NAME])
        .status()
        .await;
    print_success("Service disabled");

    // Remove service file
    if service_path.exists() {
        std::fs::remove_file(&service_path)?;
        print_success(&format!("Removed: {:?}", service_path));
    } else {
        print_info("Service file not found (already removed?)");
    }

    // Reload systemd
    println!("\nReloading systemd...");
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()
        .await;
    print_success("Systemd daemon reloaded");

    println!("\n---");
    println!("Service uninstalled successfully!");

    Ok(())
}

/// Show the service status
pub async fn status() -> anyhow::Result<()> {
    let output = Command::new("systemctl")
        .args(["--user", "status", SERVICE_NAME])
        .output()
        .await?;

    println!("{}", String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(())
}

/// Regenerate the service file content (used after backend switches)
///
/// Returns true if the service file was updated, false if no service was installed.
pub fn regenerate_service_file() -> anyhow::Result<bool> {
    let service_path = service_path();

    if !service_path.exists() {
        return Ok(false); // No service installed, nothing to update
    }

    let content = generate_service_file();
    std::fs::write(&service_path, &content)?;
    Ok(true)
}
