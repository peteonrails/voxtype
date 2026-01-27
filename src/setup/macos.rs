//! macOS interactive setup wizard
//!
//! Provides a guided setup experience for macOS users, covering:
//! - Accessibility permission checks
//! - Hotkey configuration (native rdev or Hammerspoon)
//! - Menu bar setup
//! - LaunchAgent auto-start
//! - Model download

use super::{print_failure, print_info, print_success, print_warning};
use std::io::{self, Write};

/// Check if Terminal/iTerm has Accessibility permission
async fn check_accessibility_permission() -> bool {
    // Try to use osascript to check if we can control System Events
    let output = tokio::process::Command::new("osascript")
        .args(["-e", "tell application \"System Events\" to return name of first process"])
        .output()
        .await;

    match output {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

/// Check if Hammerspoon is installed
async fn check_hammerspoon() -> bool {
    std::path::Path::new("/Applications/Hammerspoon.app").exists()
        || tokio::process::Command::new("which")
            .arg("hs")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Check if terminal-notifier is installed
async fn check_terminal_notifier() -> bool {
    tokio::process::Command::new("which")
        .arg("terminal-notifier")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get user input with a default value
fn prompt(message: &str, default: &str) -> String {
    print!("{} [{}]: ", message, default);
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let input = input.trim();

    if input.is_empty() {
        default.to_string()
    } else {
        input.to_string()
    }
}

/// Get yes/no input
fn prompt_yn(message: &str, default: bool) -> bool {
    let default_str = if default { "Y/n" } else { "y/N" };
    print!("{} [{}]: ", message, default_str);
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let input = input.trim().to_lowercase();

    match input.as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        "" => default,
        _ => default,
    }
}

/// Print a section header
fn section(title: &str) {
    println!("\n\x1b[1m{}\x1b[0m", title);
    println!("{}", "─".repeat(title.len()));
}

/// Check if a notification icon is installed
fn check_notification_icon() -> bool {
    let candidates = [
        dirs::data_dir().map(|d| d.join("voxtype/icon.png")),
        dirs::config_dir().map(|d| d.join("voxtype/icon.png")),
    ];

    candidates.into_iter().flatten().any(|p| p.exists())
}

/// Install a default notification icon
fn install_default_icon_file() -> anyhow::Result<()> {
    // Create the data directory
    let data_dir = dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find Application Support directory"))?
        .join("voxtype");

    std::fs::create_dir_all(&data_dir)?;

    let icon_path = data_dir.join("icon.png");

    // Create a simple microphone icon as PNG
    // This is a 64x64 PNG with a microphone glyph
    // Base64-encoded PNG data for a simple blue microphone icon
    let icon_data = include_bytes!("../../assets/icon.png");
    std::fs::write(&icon_path, icon_data)?;

    println!("    Installed icon: {}", icon_path.display());
    Ok(())
}

/// Run the macOS setup wizard
pub async fn run() -> anyhow::Result<()> {
    println!("\x1b[1mVoxtype macOS Setup Wizard\x1b[0m");
    println!("==========================\n");
    println!("This wizard will guide you through setting up Voxtype on macOS.\n");

    // Step 1: Check system requirements
    section("Step 1: System Requirements");

    // Check macOS version
    let macos_version = tokio::process::Command::new("sw_vers")
        .args(["-productVersion"])
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "Unknown".to_string());
    print_success(&format!("macOS version: {}", macos_version));

    // Check accessibility permission
    print!("  Checking Accessibility permission... ");
    io::stdout().flush().unwrap();
    let has_accessibility = check_accessibility_permission().await;
    println!();

    if has_accessibility {
        print_success("Accessibility permission granted");
    } else {
        print_warning("Accessibility permission not granted");
        println!();
        println!("    To enable typing output, grant Accessibility permission to your terminal:");
        println!("    1. Open System Settings → Privacy & Security → Accessibility");
        println!("    2. Add your terminal app (Terminal, iTerm2, Alacritty, etc.)");
        println!("    3. Restart your terminal after granting permission");
        println!();
        println!("    Alternatively, use Hammerspoon for hotkey support (no terminal permission needed)");
    }

    // Check terminal-notifier
    let has_notifier = check_terminal_notifier().await;
    if has_notifier {
        print_success("terminal-notifier installed (enhanced notifications)");
    } else {
        print_info("terminal-notifier not installed (optional, for better notifications)");
        println!("       Install with: brew install terminal-notifier");
    }

    // Step 2: Hotkey configuration
    section("Step 2: Hotkey Configuration");

    let has_hammerspoon = check_hammerspoon().await;

    println!("Voxtype supports two methods for global hotkey capture:\n");
    println!("  1. Native (rdev) - Built-in, requires Accessibility permission for terminal");
    println!("  2. Hammerspoon   - External app, doesn't require terminal permission\n");

    if has_hammerspoon {
        print_success("Hammerspoon is installed");
    } else {
        print_info("Hammerspoon is not installed");
        println!("       Install with: brew install --cask hammerspoon");
    }

    let use_hammerspoon = if has_hammerspoon {
        println!();
        prompt_yn("Use Hammerspoon for hotkey support?", !has_accessibility)
    } else {
        false
    };

    let hotkey = prompt("\nHotkey to use", "rightalt");
    let toggle_mode = prompt_yn("Use toggle mode? (press to start/stop instead of hold to record)", false);

    if use_hammerspoon {
        println!();
        println!("Setting up Hammerspoon integration...");

        // Install the Hammerspoon module
        if let Err(e) = super::hammerspoon::run(true, false, &hotkey, toggle_mode).await {
            print_warning(&format!("Could not set up Hammerspoon: {}", e));
        }
    } else {
        print_success(&format!("Configured native hotkey: {}", hotkey));
        print_info(&format!("Mode: {}", if toggle_mode { "toggle" } else { "push-to-talk" }));

        if !has_accessibility {
            println!();
            print_warning("Remember to grant Accessibility permission to your terminal!");
        }
    }

    // Step 3: Menu bar
    section("Step 3: Menu Bar Integration");

    println!("The menu bar helper shows recording status and provides quick controls.\n");

    let setup_menubar = prompt_yn("Set up menu bar helper?", true);

    if setup_menubar {
        print_success("Menu bar helper will be available via: voxtype menubar");
        print_info("You can add it to LaunchAgent for auto-start (next step)");
    }

    // Step 4: Auto-start
    section("Step 4: Auto-start Configuration");

    println!("Voxtype can start automatically when you log in.\n");

    let setup_autostart = prompt_yn("Set up auto-start (LaunchAgent)?", true);

    if setup_autostart {
        println!();
        println!("Installing LaunchAgent...");

        if let Err(e) = super::launchd::install().await {
            print_warning(&format!("Could not install LaunchAgent: {}", e));
        }
    }

    // Step 5: Notification icon
    section("Step 5: Notification Icon (Optional)");

    if has_notifier {
        println!("terminal-notifier supports custom notification icons.\n");

        let icon_installed = check_notification_icon();
        if icon_installed {
            print_success("Custom notification icon is installed");
        } else {
            print_info("No custom notification icon found");
            println!();
            println!("    To add a custom icon, place a PNG file at one of:");
            println!("    - ~/Library/Application Support/voxtype/icon.png");
            println!("    - ~/.config/voxtype/icon.png");
            println!();

            let install_default_icon = prompt_yn("Install a default microphone icon?", true);
            if install_default_icon {
                if let Err(e) = install_default_icon_file() {
                    print_warning(&format!("Could not install icon: {}", e));
                } else {
                    print_success("Default icon installed");
                }
            }
        }
    } else {
        print_info("Install terminal-notifier to enable custom notification icons");
    }

    // Step 6: Model download
    section("Step 6: Whisper Model");

    // Load config to get current model
    let config = crate::config::load_config(None).unwrap_or_default();
    let current_model = &config.whisper.model;

    println!("Voxtype uses Whisper for speech recognition.\n");
    println!("Available models (from fastest to most accurate):");
    println!("  tiny.en    - Fastest, English only (~75 MB)");
    println!("  base.en    - Fast, English only (~145 MB)");
    println!("  small.en   - Balanced, English only (~500 MB)");
    println!("  medium.en  - Accurate, English only (~1.5 GB)");
    println!("  large-v3-turbo - Most accurate, all languages (~1.6 GB)");
    println!();
    println!("Current model: {}", current_model);

    let model = prompt("\nModel to use", current_model);

    // Check if model is downloaded
    let models_dir = crate::Config::models_dir();
    let model_filename = crate::transcribe::whisper::get_model_filename(&model);
    let model_path = models_dir.join(&model_filename);

    if model_path.exists() {
        print_success(&format!("Model '{}' is already downloaded", model));
    } else {
        let download = prompt_yn(&format!("Download model '{}'?", model), true);
        if download {
            println!();
            println!("Downloading model... (this may take a while)");
            if let Err(e) = super::model::download_model(&model) {
                print_failure(&format!("Download failed: {}", e));
            } else {
                print_success("Model downloaded successfully");

                // Update config to use the new model
                if let Err(e) = super::model::set_model_config(&model) {
                    print_warning(&format!("Could not update config: {}", e));
                }
            }
        }
    }

    // Summary
    section("Setup Complete!");

    println!("Your voxtype installation is ready. Here's a summary:\n");

    if use_hammerspoon {
        println!("  Hotkey method:  Hammerspoon");
        println!("  Hotkey:         {} ({})", hotkey, if toggle_mode { "toggle" } else { "push-to-talk" });
    } else {
        println!("  Hotkey method:  Native (rdev)");
        println!("  Hotkey:         {} ({})", hotkey, if toggle_mode { "toggle" } else { "push-to-talk" });
    }
    println!("  Model:          {}", model);
    println!("  Menu bar:       {}", if setup_menubar { "enabled" } else { "disabled" });
    println!("  Auto-start:     {}", if setup_autostart { "enabled" } else { "disabled" });

    println!("\n\x1b[1mNext steps:\x1b[0m\n");

    if !setup_autostart {
        println!("  1. Start the daemon:        voxtype daemon");
    } else {
        println!("  1. The daemon will start automatically (or run: voxtype daemon)");
    }

    if setup_menubar {
        println!("  2. Start the menu bar:      voxtype menubar");
    }

    if use_hammerspoon {
        println!("  3. Reload Hammerspoon config (click menu bar icon → Reload Config)");
    }

    println!();
    println!("Then just press {} to start recording!", hotkey);

    if !has_accessibility && !use_hammerspoon {
        println!();
        print_warning("Don't forget to grant Accessibility permission to your terminal!");
    }

    Ok(())
}
