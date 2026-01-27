//! Hammerspoon integration setup for macOS
//!
//! Helps users configure Hammerspoon for hotkey support as an alternative
//! to the built-in rdev-based hotkey capture.

use std::path::PathBuf;

/// Generate the Hammerspoon init.lua snippet
fn generate_config(hotkey: &str, toggle: bool) -> String {
    let mode = if toggle { "toggle" } else { "push_to_talk" };
    format!(
        r#"-- Voxtype Hammerspoon Integration
-- Add this to your ~/.hammerspoon/init.lua

local voxtype = require("voxtype")
voxtype.setup({{
    hotkey = "{}",
    mode = "{}",
}})

-- Optional: Add a cancel hotkey
-- voxtype.add_cancel_hotkey({{"cmd", "shift"}}, "escape")
"#,
        hotkey, mode
    )
}

/// Get the path to the Hammerspoon config directory
fn hammerspoon_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".hammerspoon"))
}

/// Check if Hammerspoon is installed
async fn is_hammerspoon_installed() -> bool {
    tokio::process::Command::new("which")
        .arg("hs")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
        || std::path::Path::new("/Applications/Hammerspoon.app").exists()
}

/// Install the voxtype.lua module to ~/.hammerspoon/
async fn install_module() -> anyhow::Result<()> {
    let hs_dir = hammerspoon_dir().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;

    // Create .hammerspoon directory if needed
    if !hs_dir.exists() {
        std::fs::create_dir_all(&hs_dir)?;
        println!("Created {}", hs_dir.display());
    }

    // Write the voxtype.lua module
    let module_path = hs_dir.join("voxtype.lua");
    let module_content = include_str!("../../contrib/hammerspoon/voxtype.lua");
    std::fs::write(&module_path, module_content)?;
    println!("Installed {}", module_path.display());

    Ok(())
}

/// Run the Hammerspoon setup command
pub async fn run(install: bool, show: bool, hotkey: &str, toggle: bool) -> anyhow::Result<()> {
    println!("Hammerspoon Integration for Voxtype");
    println!("====================================\n");

    // Show config if requested (even without Hammerspoon installed)
    if show {
        println!("Add the following to your ~/.hammerspoon/init.lua:\n");
        println!("{}", generate_config(hotkey, toggle));
        return Ok(());
    }

    // Check if Hammerspoon is installed for other actions
    if !is_hammerspoon_installed().await {
        println!("Hammerspoon is not installed.\n");
        println!("Install it with:");
        println!("  brew install --cask hammerspoon\n");
        println!("Then run this command again.\n");
        println!("Or use --show to see the config snippet anyway.");
        return Ok(());
    }

    if install {
        // Install the module
        install_module().await?;
        println!();
        println!("Now add the following to your ~/.hammerspoon/init.lua:");
        println!();
        println!("{}", generate_config(hotkey, toggle));
        println!();
        println!("Then reload Hammerspoon config:");
        println!("  - Click Hammerspoon menu bar icon -> Reload Config");
        println!("  - Or press Cmd+Shift+R while Hammerspoon console is focused");
    } else if show {
        // Just show the config
        println!("Add the following to your ~/.hammerspoon/init.lua:\n");
        println!("{}", generate_config(hotkey, toggle));
    } else {
        // Default: show instructions
        println!("Hammerspoon provides hotkey support without granting Accessibility");
        println!("permissions to Terminal.\n");

        println!("Setup options:\n");
        println!("  voxtype setup hammerspoon --install");
        println!("      Install voxtype.lua module and show config snippet\n");
        println!("  voxtype setup hammerspoon --show");
        println!("      Show the init.lua configuration snippet\n");
        println!("  voxtype setup hammerspoon --install --hotkey rightcmd");
        println!("      Install with a different hotkey\n");
        println!("  voxtype setup hammerspoon --install --toggle");
        println!("      Use toggle mode (press to start/stop) instead of push-to-talk\n");

        println!("Available hotkeys:");
        println!("  rightalt, leftalt, rightcmd, leftcmd, rightctrl, leftctrl");
        println!("  rightshift, leftshift, f1-f20, escape, space, tab, etc.\n");

        println!("Current Hammerspoon directory: {}",
            hammerspoon_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not found".to_string())
        );
    }

    Ok(())
}
