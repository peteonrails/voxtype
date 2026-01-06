//! Setup module for voxtype installation and configuration
//!
//! Provides subcommands for:
//! - systemd service installation
//! - Waybar configuration generation
//! - Interactive model selection
//! - Output chain detection
//! - GPU backend management

pub mod gpu;
pub mod model;
pub mod systemd;
pub mod waybar;

use crate::config::Config;
use std::process::Stdio;
use tokio::process::Command;

/// Display server type
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayServer {
    Wayland,
    X11,
    Unknown,
}

impl std::fmt::Display for DisplayServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisplayServer::Wayland => write!(f, "Wayland"),
            DisplayServer::X11 => write!(f, "X11"),
            DisplayServer::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Output tool status
#[derive(Debug)]
pub struct OutputToolStatus {
    pub name: &'static str,
    pub installed: bool,
    pub available: bool,
    pub path: Option<String>,
    pub note: Option<String>,
}

/// Complete output chain status
#[derive(Debug)]
pub struct OutputChainStatus {
    pub display_server: DisplayServer,
    pub wtype: OutputToolStatus,
    pub ydotool: OutputToolStatus,
    pub ydotool_daemon: bool,
    pub wl_copy: OutputToolStatus,
    pub xclip: OutputToolStatus,
    pub primary_method: Option<String>,
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

/// Print a warning message
pub fn print_warning(msg: &str) {
    println!("  \x1b[33m⚠\x1b[0m {}", msg);
}

/// Detect the current display server
pub fn detect_display_server() -> DisplayServer {
    // Check for Wayland first
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        return DisplayServer::Wayland;
    }
    // Check for X11
    if std::env::var("DISPLAY").is_ok() {
        return DisplayServer::X11;
    }
    DisplayServer::Unknown
}

/// Get the path to a command if it exists
pub async fn get_command_path(cmd: &str) -> Option<String> {
    let output = Command::new("which")
        .arg(cmd)
        .output()
        .await
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Check if ydotool daemon is running
pub async fn is_ydotool_daemon_running() -> bool {
    // Try using systemctl first
    let systemctl_check = Command::new("systemctl")
        .args(["--user", "is-active", "ydotool"])
        .output()
        .await;

    if let Ok(output) = systemctl_check {
        if output.status.success() {
            return true;
        }
    }

    // Fallback: try a no-op ydotool command
    Command::new("ydotool")
        .args(["type", ""])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Detect the full output chain status
pub async fn detect_output_chain() -> OutputChainStatus {
    let display_server = detect_display_server();

    // Check wtype
    let wtype_path = get_command_path("wtype").await;
    let wtype_installed = wtype_path.is_some();
    let wtype_available = wtype_installed && display_server == DisplayServer::Wayland;
    let wtype_note = if wtype_installed && display_server != DisplayServer::Wayland {
        Some("Wayland only".to_string())
    } else {
        None
    };

    // Check ydotool
    let ydotool_path = get_command_path("ydotool").await;
    let ydotool_installed = ydotool_path.is_some();
    let ydotool_daemon = if ydotool_installed {
        is_ydotool_daemon_running().await
    } else {
        false
    };
    let ydotool_available = ydotool_installed && ydotool_daemon;
    let ydotool_note = if ydotool_installed && !ydotool_daemon {
        Some("daemon not running".to_string())
    } else {
        None
    };

    // Check wl-copy
    let wl_copy_path = get_command_path("wl-copy").await;
    let wl_copy_installed = wl_copy_path.is_some();
    let wl_copy_available = wl_copy_installed && display_server == DisplayServer::Wayland;
    let wl_copy_note = if wl_copy_installed && display_server != DisplayServer::Wayland {
        Some("Wayland only".to_string())
    } else {
        None
    };

    // Check xclip
    let xclip_path = get_command_path("xclip").await;
    let xclip_installed = xclip_path.is_some();
    let xclip_available = xclip_installed && display_server == DisplayServer::X11;
    let xclip_note = if xclip_installed && display_server != DisplayServer::X11 {
        Some("X11 only".to_string())
    } else {
        None
    };

    // Determine primary method
    let primary_method = if wtype_available {
        Some("wtype".to_string())
    } else if ydotool_available {
        Some("ydotool".to_string())
    } else if wl_copy_available || xclip_available {
        Some("clipboard".to_string())
    } else {
        None
    };

    OutputChainStatus {
        display_server,
        wtype: OutputToolStatus {
            name: "wtype",
            installed: wtype_installed,
            available: wtype_available,
            path: wtype_path,
            note: wtype_note,
        },
        ydotool: OutputToolStatus {
            name: "ydotool",
            installed: ydotool_installed,
            available: ydotool_available,
            path: ydotool_path,
            note: ydotool_note,
        },
        ydotool_daemon,
        wl_copy: OutputToolStatus {
            name: "wl-copy",
            installed: wl_copy_installed,
            available: wl_copy_available,
            path: wl_copy_path,
            note: wl_copy_note,
        },
        xclip: OutputToolStatus {
            name: "xclip",
            installed: xclip_installed,
            available: xclip_available,
            path: xclip_path,
            note: xclip_note,
        },
        primary_method,
    }
}

/// Print output chain status
pub fn print_output_chain_status(status: &OutputChainStatus) {
    println!("\nOutput Chain:");

    // Display server
    let ds_info = match status.display_server {
        DisplayServer::Wayland => {
            let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
            format!("Wayland (WAYLAND_DISPLAY={})", display)
        }
        DisplayServer::X11 => {
            let display = std::env::var("DISPLAY").unwrap_or_default();
            format!("X11 (DISPLAY={})", display)
        }
        DisplayServer::Unknown => "Unknown (no WAYLAND_DISPLAY or DISPLAY set)".to_string(),
    };
    println!("  Display server:  {}", ds_info);

    // wtype
    print_tool_status(&status.wtype, status.display_server == DisplayServer::Wayland);

    // ydotool
    if status.ydotool.installed {
        let daemon_status = if status.ydotool_daemon {
            "\x1b[32mdaemon running\x1b[0m"
        } else {
            "\x1b[31mdaemon not running\x1b[0m"
        };
        if let Some(ref path) = status.ydotool.path {
            if status.ydotool.available {
                println!("  ydotool:         \x1b[32m✓\x1b[0m installed ({}), {}", path, daemon_status);
            } else {
                println!("  ydotool:         \x1b[33m⚠\x1b[0m installed ({}), {}", path, daemon_status);
            }
        }
    } else {
        println!("  ydotool:         \x1b[31m✗\x1b[0m not installed");
    }

    // wl-copy
    print_tool_status(&status.wl_copy, status.display_server == DisplayServer::Wayland);

    // xclip (only show on X11 or if installed)
    if status.display_server == DisplayServer::X11 || status.xclip.installed {
        print_tool_status(&status.xclip, status.display_server == DisplayServer::X11);
    }

    // Summary
    println!();
    if let Some(ref method) = status.primary_method {
        let method_desc = match method.as_str() {
            "wtype" => "wtype (CJK supported)",
            "ydotool" => "ydotool (CJK not supported)",
            "clipboard" => "clipboard (requires manual paste)",
            _ => method.as_str(),
        };
        println!("  \x1b[32m→\x1b[0m Text will be typed via {}", method_desc);
    } else {
        println!("  \x1b[31m→\x1b[0m No text output method available!");
        println!("    Install wtype (Wayland) or ydotool (X11) for typing support");
    }
}

fn print_tool_status(tool: &OutputToolStatus, is_relevant: bool) {
    if tool.installed {
        let path = tool.path.as_deref().unwrap_or("?");
        let note = tool.note.as_deref().map(|n| format!(" ({})", n)).unwrap_or_default();

        if tool.available {
            println!("  {}:{}  \x1b[32m✓\x1b[0m installed ({}){}",
                tool.name,
                " ".repeat(14 - tool.name.len()),
                path,
                note
            );
        } else if is_relevant {
            println!("  {}:{}  \x1b[33m⚠\x1b[0m installed ({}){}",
                tool.name,
                " ".repeat(14 - tool.name.len()),
                path,
                note
            );
        } else {
            println!("  {}:{}  \x1b[90m✓ installed ({}){}\x1b[0m",
                tool.name,
                " ".repeat(14 - tool.name.len()),
                path,
                note
            );
        }
    } else if is_relevant {
        println!("  {}:{}  \x1b[31m✗\x1b[0m not installed",
            tool.name,
            " ".repeat(14 - tool.name.len())
        );
    } else {
        println!("  {}:{}  \x1b[90m- not installed\x1b[0m",
            tool.name,
            " ".repeat(14 - tool.name.len())
        );
    }
}

/// Run setup tasks (non-blocking, no red X errors)
///
/// Flags:
/// - `quiet`: Suppress ALL output (for scripting/automation)
/// - `no_post_install`: Suppress only "Next steps" instructions
pub async fn run_setup(config: &Config, download: bool, quiet: bool, no_post_install: bool) -> anyhow::Result<()> {
    if !quiet {
        println!("Voxtype Setup\n");
        println!("=============\n");

        // Ensure directories exist first
        println!("Creating directories...");
    }
    Config::ensure_directories()?;
    if !quiet {
        print_success(&format!(
            "Config directory: {:?}",
            Config::config_dir().unwrap_or_default()
        ));
        print_success(&format!("Models directory: {:?}", Config::models_dir()));
    }

    // Create default config file if it doesn't exist
    if let Some(config_path) = Config::default_path() {
        if !config_path.exists() {
            if !quiet {
                println!("\nCreating default config file...");
            }
            std::fs::write(&config_path, crate::config::DEFAULT_CONFIG)?;
            if !quiet {
                print_success(&format!("Created: {:?}", config_path));
            }
        } else if !quiet {
            print_success(&format!("Config file: {:?}", config_path));
        }
    }

    // Check/download whisper model
    if !quiet {
        println!("\nWhisper model...");
    }
    let models_dir = Config::models_dir();
    let model_name = &config.whisper.model;
    let model_filename = crate::transcribe::whisper::get_model_filename(model_name);
    let model_path = models_dir.join(&model_filename);

    if model_path.exists() {
        if !quiet {
            let size = std::fs::metadata(&model_path)
                .map(|m| m.len() as f64 / 1024.0 / 1024.0)
                .unwrap_or(0.0);
            print_success(&format!(
                "Model ready: {} ({:.0} MB)",
                model_name, size
            ));
        }
    } else if download {
        if !quiet {
            println!("  Downloading {}...", model_name);
        }
        model::download_model(model_name)?;
    } else if !quiet {
        print_info(&format!("Model '{}' not downloaded yet", model_name));
        println!("       Run: voxtype setup --download");
    }

    // Summary
    if !quiet {
        println!("\n---");
        println!("\x1b[32m✓ Setup complete!\x1b[0m");
    }

    // Show next steps unless --quiet or --no-post-install is passed
    if !quiet && !no_post_install {
        println!();
        println!("Next steps:");
        println!("  1. Set up a compositor keybinding to trigger recording:");
        println!("     Example for Hyprland: bind = , XF86AudioRecord, exec, voxtype record-toggle\n");
        println!("  2. Start the daemon: voxtype daemon\n");
        println!("Optional:");
        println!("  voxtype setup check    - Verify system configuration");
        println!("  voxtype setup model    - Download/switch whisper models");
        println!("  voxtype setup systemd  - Install as systemd service");
        println!("  voxtype setup waybar   - Get Waybar integration config");
    }

    Ok(())
}

/// Run system checks (blocking, shows red X for failures)
pub async fn run_checks(config: &Config) -> anyhow::Result<()> {
    println!("Voxtype System Check\n");
    println!("====================\n");

    let mut all_ok = true;

    // Check CPU compatibility
    println!("CPU:");
    if let Some(warning) = crate::cpu::check_cpu_compatibility() {
        print_warning(&warning);
    } else {
        print_success("CPU features compatible");
    }
    if crate::cpu::is_running_in_vm() {
        print_info("Running in a virtual machine - ensure CPU features are passed through");
    }

    // Check directories
    println!("Directories:");
    if let Some(config_dir) = Config::config_dir() {
        if config_dir.exists() {
            print_success(&format!("Config directory: {:?}", config_dir));
        } else {
            print_failure(&format!("Config directory missing: {:?}", config_dir));
            println!("       Run: voxtype setup");
            all_ok = false;
        }
    }

    let models_dir = Config::models_dir();
    if models_dir.exists() {
        print_success(&format!("Models directory: {:?}", models_dir));
    } else {
        print_failure(&format!("Models directory missing: {:?}", models_dir));
        println!("       Run: voxtype setup");
        all_ok = false;
    }

    // Check config file
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            print_success(&format!("Config file: {:?}", config_path));
        } else {
            print_failure(&format!("Config file missing: {:?}", config_path));
            println!("       Run: voxtype setup");
            all_ok = false;
        }
    }

    // Check input group
    println!("\nInput:");
    if user_in_group("input") {
        print_success("User is in 'input' group (evdev hotkeys available)");
    } else {
        print_warning("User is not in 'input' group (evdev hotkeys unavailable)");
        println!("       Required only for evdev hotkey mode, not compositor keybindings");
        println!("       To enable: sudo usermod -aG input $USER && logout");
    }

    // Check output chain
    let output_status = detect_output_chain().await;
    print_output_chain_status(&output_status);

    if output_status.primary_method.is_none() {
        print_failure("No text output method available");
        if output_status.display_server == DisplayServer::Wayland {
            println!("       Install wtype: sudo pacman -S wtype");
        } else {
            println!("       Install ydotool: sudo pacman -S ydotool");
        }
        all_ok = false;
    } else if output_status.primary_method.as_deref() == Some("clipboard") {
        print_warning("Only clipboard mode available - typing won't work");
        if output_status.display_server == DisplayServer::Wayland {
            println!("       Install wtype: sudo pacman -S wtype");
        } else {
            println!("       Install ydotool: sudo pacman -S ydotool");
        }
    }

    // Check whisper model
    println!("\nWhisper Model:");
    let model_name = &config.whisper.model;
    let model_filename = crate::transcribe::whisper::get_model_filename(model_name);
    let model_path = models_dir.join(&model_filename);

    if model_path.exists() {
        let size = std::fs::metadata(&model_path)
            .map(|m| m.len() as f64 / 1024.0 / 1024.0)
            .unwrap_or(0.0);
        print_success(&format!("Model '{}' installed ({:.0} MB)", model_name, size));
    } else {
        print_failure(&format!("Model '{}' not found", model_name));
        println!("       Run: voxtype setup --download");
        all_ok = false;
    }

    // Summary
    println!("\n---");
    if all_ok {
        println!("\x1b[32m✓ All checks passed!\x1b[0m");
    } else {
        println!("\x1b[31m✗ Some checks failed.\x1b[0m Please fix the issues above.");
    }

    Ok(())
}
