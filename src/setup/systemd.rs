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

    // The unit inherits its display from the user manager. If the compositor
    // never imported WAYLAND_DISPLAY/DISPLAY there, the daemon can't type.
    if let Ok(env) = Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
        .await
    {
        if env.status.success() && manager_env_lacks_display(&String::from_utf8_lossy(&env.stdout))
        {
            print_failure("The systemd user manager has no WAYLAND_DISPLAY or DISPLAY");
            println!("    voxtype will not be able to type or copy text. Fix it now with:");
            println!(
                "      systemctl --user import-environment WAYLAND_DISPLAY DISPLAY && systemctl --user restart voxtype"
            );
            println!("    and make it permanent by running the import when your compositor starts");
            println!(
                "    (Hyprland: exec-once = systemctl --user import-environment WAYLAND_DISPLAY DISPLAY)"
            );
        }
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

/// `systemctl --user show-environment` output with neither WAYLAND_DISPLAY nor
/// DISPLAY set, so services started by the user manager have no display.
fn manager_env_lacks_display(show_env: &str) -> bool {
    !show_env.lines().any(|line| {
        line.split_once('=')
            .is_some_and(|(k, v)| (k == "WAYLAND_DISPLAY" || k == "DISPLAY") && !v.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_env_lacks_display_reads_show_environment() {
        assert!(manager_env_lacks_display(
            "HOME=/home/u\nXDG_RUNTIME_DIR=/run/user/1000\n"
        ));
        assert!(manager_env_lacks_display("WAYLAND_DISPLAY=\n"));
        assert!(!manager_env_lacks_display(
            "HOME=/home/u\nWAYLAND_DISPLAY=wayland-1\n"
        ));
        assert!(!manager_env_lacks_display("DISPLAY=:0\n"));
    }
}
