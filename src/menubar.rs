//! macOS menu bar integration
//!
//! Provides a system tray icon showing voxtype status with a context menu
//! for controlling recording and configuring settings.

use crate::config::{ActivationMode, Config, OutputMode, TranscriptionEngine};
use pidlock::Pidlock;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    TrayIconBuilder,
};

/// Current voxtype state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxtypeState {
    Idle,
    Recording,
    Transcribing,
    Stopped,
}

impl VoxtypeState {
    fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "idle" => VoxtypeState::Idle,
            "recording" => VoxtypeState::Recording,
            "transcribing" => VoxtypeState::Transcribing,
            _ => VoxtypeState::Stopped,
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            VoxtypeState::Idle => "🎙",
            VoxtypeState::Recording => "🔴",
            VoxtypeState::Transcribing => "⏳",
            VoxtypeState::Stopped => "⬛",
        }
    }

    fn status_text(&self) -> &'static str {
        match self {
            VoxtypeState::Idle => "Status: Ready",
            VoxtypeState::Recording => "Status: Recording...",
            VoxtypeState::Transcribing => "Status: Transcribing...",
            VoxtypeState::Stopped => "Status: Daemon not running",
        }
    }
}

/// Menu item IDs
mod menu_ids {
    // Recording controls
    pub const TOGGLE: &str = "toggle";
    pub const CANCEL: &str = "cancel";

    // Engine selection
    pub const ENGINE_PARAKEET: &str = "engine_parakeet";
    pub const ENGINE_WHISPER: &str = "engine_whisper";

    // Hotkey mode
    pub const HOTKEY_PTT: &str = "hotkey_ptt";
    pub const HOTKEY_TOGGLE: &str = "hotkey_toggle";

    // Output mode
    pub const OUTPUT_TYPE: &str = "output_type";
    pub const OUTPUT_CLIPBOARD: &str = "output_clipboard";
    pub const OUTPUT_PASTE: &str = "output_paste";

    // Model prefixes (actual ID will be model_<name>)
    pub const MODEL_PREFIX: &str = "model_";

    // Utilities
    pub const DOWNLOAD_MODEL: &str = "download_model";
    pub const OPEN_CONFIG: &str = "open_config";
    pub const VIEW_LOGS: &str = "view_logs";
    pub const RESTART_DAEMON: &str = "restart_daemon";

    // Auto-start
    pub const AUTOSTART_ENABLE: &str = "autostart_enable";
    pub const AUTOSTART_DISABLE: &str = "autostart_disable";

    // Quit
    pub const QUIT: &str = "quit";
}

/// Read state from file
fn read_state_from_file(path: &PathBuf) -> VoxtypeState {
    std::fs::read_to_string(path)
        .map(|s| VoxtypeState::from_str(&s))
        .unwrap_or(VoxtypeState::Stopped)
}

/// Get the voxtype binary path
fn get_voxtype_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("voxtype")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("voxtype"))
}

/// Execute voxtype command
fn voxtype_cmd(args: &[&str]) {
    let voxtype_path = get_voxtype_path();
    let _ = std::process::Command::new(voxtype_path).args(args).spawn();
}

/// Execute voxtype command and wait for completion
fn voxtype_cmd_wait(args: &[&str]) -> bool {
    let voxtype_path = get_voxtype_path();
    std::process::Command::new(voxtype_path)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Open a file or URL with the default application
fn open_path(path: &str) {
    let _ = std::process::Command::new("open").arg(path).spawn();
}

/// Check if LaunchAgent is installed
fn is_autostart_enabled() -> bool {
    let home = dirs::home_dir().unwrap_or_default();
    let plist = home.join("Library/LaunchAgents/io.voxtype.daemon.plist");
    plist.exists()
}

/// Get list of downloaded models (both Whisper and Parakeet)
fn get_downloaded_models() -> Vec<(String, bool)> {
    let mut models = Vec::new();
    let models_dir = Config::models_dir();

    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();

            // Check for Whisper models (ggml-*.bin files)
            if name.starts_with("ggml-") && name.ends_with(".bin") {
                // Extract model name from filename (e.g., "ggml-base.en.bin" -> "base.en")
                let model_name = name
                    .strip_prefix("ggml-")
                    .and_then(|s| s.strip_suffix(".bin"))
                    .unwrap_or(&name)
                    .to_string();
                models.push((model_name, false)); // false = Whisper
            }

            // Check for Parakeet models (directories with encoder-model.onnx)
            if path.is_dir() && name.contains("parakeet") {
                if path.join("encoder-model.onnx").exists() {
                    models.push((name, true)); // true = Parakeet
                }
            }
        }
    }

    models.sort();
    models
}

/// Update config file with new engine
fn set_engine(engine: TranscriptionEngine) -> bool {
    let config_path = match Config::default_path() {
        Some(p) => p,
        None => return false,
    };

    let content = std::fs::read_to_string(&config_path).unwrap_or_default();

    let engine_str = match engine {
        TranscriptionEngine::Parakeet => "parakeet",
        TranscriptionEngine::Whisper => "whisper",
        TranscriptionEngine::Moonshine => "moonshine",
        TranscriptionEngine::SenseVoice => "sensevoice",
        TranscriptionEngine::Paraformer => "paraformer",
        TranscriptionEngine::Dolphin => "dolphin",
        TranscriptionEngine::Omnilingual => "omnilingual",
    };

    // Check if engine line exists
    let new_content = if content.contains("engine =") {
        // Replace existing engine line
        let re = regex::Regex::new(r#"engine\s*=\s*"[^"]*""#).unwrap();
        re.replace(&content, format!(r#"engine = "{}""#, engine_str))
            .to_string()
    } else {
        // Add engine line at the beginning
        format!("engine = \"{}\"\n{}", engine_str, content)
    };

    std::fs::write(&config_path, new_content).is_ok()
}

/// Update config file with new output mode
fn set_output_mode(mode: OutputMode) -> bool {
    let config_path = match Config::default_path() {
        Some(p) => p,
        None => return false,
    };

    let content = std::fs::read_to_string(&config_path).unwrap_or_default();

    let mode_str = match mode {
        OutputMode::Type => "type",
        OutputMode::Clipboard => "clipboard",
        OutputMode::Paste => "paste",
        OutputMode::File => "file",
    };

    // Check if [output] section exists with mode
    let new_content = if content.contains("[output]") {
        // Check if mode line exists under [output]
        if let Some(output_start) = content.find("[output]") {
            let after_output = &content[output_start..];
            if after_output.contains("mode =") {
                // Replace existing mode line
                let re = regex::Regex::new(r#"(\[output\][^\[]*?)mode\s*=\s*"[^"]*""#).unwrap();
                re.replace(&content, format!(r#"$1mode = "{}""#, mode_str))
                    .to_string()
            } else {
                // Add mode line after [output]
                content.replace("[output]", &format!("[output]\nmode = \"{}\"", mode_str))
            }
        } else {
            content.clone()
        }
    } else {
        // Add [output] section
        format!("{}\n[output]\nmode = \"{}\"\n", content, mode_str)
    };

    std::fs::write(&config_path, new_content).is_ok()
}

/// Update config file with new hotkey mode
fn set_hotkey_mode(mode: ActivationMode) -> bool {
    let config_path = match Config::default_path() {
        Some(p) => p,
        None => return false,
    };

    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mode_str = match mode {
        ActivationMode::PushToTalk => "push_to_talk",
        ActivationMode::Toggle => "toggle",
    };

    // Check if [hotkey] section exists
    let new_content = if content.contains("[hotkey]") {
        if content.contains("mode =") {
            // Replace existing mode line
            let re = regex::Regex::new(r#"mode\s*=\s*"[^"]*""#).unwrap();
            re.replace(&content, format!(r#"mode = "{}""#, mode_str))
                .to_string()
        } else {
            // Add mode line after [hotkey]
            content.replace("[hotkey]", &format!("[hotkey]\nmode = \"{}\"", mode_str))
        }
    } else {
        // Add [hotkey] section
        format!("{}\n[hotkey]\nmode = \"{}\"\n", content, mode_str)
    };

    std::fs::write(&config_path, new_content).is_ok()
}

/// Update config to use a specific model
fn set_model(model_name: &str, is_parakeet: bool) -> bool {
    if is_parakeet {
        voxtype_cmd_wait(&["setup", "parakeet", "--set", model_name])
    } else {
        voxtype_cmd_wait(&["setup", "model", "--set", model_name])
    }
}

/// Restart the daemon
fn restart_daemon() {
    // Try launchctl first
    let _ = std::process::Command::new("launchctl")
        .args(["kickstart", "-k", "gui/$(id -u)/io.voxtype.daemon"])
        .status();

    // Fallback: kill and restart
    let _ = std::process::Command::new("pkill")
        .args(["-f", "voxtype daemon"])
        .status();

    std::thread::sleep(Duration::from_millis(500));

    voxtype_cmd(&["daemon"]);
}

/// Show notification
fn notify(title: &str, message: &str) {
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            &format!(
                "display notification \"{}\" with title \"{}\"",
                message, title
            ),
        ])
        .spawn();
}

/// Build the settings submenus
/// Returns (menu, status_item) so status can be updated later
fn build_menu(config: &Config) -> (Menu, MenuItem) {
    let menu = Menu::new();

    // Recording controls
    let toggle_item = MenuItem::with_id(menu_ids::TOGGLE, "Toggle Recording", true, None);
    let cancel_item = MenuItem::with_id(menu_ids::CANCEL, "Cancel Recording", true, None);

    menu.append(&toggle_item).unwrap();
    menu.append(&cancel_item).unwrap();
    menu.append(&PredefinedMenuItem::separator()).unwrap();

    // Engine submenu
    let engine_menu = Submenu::new("Engine", true);
    let is_parakeet = config.engine == TranscriptionEngine::Parakeet;

    #[cfg(feature = "parakeet")]
    {
        let parakeet_item = CheckMenuItem::with_id(
            menu_ids::ENGINE_PARAKEET,
            "🦜 Parakeet (Fast)",
            true,
            is_parakeet,
            None,
        );
        engine_menu.append(&parakeet_item).unwrap();
    }

    let whisper_item = CheckMenuItem::with_id(
        menu_ids::ENGINE_WHISPER,
        "🗣️ Whisper",
        true,
        !is_parakeet,
        None,
    );
    engine_menu.append(&whisper_item).unwrap();
    menu.append(&engine_menu).unwrap();

    // Model submenu
    let model_menu = Submenu::new("Model", true);
    let downloaded_models = get_downloaded_models();
    let current_model = if is_parakeet {
        config
            .parakeet
            .as_ref()
            .map(|p| p.model.clone())
            .unwrap_or_default()
    } else {
        config.whisper.model.clone()
    };

    if downloaded_models.is_empty() {
        let no_models = MenuItem::new("No models downloaded", false, None);
        model_menu.append(&no_models).unwrap();
    } else {
        for (model_name, model_is_parakeet) in &downloaded_models {
            // Show models for the current engine
            if *model_is_parakeet == is_parakeet {
                let is_current = model_name == &current_model;
                let display_name = if *model_is_parakeet {
                    format!("🦜 {}", model_name)
                } else {
                    model_name.clone()
                };
                let item = CheckMenuItem::with_id(
                    format!("{}{}", menu_ids::MODEL_PREFIX, model_name),
                    display_name,
                    true,
                    is_current,
                    None,
                );
                model_menu.append(&item).unwrap();
            }
        }
    }

    model_menu.append(&PredefinedMenuItem::separator()).unwrap();
    let download_item =
        MenuItem::with_id(menu_ids::DOWNLOAD_MODEL, "Download Model...", true, None);
    model_menu.append(&download_item).unwrap();
    menu.append(&model_menu).unwrap();

    // Output mode submenu
    let output_menu = Submenu::new("Output Mode", true);
    let output_type = CheckMenuItem::with_id(
        menu_ids::OUTPUT_TYPE,
        "Type Text",
        true,
        config.output.mode == OutputMode::Type,
        None,
    );
    let output_clipboard = CheckMenuItem::with_id(
        menu_ids::OUTPUT_CLIPBOARD,
        "Copy to Clipboard",
        true,
        config.output.mode == OutputMode::Clipboard,
        None,
    );
    let output_paste = CheckMenuItem::with_id(
        menu_ids::OUTPUT_PASTE,
        "Clipboard + Paste",
        true,
        config.output.mode == OutputMode::Paste,
        None,
    );
    output_menu.append(&output_type).unwrap();
    output_menu.append(&output_clipboard).unwrap();
    output_menu.append(&output_paste).unwrap();
    menu.append(&output_menu).unwrap();

    // Hotkey mode submenu
    let hotkey_menu = Submenu::new("Hotkey Mode", true);
    let is_toggle = config.hotkey.mode == ActivationMode::Toggle;
    let ptt_item = CheckMenuItem::with_id(
        menu_ids::HOTKEY_PTT,
        "Push-to-Talk (hold)",
        true,
        !is_toggle,
        None,
    );
    let toggle_item = CheckMenuItem::with_id(
        menu_ids::HOTKEY_TOGGLE,
        "Toggle (press to start/stop)",
        true,
        is_toggle,
        None,
    );
    hotkey_menu.append(&ptt_item).unwrap();
    hotkey_menu.append(&toggle_item).unwrap();
    menu.append(&hotkey_menu).unwrap();

    // Auto-start submenu
    let autostart_menu = Submenu::new("Auto-start", true);
    let autostart_enabled = is_autostart_enabled();
    let enable_item = CheckMenuItem::with_id(
        menu_ids::AUTOSTART_ENABLE,
        "Start at Login",
        true,
        autostart_enabled,
        None,
    );
    autostart_menu.append(&enable_item).unwrap();
    menu.append(&autostart_menu).unwrap();

    menu.append(&PredefinedMenuItem::separator()).unwrap();

    // Status (disabled, just for display)
    let status_item = MenuItem::new("Status: Checking...", false, None);
    menu.append(&status_item).unwrap();

    menu.append(&PredefinedMenuItem::separator()).unwrap();

    // Utilities
    let config_item = MenuItem::with_id(menu_ids::OPEN_CONFIG, "Edit Config File...", true, None);
    let logs_item = MenuItem::with_id(menu_ids::VIEW_LOGS, "View Logs", true, None);
    let restart_item = MenuItem::with_id(menu_ids::RESTART_DAEMON, "Restart Daemon", true, None);

    menu.append(&config_item).unwrap();
    menu.append(&logs_item).unwrap();
    menu.append(&restart_item).unwrap();

    menu.append(&PredefinedMenuItem::separator()).unwrap();

    // Quit
    let quit_item = MenuItem::with_id(menu_ids::QUIT, "Quit Menu Bar", true, None);
    menu.append(&quit_item).unwrap();

    (menu, status_item)
}

/// Run the menu bar application
/// This should be called from the main thread
/// Note: This function never returns (runs the macOS event loop)
pub fn run(state_file: PathBuf) -> ! {
    println!("Starting Voxtype menu bar...");
    println!("State file: {}", state_file.display());

    // Single instance check
    let lock_path = Config::runtime_dir().join("menubar.lock");
    let lock_path_str = lock_path.to_string_lossy().to_string();
    let mut pidlock = Pidlock::new(&lock_path_str);

    match pidlock.acquire() {
        Ok(_) => {
            println!("Acquired menu bar lock");
        }
        Err(_) => {
            eprintln!("Error: Another voxtype menubar instance is already running.");
            std::process::exit(1);
        }
    }

    // Check if state file exists (daemon should be running)
    if !state_file.exists() {
        println!("\nWarning: State file not found. Is the voxtype daemon running?");
        println!("Start it with: voxtype daemon\n");
    }

    // Load config
    let config = crate::config::load_config(None).unwrap_or_default();

    // Build menu (returns menu and status item for updates)
    let (menu, status_item) = build_menu(&config);

    // Get initial state
    let initial_state = read_state_from_file(&state_file);

    // Update status item with initial state
    let _ = status_item.set_text(initial_state.status_text());

    // Create tray icon
    let tray = TrayIconBuilder::new()
        .with_tooltip("Voxtype")
        .with_title(initial_state.icon())
        .with_menu(Box::new(menu))
        .build()
        .expect("Failed to create tray icon");

    println!("Menu bar is running. Look for the icon in your menu bar.");
    println!("Press Ctrl+C to stop.\n");

    // Track state
    let mut last_state = initial_state;
    let mut last_update = Instant::now();
    let update_interval = Duration::from_millis(500);
    let running = Arc::new(AtomicBool::new(true));

    // Set up menu event receiver
    let menu_channel = MenuEvent::receiver();

    // Create event loop
    let event_loop = EventLoopBuilder::new().build();

    event_loop.run(move |_event, _, control_flow| {
        // Set to poll mode so we can check state periodically
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));

        // Check for menu events (non-blocking)
        if let Ok(event) = menu_channel.try_recv() {
            let id = event.id().0.as_str();

            match id {
                // Recording controls
                menu_ids::TOGGLE => {
                    voxtype_cmd(&["record", "toggle"]);
                }
                menu_ids::CANCEL => {
                    voxtype_cmd(&["record", "cancel"]);
                }

                // Engine selection
                menu_ids::ENGINE_PARAKEET => {
                    if set_engine(TranscriptionEngine::Parakeet) {
                        notify(
                            "Voxtype",
                            "Switched to Parakeet engine. Restart daemon to apply.",
                        );
                    }
                }
                menu_ids::ENGINE_WHISPER => {
                    if set_engine(TranscriptionEngine::Whisper) {
                        notify(
                            "Voxtype",
                            "Switched to Whisper engine. Restart daemon to apply.",
                        );
                    }
                }

                // Hotkey mode
                menu_ids::HOTKEY_PTT => {
                    if set_hotkey_mode(ActivationMode::PushToTalk) {
                        notify(
                            "Voxtype",
                            "Switched to push-to-talk mode. Restart daemon to apply.",
                        );
                    }
                }
                menu_ids::HOTKEY_TOGGLE => {
                    if set_hotkey_mode(ActivationMode::Toggle) {
                        notify(
                            "Voxtype",
                            "Switched to toggle mode. Restart daemon to apply.",
                        );
                    }
                }

                // Output mode
                menu_ids::OUTPUT_TYPE => {
                    if set_output_mode(OutputMode::Type) {
                        notify("Voxtype", "Output mode: Type text");
                    }
                }
                menu_ids::OUTPUT_CLIPBOARD => {
                    if set_output_mode(OutputMode::Clipboard) {
                        notify("Voxtype", "Output mode: Copy to clipboard");
                    }
                }
                menu_ids::OUTPUT_PASTE => {
                    if set_output_mode(OutputMode::Paste) {
                        notify("Voxtype", "Output mode: Clipboard + Paste");
                    }
                }

                // Auto-start
                menu_ids::AUTOSTART_ENABLE => {
                    if is_autostart_enabled() {
                        // Disable
                        if voxtype_cmd_wait(&["setup", "launchd", "--uninstall"]) {
                            notify("Voxtype", "Auto-start disabled");
                        }
                    } else {
                        // Enable
                        if voxtype_cmd_wait(&["setup", "launchd"]) {
                            notify("Voxtype", "Auto-start enabled");
                        }
                    }
                }

                // Utilities
                menu_ids::DOWNLOAD_MODEL => {
                    // Open terminal with model download command
                    let voxtype_path = get_voxtype_path();
                    let script = format!(
                        "tell application \"Terminal\" to do script \"{}\" & \" setup model\"",
                        voxtype_path.display()
                    );
                    let _ = std::process::Command::new("osascript")
                        .args(["-e", &script])
                        .spawn();
                }
                menu_ids::OPEN_CONFIG => {
                    if let Some(config_path) = Config::default_path() {
                        open_path(config_path.to_str().unwrap_or(""));
                    }
                }
                menu_ids::VIEW_LOGS => {
                    let home = dirs::home_dir().unwrap_or_default();
                    let log_path = home.join("Library/Logs/voxtype");
                    open_path(log_path.to_str().unwrap_or(""));
                }
                menu_ids::RESTART_DAEMON => {
                    notify("Voxtype", "Restarting daemon...");
                    restart_daemon();
                }

                // Quit
                menu_ids::QUIT => {
                    running.store(false, Ordering::SeqCst);
                    *control_flow = ControlFlow::Exit;
                }

                // Model selection (dynamic IDs)
                _ if id.starts_with(menu_ids::MODEL_PREFIX) => {
                    let model_name = id.strip_prefix(menu_ids::MODEL_PREFIX).unwrap_or("");
                    let is_parakeet = model_name.contains("parakeet");
                    if set_model(model_name, is_parakeet) {
                        notify("Voxtype", &format!("Switched to model: {}", model_name));
                    }
                }

                _ => {}
            }
        }

        // Update state periodically
        if last_update.elapsed() >= update_interval {
            let new_state = read_state_from_file(&state_file);

            if new_state != last_state {
                // Update icon and status text
                let _ = tray.set_title(Some(new_state.icon()));
                let _ = status_item.set_text(new_state.status_text());
                last_state = new_state;
            }

            last_update = Instant::now();
        }

        if !running.load(Ordering::SeqCst) {
            *control_flow = ControlFlow::Exit;
        }
    });
}
