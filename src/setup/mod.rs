//! Setup module for voxtype installation and configuration
//!
//! Provides subcommands for:
//! - systemd service installation
//! - Waybar configuration generation
//! - Interactive model selection

pub mod model;
pub mod systemd;
pub mod waybar;

use crate::config::Config;
use std::process::Stdio;
use tokio::process::Command;

/// Check if a command exists in PATH
pub async fn command_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if user is in a specific group
pub fn user_in_group(group: &str) -> bool {
    std::process::Command::new("groups")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(group))
        .unwrap_or(false)
}

/// Get the voxtype binary path
pub fn get_voxtype_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "voxtype".to_string())
}

/// Print a success message
pub fn print_success(msg: &str) {
    println!("  \x1b[32m✓\x1b[0m {}", msg);
}

/// Print a failure message
pub fn print_failure(msg: &str) {
    println!("  \x1b[31m✗\x1b[0m {}", msg);
}

/// Print an info message
pub fn print_info(msg: &str) {
    println!("  \x1b[34mℹ\x1b[0m {}", msg);
}

/// Run the basic setup checks (existing functionality)
pub async fn run_basic_setup(config: &Config, download: bool) -> anyhow::Result<()> {
    println!("Voxtype Setup\n");
    println!("=============\n");

    // Ensure directories exist first
    println!("Creating directories...");
    Config::ensure_directories()?;
    print_success(&format!(
        "Config directory: {:?}",
        Config::config_dir().unwrap_or_default()
    ));
    print_success(&format!("Models directory: {:?}", Config::models_dir()));

    // Create default config file if it doesn't exist
    if let Some(config_path) = Config::default_path() {
        if !config_path.exists() {
            println!("\nCreating default config file...");
            std::fs::write(&config_path, crate::config::DEFAULT_CONFIG)?;
            print_success(&format!("Created: {:?}", config_path));
        } else {
            println!("\n  Config file exists: {:?}", config_path);
        }
    }

    let mut all_ok = true;

    // Check input group
    println!("\nChecking input group membership...");
    if user_in_group("input") {
        print_success("User is in 'input' group");
    } else {
        print_failure("User is NOT in 'input' group");
        println!("    Run: sudo usermod -aG input $USER");
        println!("    Then log out and back in");
        all_ok = false;
    }

    // Check ydotool
    println!("\nChecking ydotool...");
    if command_exists("ydotool").await {
        print_success("ydotool found");

        // Check daemon
        let daemon_check = Command::new("systemctl")
            .args(["--user", "is-active", "ydotool"])
            .output()
            .await?;
        if daemon_check.status.success() {
            print_success("ydotool daemon running");
        } else {
            print_failure("ydotool daemon not running");
            println!("    Run: systemctl --user enable --now ydotool");
            all_ok = false;
        }
    } else {
        print_failure("ydotool not found (typing won't work, will use clipboard)");
        println!("    Install via your package manager");
    }

    // Check wl-copy
    println!("\nChecking wl-clipboard...");
    if command_exists("wl-copy").await {
        print_success("wl-copy found");
    } else {
        print_failure("wl-copy not found");
        println!("    Install wl-clipboard via your package manager");
        all_ok = false;
    }

    // Check whisper model
    println!("\nChecking whisper model...");
    let models_dir = Config::models_dir();
    let model_name = &config.whisper.model;

    let model_filename = crate::transcribe::whisper::get_model_filename(model_name);
    let model_path = models_dir.join(&model_filename);

    if model_path.exists() {
        let size = std::fs::metadata(&model_path)
            .map(|m| m.len() as f64 / 1024.0 / 1024.0)
            .unwrap_or(0.0);
        print_success(&format!(
            "Model found: {:?} ({:.0} MB)",
            model_path, size
        ));
    } else {
        print_failure(&format!("Model not found: {:?}", model_path));
        all_ok = false;

        if download {
            println!("\n  Downloading model...");
            std::fs::create_dir_all(&models_dir)?;

            let url = crate::transcribe::whisper::get_model_url(model_name);
            println!("  URL: {}", url);

            let response = reqwest::get(&url).await?;
            let total_size = response.content_length().unwrap_or(0);
            println!("  Size: {:.0} MB", total_size as f64 / 1024.0 / 1024.0);

            let bytes = response.bytes().await?;
            std::fs::write(&model_path, &bytes)?;
            print_success(&format!("Downloaded to {:?}", model_path));
        } else {
            let url = crate::transcribe::whisper::get_model_url(model_name);
            println!("\n  To download automatically, run: voxtype setup --download");
            println!("  Or manually download from:");
            println!("    {}", url);
            println!("  And place in: {:?}", models_dir);
        }
    }

    // Summary
    println!("\n---");
    if all_ok {
        println!("\x1b[32m✓ All checks passed!\x1b[0m Run 'voxtype' to start.");
        println!("\nOptional next steps:");
        println!("  voxtype setup systemd  - Install as systemd service");
        println!("  voxtype setup waybar   - Get Waybar integration config");
    } else {
        println!("\x1b[31m✗ Some checks failed.\x1b[0m Please fix the issues above.");
    }

    Ok(())
}
