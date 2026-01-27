//! macOS menu bar integration
//!
//! Provides a system tray icon showing voxtype status with a context menu
//! for controlling recording.

use crate::config::Config;
use pidlock::Pidlock;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
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
const MENU_TOGGLE: &str = "toggle";
const MENU_CANCEL: &str = "cancel";
const MENU_QUIT: &str = "quit";

/// Read state from file
fn read_state_from_file(path: &PathBuf) -> VoxtypeState {
    std::fs::read_to_string(path)
        .map(|s| VoxtypeState::from_str(&s))
        .unwrap_or(VoxtypeState::Stopped)
}

/// Execute voxtype command
fn voxtype_cmd(cmd: &str) {
    // Use the full path if available, otherwise hope it's in PATH
    let voxtype_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("voxtype")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("voxtype"));

    let _ = std::process::Command::new(voxtype_path)
        .args(["record", cmd])
        .spawn();
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

    // Create menu items
    let toggle_item = MenuItem::with_id(MENU_TOGGLE, "Toggle Recording", true, None);
    let cancel_item = MenuItem::with_id(MENU_CANCEL, "Cancel Recording", true, None);
    let status_item = MenuItem::new("Status: Checking...", false, None);
    let quit_item = MenuItem::with_id(MENU_QUIT, "Quit Menu Bar", true, None);

    // Create menu
    let menu = Menu::new();
    menu.append(&toggle_item).expect("Failed to append toggle item");
    menu.append(&cancel_item).expect("Failed to append cancel item");
    menu.append(&PredefinedMenuItem::separator()).expect("Failed to append separator");
    menu.append(&status_item).expect("Failed to append status item");
    menu.append(&PredefinedMenuItem::separator()).expect("Failed to append separator");
    menu.append(&quit_item).expect("Failed to append quit item");

    // Get initial state
    let initial_state = read_state_from_file(&state_file);

    // Create tray icon
    let tray = TrayIconBuilder::new()
        .with_tooltip("Voxtype")
        .with_title(initial_state.icon())
        .with_menu(Box::new(menu))
        .build()
        .expect("Failed to create tray icon");

    // Update initial status
    let _ = status_item.set_text(initial_state.status_text());

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
            match event.id().0.as_str() {
                MENU_TOGGLE => {
                    voxtype_cmd("toggle");
                }
                MENU_CANCEL => {
                    voxtype_cmd("cancel");
                }
                MENU_QUIT => {
                    running.store(false, Ordering::SeqCst);
                    *control_flow = ControlFlow::Exit;
                }
                _ => {}
            }
        }

        // Update state periodically
        if last_update.elapsed() >= update_interval {
            let new_state = read_state_from_file(&state_file);

            if new_state != last_state {
                // Update icon
                let _ = tray.set_title(Some(new_state.icon()));

                // Update status text
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
