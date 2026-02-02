//! macOS interactive setup wizard
//!
//! Provides a guided setup experience for macOS users, covering:
//! - App bundle creation and code signing
//! - Microphone permission (required for audio capture)
//! - Accessibility permission (required for text injection)
//! - Notification permission (optional)
//! - Hotkey configuration (native rdev or Hammerspoon)
//! - LaunchAgent auto-start
//! - Model download

use super::{print_failure, print_info, print_success, print_warning};
use std::io::{self, Write};
use std::path::PathBuf;

const APP_BUNDLE_PATH: &str = "/Applications/Voxtype.app";
const BUNDLE_IDENTIFIER: &str = "io.voxtype";

/// Check if the app bundle exists and is properly signed
fn check_app_bundle() -> bool {
    let app_path = PathBuf::from(APP_BUNDLE_PATH);
    let binary_path = app_path.join("Contents/MacOS/voxtype");
    let info_plist = app_path.join("Contents/Info.plist");

    app_path.exists() && binary_path.exists() && info_plist.exists()
}

/// Create the app bundle with proper Info.plist for permissions
async fn create_app_bundle() -> anyhow::Result<()> {
    let app_path = PathBuf::from(APP_BUNDLE_PATH);
    let contents_path = app_path.join("Contents");
    let macos_path = contents_path.join("MacOS");
    let resources_path = contents_path.join("Resources");

    // Create directory structure
    std::fs::create_dir_all(&macos_path)?;
    std::fs::create_dir_all(&resources_path)?;

    // Get current binary path
    let current_exe = std::env::current_exe()?;
    let binary_dest = macos_path.join("voxtype");

    // Copy binary to app bundle
    std::fs::copy(&current_exe, &binary_dest)?;

    // Get version from Cargo
    let version = env!("CARGO_PKG_VERSION");

    // Create Info.plist with all required permission descriptions
    let info_plist = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>voxtype</string>
    <key>CFBundleIdentifier</key>
    <string>{}</string>
    <key>CFBundleName</key>
    <string>Voxtype</string>
    <key>CFBundleDisplayName</key>
    <string>Voxtype</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>{}</string>
    <key>CFBundleVersion</key>
    <string>{}</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>Voxtype needs microphone access to capture your voice for speech-to-text transcription.</string>
    <key>NSAppleEventsUsageDescription</key>
    <string>Voxtype needs to send keystrokes to type transcribed text into applications.</string>
</dict>
</plist>
"#, BUNDLE_IDENTIFIER, version, version);

    std::fs::write(contents_path.join("Info.plist"), info_plist)?;

    // Copy icon if available
    if let Some(data_dir) = directories::BaseDirs::new().map(|d| d.data_dir().join("voxtype")) {
        let icon_src = data_dir.join("icon.png");
        if icon_src.exists() {
            let _ = std::fs::copy(&icon_src, resources_path.join("icon.png"));
        }
    }

    // Sign the app bundle
    let sign_result = tokio::process::Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-", APP_BUNDLE_PATH])
        .output()
        .await;

    match sign_result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Code signing failed: {}", stderr)
        }
        Err(e) => anyhow::bail!("Failed to run codesign: {}", e),
    }
}

/// Reset TCC permissions for Voxtype (forces re-prompt)
async fn reset_permissions() -> bool {
    let mic_reset = tokio::process::Command::new("tccutil")
        .args(["reset", "Microphone", BUNDLE_IDENTIFIER])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    let acc_reset = tokio::process::Command::new("tccutil")
        .args(["reset", "Accessibility", BUNDLE_IDENTIFIER])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    mic_reset || acc_reset
}

/// Check if microphone permission is granted
/// This is tricky - we can't directly check, but we can try to access an audio device
async fn check_microphone_permission() -> bool {
    // Use a simple AppleScript to check if we can access audio input
    // This isn't perfect but gives a reasonable indication
    let output = tokio::process::Command::new("osascript")
        .args(["-e", "do shell script \"echo test\""])
        .output()
        .await;

    // For now, we'll assume permission is needed and guide the user
    // The real check happens when the daemon tries to capture audio
    output.map(|o| o.status.success()).unwrap_or(false)
}

/// Check if Accessibility permission is granted using AXIsProcessTrusted equivalent
async fn check_accessibility_permission() -> bool {
    // Try to use osascript to control System Events - this requires Accessibility
    let output = tokio::process::Command::new("osascript")
        .args(["-e", "tell application \"System Events\" to return name of first process"])
        .output()
        .await;

    match output {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

/// Open System Settings to a specific privacy pane
async fn open_privacy_settings(pane: &str) -> bool {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?Privacy_{}", pane);

    tokio::process::Command::new("open")
        .arg(&url)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if Voxtype is in the Accessibility list (even if disabled)
async fn is_in_accessibility_list() -> bool {
    // Check the TCC database - this is a heuristic
    let output = tokio::process::Command::new("sqlite3")
        .args([
            "/Library/Application Support/com.apple.TCC/TCC.db",
            &format!("SELECT client FROM access WHERE client='{}' AND service='kTCCServiceAccessibility'", BUNDLE_IDENTIFIER),
        ])
        .output()
        .await;

    // If we can't query (permission denied), check user database
    if output.is_err() || !output.as_ref().unwrap().status.success() {
        if let Some(home) = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()) {
            let user_db = home.join("Library/Application Support/com.apple.TCC/TCC.db");
            let output = tokio::process::Command::new("sqlite3")
                .args([
                    user_db.to_str().unwrap_or(""),
                    &format!("SELECT client FROM access WHERE client='{}' AND service='kTCCServiceAccessibility'", BUNDLE_IDENTIFIER),
                ])
                .output()
                .await;

            return output.map(|o| !o.stdout.is_empty()).unwrap_or(false);
        }
    }

    output.map(|o| !o.stdout.is_empty()).unwrap_or(false)
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

/// Check if system language is English
async fn is_system_language_english() -> bool {
    let output = tokio::process::Command::new("defaults")
        .args(["read", "NSGlobalDomain", "AppleLanguages"])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            let languages = String::from_utf8_lossy(&o.stdout);
            languages
                .lines()
                .find(|line| line.trim().starts_with('"'))
                .map(|line| {
                    let trimmed = line.trim().trim_matches(|c| c == '"' || c == ',');
                    trimmed.starts_with("en")
                })
                .unwrap_or(true)
        }
        _ => true,
    }
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

/// Wait for user to press Enter
fn wait_for_enter(message: &str) {
    print!("{}", message);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
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
    let data_dir = dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find Application Support directory"))?
        .join("voxtype");

    std::fs::create_dir_all(&data_dir)?;

    let icon_path = data_dir.join("icon.png");
    let icon_data = include_bytes!("../../assets/icon.png");
    std::fs::write(&icon_path, icon_data)?;

    println!("    Installed icon: {}", icon_path.display());
    Ok(())
}

/// Get the app bundle binary path
pub fn get_app_bundle_path() -> String {
    format!("{}/Contents/MacOS/voxtype", APP_BUNDLE_PATH)
}

/// Run the macOS setup wizard
pub async fn run() -> anyhow::Result<()> {
    println!("\x1b[1mVoxtype macOS Setup Wizard\x1b[0m");
    println!("==========================\n");
    println!("This wizard will set up Voxtype as a native macOS app with proper permissions.\n");

    // Step 1: Create App Bundle
    section("Step 1: App Bundle");

    let app_exists = check_app_bundle();
    if app_exists {
        print_success("Voxtype.app already exists");
        let recreate = prompt_yn("Recreate app bundle? (recommended after updates)", true);
        if recreate {
            println!("  Creating app bundle...");
            match create_app_bundle().await {
                Ok(_) => print_success("App bundle created and signed"),
                Err(e) => {
                    print_failure(&format!("Failed to create app bundle: {}", e));
                    println!("    You may need to run with sudo or manually create the bundle");
                    return Err(e);
                }
            }
        }
    } else {
        println!("Voxtype needs to be installed as an app bundle for proper macOS integration.\n");
        println!("This will:");
        println!("  - Create /Applications/Voxtype.app");
        println!("  - Enable proper permission prompts");
        println!("  - Allow adding to Login Items\n");

        let create = prompt_yn("Create app bundle?", true);
        if create {
            println!("  Creating app bundle...");
            match create_app_bundle().await {
                Ok(_) => print_success("App bundle created and signed"),
                Err(e) => {
                    print_failure(&format!("Failed to create app bundle: {}", e));
                    println!("    You may need to run with sudo or manually create the bundle");
                    return Err(e);
                }
            }
        } else {
            print_warning("Skipping app bundle creation");
            println!("    Note: Without the app bundle, permissions may not work correctly");
        }
    }

    // Step 2: Microphone Permission
    section("Step 2: Microphone Permission");

    println!("Voxtype needs microphone access to capture your voice.\n");
    println!("We'll open System Settings and launch Voxtype to trigger the permission prompt.\n");

    let setup_mic = prompt_yn("Set up microphone permission now?", true);
    if setup_mic {
        // Reset permissions to ensure a fresh prompt
        let _ = reset_permissions().await;

        // Open System Settings to Microphone
        print_info("Opening System Settings > Privacy & Security > Microphone...");
        open_privacy_settings("Microphone").await;

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        // Launch the app to trigger the permission prompt
        print_info("Launching Voxtype to trigger permission prompt...");
        let app_binary = get_app_bundle_path();
        let _ = tokio::process::Command::new(&app_binary)
            .arg("daemon")
            .spawn();

        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        println!();
        println!("    \x1b[1mAction required:\x1b[0m");
        println!("    1. If a permission dialog appears, click 'OK' to allow microphone access");
        println!("    2. If no dialog appears, find 'Voxtype' in the list and toggle it ON");
        println!("    3. If Voxtype isn't in the list, press the hotkey once to trigger the prompt");
        println!();

        wait_for_enter("Press Enter when microphone permission is granted...");

        // Kill the test daemon
        let _ = tokio::process::Command::new("pkill")
            .args(["-f", "voxtype"])
            .output()
            .await;

        print_success("Microphone permission configured");
    }

    // Step 3: Input Monitoring Permission (for hotkey capture)
    section("Step 3: Input Monitoring Permission");

    println!("Voxtype needs Input Monitoring permission to capture global hotkeys.\n");

    let setup_input = prompt_yn("Set up Input Monitoring permission now?", true);
    if setup_input {
        print_info("Opening System Settings > Privacy & Security > Input Monitoring...");
        open_privacy_settings("ListenEvent").await;

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        println!();
        println!("    \x1b[1mAction required:\x1b[0m");
        println!("    1. Click the '+' button");
        println!("    2. Navigate to /Applications");
        println!("    3. Select 'Voxtype.app'");
        println!("    4. Ensure the toggle is ON");
        println!();

        wait_for_enter("Press Enter when Input Monitoring permission is granted...");
        print_success("Input Monitoring permission configured");
    }

    // Step 4: Accessibility Permission (for text injection)
    section("Step 4: Accessibility Permission");

    println!("Voxtype needs Accessibility permission to type transcribed text.\n");

    let has_accessibility = check_accessibility_permission().await;

    if has_accessibility {
        print_success("Accessibility permission already granted");
    } else {
        let setup_acc = prompt_yn("Set up Accessibility permission now?", true);
        if setup_acc {
            print_info("Opening System Settings > Privacy & Security > Accessibility...");
            open_privacy_settings("Accessibility").await;

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            println!();
            println!("    \x1b[1mAction required:\x1b[0m");
            println!("    1. Click the '+' button");
            println!("    2. Navigate to /Applications");
            println!("    3. Select 'Voxtype.app'");
            println!("    4. Ensure the toggle is ON");
            println!();

            wait_for_enter("Press Enter when Accessibility permission is granted...");

            // Verify permission
            let has_acc_now = check_accessibility_permission().await;
            if has_acc_now {
                print_success("Accessibility permission granted");
            } else {
                print_warning("Accessibility permission may not be fully configured");
                println!("    Text typing will fall back to clipboard if needed");
            }
        }
    }

    // Step 5: Notification Permission (Optional)
    section("Step 5: Notifications (Optional)");

    let has_notifier = check_terminal_notifier().await;

    println!("Voxtype can show notifications when transcription completes.\n");

    if has_notifier {
        print_success("terminal-notifier installed (enhanced notifications)");
    } else {
        print_info("terminal-notifier not installed");
        println!("       Install with: brew install terminal-notifier");
    }

    let setup_notifications = prompt_yn("Configure notification permission?", false);
    if setup_notifications {
        print_info("Opening System Settings > Notifications...");
        let _ = tokio::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.notifications")
            .output()
            .await;

        println!();
        println!("    Find 'Voxtype' in the list and configure notification settings.");
        println!();

        wait_for_enter("Press Enter when done...");
    }

    // Step 6: Hotkey Configuration
    section("Step 6: Hotkey Configuration");

    let has_hammerspoon = check_hammerspoon().await;

    println!("Voxtype supports two methods for global hotkey capture:\n");
    println!("  1. Native (rdev) - Built-in, requires Accessibility permission");
    println!("  2. Hammerspoon   - External app, more reliable on some systems\n");

    if has_hammerspoon {
        print_success("Hammerspoon is installed");
    } else {
        print_info("Hammerspoon is not installed (optional)");
        println!("       Install with: brew install --cask hammerspoon");
    }

    let use_hammerspoon = if has_hammerspoon {
        println!();
        prompt_yn("Use Hammerspoon for hotkey support?", false)
    } else {
        false
    };

    let hotkey = prompt("\nHotkey to use", "rightalt");
    let toggle_mode = prompt_yn("Use toggle mode? (press to start/stop instead of hold)", false);

    if use_hammerspoon {
        println!();
        println!("Setting up Hammerspoon integration...");

        if let Err(e) = super::hammerspoon::run(true, false, &hotkey, toggle_mode).await {
            print_warning(&format!("Could not set up Hammerspoon: {}", e));
        }
    } else {
        print_success(&format!("Configured native hotkey: {}", hotkey));
        print_info(&format!("Mode: {}", if toggle_mode { "toggle" } else { "push-to-talk" }));
    }

    // Step 7: Auto-start
    section("Step 7: Auto-start Configuration");

    println!("Voxtype can start automatically when you log in.\n");

    let setup_autostart = prompt_yn("Set up auto-start (LaunchAgent)?", true);

    if setup_autostart {
        println!();
        println!("Installing LaunchAgent...");

        // First, update launchd to use the app bundle path
        if let Err(e) = install_launchd_with_app_bundle().await {
            print_warning(&format!("Could not install LaunchAgent: {}", e));
        } else {
            print_success("LaunchAgent installed");
        }
    }

    // Step 8: Notification icon
    section("Step 8: Notification Icon (Optional)");

    if has_notifier {
        println!("terminal-notifier supports custom notification icons.\n");

        let icon_installed = check_notification_icon();
        if icon_installed {
            print_success("Custom notification icon is installed");
        } else {
            print_info("No custom notification icon found");

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

    // Step 9: Model download
    section("Step 9: Speech Recognition Model");

    let config = crate::config::load_config(None).unwrap_or_default();
    let models_dir = crate::Config::models_dir();

    #[cfg(feature = "parakeet")]
    let has_parakeet = true;
    #[cfg(not(feature = "parakeet"))]
    let has_parakeet = false;

    let is_english = is_system_language_english().await;

    let (use_parakeet, model) = if has_parakeet {
        println!("Voxtype supports two speech recognition engines:\n");

        if is_english {
            println!("  1. Parakeet (Recommended) - NVIDIA's FastConformer via CoreML");
            println!("     - ~8x faster than Whisper on Apple Silicon");
            println!("     - Optimized for macOS Neural Engine");
            println!("     - English only");
            println!();
            println!("  2. Whisper - OpenAI's Whisper via whisper.cpp");
            println!("     - Broader language support");
            println!("     - More model size options");
        } else {
            println!("  1. Whisper (Recommended) - OpenAI's Whisper via whisper.cpp");
            println!("     - Supports your system language");
            println!("     - Multiple model sizes available");
            println!();
            println!("  2. Parakeet - NVIDIA's FastConformer via CoreML");
            println!("     - ~8x faster on Apple Silicon");
            println!("     - English only");
            print_warning("Your system language is not English. Parakeet only supports English.");
        }
        println!();

        let use_parakeet = prompt_yn("Use Parakeet?", is_english);

        if use_parakeet {
            println!();
            println!("Available Parakeet models:");
            println!("  parakeet-tdt-0.6b-v3      - Full precision (~1.2 GB)");
            println!("  parakeet-tdt-0.6b-v3-int8 - Quantized, faster (~670 MB)");
            println!();

            let current = config.parakeet.as_ref()
                .map(|p| p.model.as_str())
                .unwrap_or("parakeet-tdt-0.6b-v3-int8");
            let model = prompt("Model to use", current);
            (true, model)
        } else {
            println!();
            println!("Available Whisper models (from fastest to most accurate):");
            if is_english {
                println!("  tiny.en        - Fastest, English only (~75 MB)");
                println!("  base.en        - Fast, English only (~145 MB)");
                println!("  small.en       - Balanced, English only (~500 MB)");
                println!("  medium.en      - Accurate, English only (~1.5 GB)");
                println!("  large-v3-turbo - Most accurate, all languages (~1.6 GB)");
            } else {
                println!("  tiny             - Fastest, multilingual (~75 MB)");
                println!("  base             - Fast, multilingual (~145 MB)");
                println!("  small            - Balanced, multilingual (~500 MB)");
                println!("  medium           - Accurate, multilingual (~1.5 GB)");
                println!("  large-v3-turbo   - Most accurate, all languages (~1.6 GB)");
            }
            println!();

            let default_model = if is_english {
                config.whisper.model.as_str()
            } else {
                "large-v3-turbo"
            };
            let model = prompt("Model to use", default_model);
            (false, model)
        }
    } else {
        println!("Voxtype uses Whisper for speech recognition.\n");
        println!("Available models (from fastest to most accurate):");
        if is_english {
            println!("  tiny.en        - Fastest, English only (~75 MB)");
            println!("  base.en        - Fast, English only (~145 MB)");
            println!("  small.en       - Balanced, English only (~500 MB)");
            println!("  medium.en      - Accurate, English only (~1.5 GB)");
            println!("  large-v3-turbo - Most accurate, all languages (~1.6 GB)");
        } else {
            println!("  tiny             - Fastest, multilingual (~75 MB)");
            println!("  base             - Fast, multilingual (~145 MB)");
            println!("  small            - Balanced, multilingual (~500 MB)");
            println!("  medium           - Accurate, multilingual (~1.5 GB)");
            println!("  large-v3-turbo   - Most accurate, all languages (~1.6 GB)");
        }
        println!();

        let default_model = if is_english {
            config.whisper.model.as_str()
        } else {
            "large-v3-turbo"
        };
        let model = prompt("Model to use", default_model);
        (false, model)
    };

    // Download and configure the selected model
    if use_parakeet {
        let model_path = models_dir.join(&model);
        let model_valid = model_path.exists()
            && super::model::validate_parakeet_model(&model_path).is_ok();

        if model_valid {
            print_success(&format!("Model '{}' is already downloaded", model));
        } else {
            let download = prompt_yn(&format!("Download model '{}'?", model), true);
            if download {
                println!();
                println!("Downloading model... (this may take a while)");
                if let Err(e) = super::model::download_parakeet_model(&model) {
                    print_failure(&format!("Download failed: {}", e));
                } else {
                    print_success("Model downloaded successfully");
                }
            }
        }

        if let Err(e) = super::model::set_parakeet_config(&model) {
            print_warning(&format!("Could not update config: {}", e));
        } else {
            print_success("Config updated to use Parakeet engine");
        }
    } else {
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

                    if let Err(e) = super::model::set_model_config(&model) {
                        print_warning(&format!("Could not update config: {}", e));
                    }
                }
            }
        }
    }

    let engine_name = if use_parakeet { "Parakeet" } else { "Whisper" };

    // Summary
    section("Setup Complete!");

    println!("Your Voxtype installation is ready. Here's a summary:\n");

    println!("  App bundle:     /Applications/Voxtype.app");
    if use_hammerspoon {
        println!("  Hotkey method:  Hammerspoon");
    } else {
        println!("  Hotkey method:  Native (rdev)");
    }
    println!("  Hotkey:         {} ({})", hotkey, if toggle_mode { "toggle" } else { "push-to-talk" });
    println!("  Engine:         {}", engine_name);
    println!("  Model:          {}", model);
    println!("  Auto-start:     {}", if setup_autostart { "enabled" } else { "disabled" });

    println!("\n\x1b[1mStarting Voxtype...\x1b[0m\n");

    // Start the daemon
    if setup_autostart {
        let _ = tokio::process::Command::new("launchctl")
            .args(["load", &format!("{}/Library/LaunchAgents/io.voxtype.daemon.plist",
                   dirs::home_dir().map(|h| h.to_string_lossy().to_string()).unwrap_or_default())])
            .output()
            .await;
        print_success("Daemon started via LaunchAgent");
    } else {
        let app_binary = get_app_bundle_path();
        let _ = tokio::process::Command::new(&app_binary)
            .arg("daemon")
            .spawn();
        print_success("Daemon started");
    }

    println!();
    println!("Press {} to start recording!", hotkey);
    println!();
    println!("Useful commands:");
    println!("  voxtype status            - Check daemon status");
    println!("  voxtype status --follow   - Watch status in real-time");
    println!("  voxtype record toggle     - Toggle recording from CLI");

    Ok(())
}

/// Install LaunchAgent configured to use the app bundle
async fn install_launchd_with_app_bundle() -> anyhow::Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let launch_agents_dir = home.join("Library/LaunchAgents");
    let logs_dir = home.join("Library/Logs/voxtype");

    std::fs::create_dir_all(&launch_agents_dir)?;
    std::fs::create_dir_all(&logs_dir)?;

    let app_binary = get_app_bundle_path();

    let plist_content = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>io.voxtype.daemon</string>

    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>daemon</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>{}/stdout.log</string>

    <key>StandardErrorPath</key>
    <string>{}/stderr.log</string>

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
"#, app_binary, logs_dir.display(), logs_dir.display());

    let plist_path = launch_agents_dir.join("io.voxtype.daemon.plist");

    // Unload existing if present
    let _ = tokio::process::Command::new("launchctl")
        .args(["unload", plist_path.to_str().unwrap_or("")])
        .output()
        .await;

    std::fs::write(&plist_path, plist_content)?;

    print_success(&format!("Created: {}", plist_path.display()));
    print_success(&format!("Logs: {}", logs_dir.display()));

    Ok(())
}
