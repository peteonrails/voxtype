//! macOS-based hotkey listener using CGEventTap
//!
//! Uses the macOS Quartz Event Services (CGEventTap) to capture global key events.
//! For the FN/Globe key, monitors the SecondaryFn modifier flag changes.
//!
//! This approach requires Accessibility permissions to be granted to the application.
//! The user must grant Accessibility access in System Preferences > Security & Privacy >
//! Privacy > Accessibility for voxtype to receive global key events.

use super::{HotkeyEvent, HotkeyListener};
use crate::config::HotkeyConfig;
use crate::error::HotkeyError;
use core_foundation::runloop::{kCFRunLoopCommonModes, kCFRunLoopDefaultMode, CFRunLoop};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, EventField,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// macOS virtual key codes
/// These are defined in Carbon HIToolbox Events.h (kVK_* constants)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum VirtualKeyCode {
    // Letter keys (kVK_ANSI_*)
    KEY_A = 0x00,
    KEY_S = 0x01,
    KEY_D = 0x02,
    KEY_F = 0x03,
    KEY_H = 0x04,
    KEY_G = 0x05,
    KEY_Z = 0x06,
    KEY_X = 0x07,
    KEY_C = 0x08,
    KEY_V = 0x09,
    KEY_B = 0x0B,
    KEY_Q = 0x0C,
    KEY_W = 0x0D,
    KEY_E = 0x0E,
    KEY_R = 0x0F,
    KEY_Y = 0x10,
    KEY_T = 0x11,
    KEY_O = 0x1F,
    KEY_U = 0x20,
    KEY_I = 0x22,
    KEY_P = 0x23,
    KEY_L = 0x25,
    KEY_J = 0x26,
    KEY_K = 0x28,
    KEY_N = 0x2D,
    KEY_M = 0x2E,

    // Number keys (kVK_ANSI_*)
    KEY_1 = 0x12,
    KEY_2 = 0x13,
    KEY_3 = 0x14,
    KEY_4 = 0x15,
    KEY_5 = 0x17,
    KEY_6 = 0x16,
    KEY_7 = 0x1A,
    KEY_8 = 0x1C,
    KEY_9 = 0x19,
    KEY_0 = 0x1D,

    // Function keys (kVK_F*)
    KEY_F1 = 0x7A,
    KEY_F2 = 0x78,
    KEY_F3 = 0x63,
    KEY_F4 = 0x76,
    KEY_F5 = 0x60,
    KEY_F6 = 0x61,
    KEY_F7 = 0x62,
    KEY_F8 = 0x64,
    KEY_F9 = 0x65,
    KEY_F10 = 0x6D,
    KEY_F11 = 0x67,
    KEY_F12 = 0x6F,
    KEY_F13 = 0x69,
    KEY_F14 = 0x6B,
    KEY_F15 = 0x71,
    KEY_F16 = 0x6A,
    KEY_F17 = 0x40,
    KEY_F18 = 0x4F,
    KEY_F19 = 0x50,
    KEY_F20 = 0x5A,

    // Modifier keys (kVK_*)
    KEY_CAPSLOCK = 0x39,
    KEY_SHIFT = 0x38,
    KEY_RIGHTSHIFT = 0x3C,
    KEY_CONTROL = 0x3B,
    KEY_RIGHTCONTROL = 0x3E,
    KEY_OPTION = 0x3A,      // Left Alt/Option
    KEY_RIGHTOPTION = 0x3D, // Right Alt/Option
    KEY_COMMAND = 0x37,     // Left Command
    KEY_RIGHTCOMMAND = 0x36,
    KEY_FN = 0x3F,

    // Special keys
    KEY_RETURN = 0x24,
    KEY_TAB = 0x30,
    KEY_SPACE = 0x31,
    KEY_DELETE = 0x33, // Backspace
    KEY_ESCAPE = 0x35,
    KEY_FORWARDDELETE = 0x75,
    KEY_HOME = 0x73,
    KEY_END = 0x77,
    KEY_PAGEUP = 0x74,
    KEY_PAGEDOWN = 0x79,

    // Arrow keys
    KEY_LEFTARROW = 0x7B,
    KEY_RIGHTARROW = 0x7C,
    KEY_DOWNARROW = 0x7D,
    KEY_UPARROW = 0x7E,

    // Misc (kVK_ANSI_*)
    KEY_GRAVE = 0x32, // ` or ~
    KEY_MINUS = 0x1B,
    KEY_EQUAL = 0x18,
    KEY_LEFTBRACKET = 0x21,
    KEY_RIGHTBRACKET = 0x1E,
    KEY_BACKSLASH = 0x2A,
    KEY_SEMICOLON = 0x29,
    KEY_QUOTE = 0x27,
    KEY_COMMA = 0x2B,
    KEY_PERIOD = 0x2F,
    KEY_SLASH = 0x2C,

    // Media keys (on keyboards with them)
    KEY_MUTE = 0x4A,
    KEY_VOLUMEDOWN = 0x49,
    KEY_VOLUMEUP = 0x48,

    // Help/Insert key (not present on most Mac keyboards but exists in code)
    KEY_HELP = 0x72,
}

impl VirtualKeyCode {
    /// Convert a raw key code to a VirtualKeyCode enum variant
    /// This is useful for debugging key events
    #[allow(dead_code)]
    fn from_u16(code: u16) -> Option<Self> {
        // Match against known values
        match code {
            0x00 => Some(Self::KEY_A),
            0x01 => Some(Self::KEY_S),
            0x02 => Some(Self::KEY_D),
            0x03 => Some(Self::KEY_F),
            0x04 => Some(Self::KEY_H),
            0x05 => Some(Self::KEY_G),
            0x06 => Some(Self::KEY_Z),
            0x07 => Some(Self::KEY_X),
            0x08 => Some(Self::KEY_C),
            0x09 => Some(Self::KEY_V),
            0x0B => Some(Self::KEY_B),
            0x0C => Some(Self::KEY_Q),
            0x0D => Some(Self::KEY_W),
            0x0E => Some(Self::KEY_E),
            0x0F => Some(Self::KEY_R),
            0x10 => Some(Self::KEY_Y),
            0x11 => Some(Self::KEY_T),
            0x1F => Some(Self::KEY_O),
            0x20 => Some(Self::KEY_U),
            0x22 => Some(Self::KEY_I),
            0x23 => Some(Self::KEY_P),
            0x25 => Some(Self::KEY_L),
            0x26 => Some(Self::KEY_J),
            0x28 => Some(Self::KEY_K),
            0x2D => Some(Self::KEY_N),
            0x2E => Some(Self::KEY_M),
            0x12 => Some(Self::KEY_1),
            0x13 => Some(Self::KEY_2),
            0x14 => Some(Self::KEY_3),
            0x15 => Some(Self::KEY_4),
            0x17 => Some(Self::KEY_5),
            0x16 => Some(Self::KEY_6),
            0x19 => Some(Self::KEY_9),
            0x1A => Some(Self::KEY_7),
            0x1C => Some(Self::KEY_8),
            0x1D => Some(Self::KEY_0),
            0x7A => Some(Self::KEY_F1),
            0x78 => Some(Self::KEY_F2),
            0x63 => Some(Self::KEY_F3),
            0x76 => Some(Self::KEY_F4),
            0x60 => Some(Self::KEY_F5),
            0x61 => Some(Self::KEY_F6),
            0x62 => Some(Self::KEY_F7),
            0x64 => Some(Self::KEY_F8),
            0x65 => Some(Self::KEY_F9),
            0x6D => Some(Self::KEY_F10),
            0x67 => Some(Self::KEY_F11),
            0x6F => Some(Self::KEY_F12),
            0x69 => Some(Self::KEY_F13),
            0x6B => Some(Self::KEY_F14),
            0x71 => Some(Self::KEY_F15),
            0x6A => Some(Self::KEY_F16),
            0x40 => Some(Self::KEY_F17),
            0x4F => Some(Self::KEY_F18),
            0x50 => Some(Self::KEY_F19),
            0x5A => Some(Self::KEY_F20),
            0x39 => Some(Self::KEY_CAPSLOCK),
            0x38 => Some(Self::KEY_SHIFT),
            0x3C => Some(Self::KEY_RIGHTSHIFT),
            0x3B => Some(Self::KEY_CONTROL),
            0x3E => Some(Self::KEY_RIGHTCONTROL),
            0x3A => Some(Self::KEY_OPTION),
            0x3D => Some(Self::KEY_RIGHTOPTION),
            0x37 => Some(Self::KEY_COMMAND),
            0x36 => Some(Self::KEY_RIGHTCOMMAND),
            0x3F => Some(Self::KEY_FN),
            0x24 => Some(Self::KEY_RETURN),
            0x30 => Some(Self::KEY_TAB),
            0x31 => Some(Self::KEY_SPACE),
            0x33 => Some(Self::KEY_DELETE),
            0x35 => Some(Self::KEY_ESCAPE),
            0x75 => Some(Self::KEY_FORWARDDELETE),
            0x73 => Some(Self::KEY_HOME),
            0x77 => Some(Self::KEY_END),
            0x74 => Some(Self::KEY_PAGEUP),
            0x79 => Some(Self::KEY_PAGEDOWN),
            0x7B => Some(Self::KEY_LEFTARROW),
            0x7C => Some(Self::KEY_RIGHTARROW),
            0x7D => Some(Self::KEY_DOWNARROW),
            0x7E => Some(Self::KEY_UPARROW),
            0x32 => Some(Self::KEY_GRAVE),
            0x1B => Some(Self::KEY_MINUS),
            0x18 => Some(Self::KEY_EQUAL),
            0x21 => Some(Self::KEY_LEFTBRACKET),
            0x1E => Some(Self::KEY_RIGHTBRACKET),
            0x2A => Some(Self::KEY_BACKSLASH),
            0x29 => Some(Self::KEY_SEMICOLON),
            0x27 => Some(Self::KEY_QUOTE),
            0x2B => Some(Self::KEY_COMMA),
            0x2F => Some(Self::KEY_PERIOD),
            0x2C => Some(Self::KEY_SLASH),
            0x4A => Some(Self::KEY_MUTE),
            0x49 => Some(Self::KEY_VOLUMEDOWN),
            0x48 => Some(Self::KEY_VOLUMEUP),
            0x72 => Some(Self::KEY_HELP),
            _ => None,
        }
    }
}

/// macOS-based hotkey listener using CGEventTap
pub struct MacOSListener {
    /// The key to listen for
    target_key: VirtualKeyCode,
    /// Required modifier flags
    modifier_flags: CGEventFlags,
    /// Optional cancel key
    cancel_key: Option<VirtualKeyCode>,
    /// Signal to stop the listener task
    stop_signal: Option<oneshot::Sender<()>>,
    /// Flag to signal stop from callback
    stop_flag: Arc<AtomicBool>,
}

impl MacOSListener {
    /// Create a new macOS listener for the configured hotkey
    pub fn new(config: &HotkeyConfig) -> Result<Self, HotkeyError> {
        let target_key = parse_key_name(&config.key)?;

        let modifier_flags = config
            .modifiers
            .iter()
            .map(|m| parse_modifier_name(m))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .fold(CGEventFlags::empty(), |acc, flag| acc | flag);

        let cancel_key = config
            .cancel_key
            .as_ref()
            .map(|k| parse_key_name(k))
            .transpose()?;

        // Check for Accessibility permissions
        if !check_accessibility_permissions() {
            return Err(HotkeyError::DeviceAccess(
                "Accessibility permissions required. Please grant access in:\n  \
                System Settings > Privacy & Security > Accessibility\n\n  \
                Add your terminal application (e.g., Terminal.app, iTerm2, or your IDE) \
                to the list of allowed apps.\n\n  \
                After granting access, restart voxtype."
                    .to_string(),
            ));
        }

        Ok(Self {
            target_key,
            modifier_flags,
            cancel_key,
            stop_signal: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
        })
    }
}

/// Check if the application has Accessibility permissions
fn check_accessibility_permissions() -> bool {
    // Use the AXIsProcessTrusted function from ApplicationServices framework
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    unsafe { AXIsProcessTrusted() }
}

#[async_trait::async_trait]
impl HotkeyListener for MacOSListener {
    async fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>, HotkeyError> {
        let (tx, rx) = mpsc::channel(32);
        let (stop_tx, stop_rx) = oneshot::channel();
        self.stop_signal = Some(stop_tx);
        self.stop_flag.store(false, Ordering::SeqCst);

        let target_key = self.target_key;
        let modifier_flags = self.modifier_flags;
        let cancel_key = self.cancel_key;
        let stop_flag = self.stop_flag.clone();

        // Spawn the listener in a blocking task since CFRunLoop blocks
        tokio::task::spawn_blocking(move || {
            if let Err(e) = macos_listener_loop(
                target_key,
                modifier_flags,
                cancel_key,
                tx,
                stop_rx,
                stop_flag,
            ) {
                tracing::error!("macOS hotkey listener error: {}", e);
            }
        });

        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), HotkeyError> {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(stop) = self.stop_signal.take() {
            let _ = stop.send(());
        }
        // Give the run loop a moment to stop
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        tracing::debug!("macOS hotkey listener stopping");
        Ok(())
    }
}

/// Main listener loop running in a blocking task
fn macos_listener_loop(
    target_key: VirtualKeyCode,
    modifier_flags: CGEventFlags,
    cancel_key: Option<VirtualKeyCode>,
    tx: mpsc::Sender<HotkeyEvent>,
    _stop_rx: oneshot::Receiver<()>,
    stop_flag: Arc<AtomicBool>,
) -> Result<(), HotkeyError> {
    // Track if we're currently "pressed" (to handle repeat events)
    let is_pressed = Arc::new(AtomicBool::new(false));

    // Create a channel for events from the callback
    let (event_tx, event_rx) = std_mpsc::channel::<HotkeyEvent>();

    // Clone values for the callback closure
    let is_pressed_clone = is_pressed.clone();
    let stop_flag_clone = stop_flag.clone();

    // Create the event tap callback
    let callback = move |_proxy: core_graphics::event::CGEventTapProxy,
                         event_type: CGEventType,
                         event: &CGEvent|
          -> Option<CGEvent> {
        // Check stop flag
        if stop_flag_clone.load(Ordering::SeqCst) {
            CFRunLoop::get_current().stop();
            return Some(event.clone());
        }

        let key_code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;

        // Get current modifier flags from the event
        let current_flags = event.get_flags();

        match event_type {
            CGEventType::KeyDown => {
                // Check cancel key first
                if let Some(cancel) = cancel_key {
                    if key_code == cancel as u16 {
                        let _ = event_tx.send(HotkeyEvent::Cancel);
                        return Some(event.clone());
                    }
                }

                // Check target key with modifiers
                if key_code == target_key as u16 {
                    let modifiers_match = check_modifiers(current_flags, modifier_flags);

                    if modifiers_match && !is_pressed_clone.load(Ordering::SeqCst) {
                        is_pressed_clone.store(true, Ordering::SeqCst);
                        tracing::debug!("Hotkey pressed (macOS)");
                        let _ = event_tx.send(HotkeyEvent::Pressed { model_override: None });
                    }
                }
            }
            CGEventType::KeyUp => {
                if key_code == target_key as u16 && is_pressed_clone.load(Ordering::SeqCst) {
                    is_pressed_clone.store(false, Ordering::SeqCst);
                    tracing::debug!("Hotkey released (macOS)");
                    let _ = event_tx.send(HotkeyEvent::Released);
                }
            }
            CGEventType::FlagsChanged => {
                // Special handling for FN key - detect via flag, not key code
                if target_key == VirtualKeyCode::KEY_FN {
                    let fn_pressed = current_flags.contains(CGEventFlags::CGEventFlagSecondaryFn);
                    let was_pressed = is_pressed_clone.load(Ordering::SeqCst);
                    tracing::debug!("FN flag check: fn_pressed={}, was_pressed={}", fn_pressed, was_pressed);
                    if fn_pressed && !was_pressed {
                        is_pressed_clone.store(true, Ordering::SeqCst);
                        tracing::debug!("FN key pressed (macOS)");
                        let _ = event_tx.send(HotkeyEvent::Pressed { model_override: None });
                    } else if !fn_pressed && is_pressed_clone.load(Ordering::SeqCst) {
                        is_pressed_clone.store(false, Ordering::SeqCst);
                        tracing::debug!("FN key released (macOS)");
                        let _ = event_tx.send(HotkeyEvent::Released);
                    }
                } else if key_code == target_key as u16 {
                    // Handle other modifier key changes
                    let is_modifier_pressed = check_modifier_pressed(key_code, current_flags);

                    if is_modifier_pressed && !is_pressed_clone.load(Ordering::SeqCst) {
                        is_pressed_clone.store(true, Ordering::SeqCst);
                        tracing::debug!("Modifier hotkey pressed (macOS)");
                        let _ = event_tx.send(HotkeyEvent::Pressed { model_override: None });
                    } else if !is_modifier_pressed && is_pressed_clone.load(Ordering::SeqCst) {
                        is_pressed_clone.store(false, Ordering::SeqCst);
                        tracing::debug!("Modifier hotkey released (macOS)");
                        let _ = event_tx.send(HotkeyEvent::Released);
                    }
                }
            }
            _ => {}
        }

        // Return the event unchanged (don't consume it)
        Some(event.clone())
    };

    // Create the event tap
    let event_tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::ListenOnly,
        vec![
            CGEventType::KeyDown,
            CGEventType::KeyUp,
            CGEventType::FlagsChanged,
        ],
        callback,
    )
    .map_err(|_| {
        HotkeyError::DeviceAccess(
            "Failed to create event tap. Ensure Accessibility permissions are granted.".to_string(),
        )
    })?;

    // Enable the event tap
    event_tap.enable();

    // Create a run loop source from the event tap
    let run_loop_source = event_tap
        .mach_port
        .create_runloop_source(0)
        .map_err(|_| HotkeyError::DeviceAccess("Failed to create run loop source".to_string()))?;

    // Get the current run loop and add the source
    let run_loop = CFRunLoop::get_current();
    run_loop.add_source(&run_loop_source, unsafe { kCFRunLoopCommonModes });

    if let Some(cancel) = cancel_key {
        tracing::info!(
            "Listening for {:?} (with modifiers: {:?}) and cancel key {:?} on macOS",
            target_key,
            modifier_flags,
            cancel
        );
    } else {
        tracing::info!(
            "Listening for {:?} (with modifiers: {:?}) on macOS",
            target_key,
            modifier_flags
        );
    }

    // Spawn a thread to forward events from std channel to tokio channel
    // and check stop flag periodically
    let tx_clone = tx.clone();
    let stop_flag_thread = stop_flag.clone();
    std::thread::spawn(move || {
        loop {
            if stop_flag_thread.load(Ordering::SeqCst) {
                break;
            }

            match event_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(event) => {
                    if tx_clone.blocking_send(event).is_err() {
                        break;
                    }
                }
                Err(std_mpsc::RecvTimeoutError::Timeout) => {
                    // Continue checking stop flag
                }
                Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }
    });

    // Run the event loop (blocks until stopped)
    // Run with a timeout to periodically check stop flag
    while !stop_flag.load(Ordering::SeqCst) {
        CFRunLoop::run_in_mode(
            unsafe { kCFRunLoopDefaultMode },
            std::time::Duration::from_millis(100),
            true,
        );
    }

    tracing::debug!("macOS hotkey listener stopping");
    Ok(())
}

/// Check if required modifier flags are satisfied
fn check_modifiers(current: CGEventFlags, required: CGEventFlags) -> bool {
    if required.is_empty() {
        return true;
    }
    current.contains(required)
}

/// Check if a modifier key is currently pressed based on flags
fn check_modifier_pressed(key_code: u16, flags: CGEventFlags) -> bool {
    match key_code {
        0x38 | 0x3C => flags.contains(CGEventFlags::CGEventFlagShift),
        0x3B | 0x3E => flags.contains(CGEventFlags::CGEventFlagControl),
        0x3A | 0x3D => flags.contains(CGEventFlags::CGEventFlagAlternate),
        0x37 | 0x36 => flags.contains(CGEventFlags::CGEventFlagCommand),
        0x39 => flags.contains(CGEventFlags::CGEventFlagAlphaShift),
        0x3F => flags.contains(CGEventFlags::CGEventFlagSecondaryFn),
        _ => false,
    }
}

/// Parse a key name string to macOS virtual key code
fn parse_key_name(name: &str) -> Result<VirtualKeyCode, HotkeyError> {
    // Normalize: uppercase and replace - or space with _
    let normalized: String = name
        .chars()
        .map(|c| match c {
            '-' | ' ' => '_',
            c => c.to_ascii_uppercase(),
        })
        .collect();

    // Remove KEY_ prefix if present
    let key_name = normalized.strip_prefix("KEY_").unwrap_or(&normalized);

    // Map common key names to macOS virtual key codes
    let key = match key_name {
        // Lock keys (good hotkey candidates)
        "SCROLLLOCK" => {
            return Err(HotkeyError::UnknownKey(
                "SCROLLLOCK is not available on macOS. Try F13-F20, FN, or a modifier key like RIGHTOPTION".to_string()
            ));
        }
        "PAUSE" => {
            return Err(HotkeyError::UnknownKey(
                "PAUSE is not available on macOS. Try F13-F20, FN, or a modifier key like RIGHTOPTION".to_string()
            ));
        }
        "CAPSLOCK" => VirtualKeyCode::KEY_CAPSLOCK,
        "NUMLOCK" => {
            return Err(HotkeyError::UnknownKey(
                "NUMLOCK is not available on macOS. Try F13-F20, FN, or CAPSLOCK".to_string(),
            ));
        }
        "INSERT" | "HELP" => VirtualKeyCode::KEY_HELP,

        // Modifier keys
        "LEFTALT" | "LALT" | "OPTION" | "LEFTOPTION" => VirtualKeyCode::KEY_OPTION,
        "RIGHTALT" | "RALT" | "RIGHTOPTION" | "ALTGR" => VirtualKeyCode::KEY_RIGHTOPTION,
        "LEFTCTRL" | "LCTRL" | "CONTROL" | "LEFTCONTROL" => VirtualKeyCode::KEY_CONTROL,
        "RIGHTCTRL" | "RCTRL" | "RIGHTCONTROL" => VirtualKeyCode::KEY_RIGHTCONTROL,
        "LEFTSHIFT" | "LSHIFT" | "SHIFT" => VirtualKeyCode::KEY_SHIFT,
        "RIGHTSHIFT" | "RSHIFT" => VirtualKeyCode::KEY_RIGHTSHIFT,
        "LEFTMETA" | "LMETA" | "SUPER" | "COMMAND" | "LEFTCOMMAND" | "CMD" => {
            VirtualKeyCode::KEY_COMMAND
        }
        "RIGHTMETA" | "RMETA" | "RIGHTCOMMAND" | "RCMD" => VirtualKeyCode::KEY_RIGHTCOMMAND,
        "FN" | "FUNCTION" | "GLOBE" => VirtualKeyCode::KEY_FN,

        // Function keys (F13-F20 are good hotkey choices on macOS)
        "F1" => VirtualKeyCode::KEY_F1,
        "F2" => VirtualKeyCode::KEY_F2,
        "F3" => VirtualKeyCode::KEY_F3,
        "F4" => VirtualKeyCode::KEY_F4,
        "F5" => VirtualKeyCode::KEY_F5,
        "F6" => VirtualKeyCode::KEY_F6,
        "F7" => VirtualKeyCode::KEY_F7,
        "F8" => VirtualKeyCode::KEY_F8,
        "F9" => VirtualKeyCode::KEY_F9,
        "F10" => VirtualKeyCode::KEY_F10,
        "F11" => VirtualKeyCode::KEY_F11,
        "F12" => VirtualKeyCode::KEY_F12,
        "F13" => VirtualKeyCode::KEY_F13,
        "F14" => VirtualKeyCode::KEY_F14,
        "F15" => VirtualKeyCode::KEY_F15,
        "F16" => VirtualKeyCode::KEY_F16,
        "F17" => VirtualKeyCode::KEY_F17,
        "F18" => VirtualKeyCode::KEY_F18,
        "F19" => VirtualKeyCode::KEY_F19,
        "F20" => VirtualKeyCode::KEY_F20,

        // Navigation keys
        "HOME" => VirtualKeyCode::KEY_HOME,
        "END" => VirtualKeyCode::KEY_END,
        "PAGEUP" => VirtualKeyCode::KEY_PAGEUP,
        "PAGEDOWN" => VirtualKeyCode::KEY_PAGEDOWN,
        "DELETE" | "FORWARDDELETE" => VirtualKeyCode::KEY_FORWARDDELETE,
        "BACKSPACE" => VirtualKeyCode::KEY_DELETE,

        // Common keys
        "SPACE" => VirtualKeyCode::KEY_SPACE,
        "ENTER" | "RETURN" => VirtualKeyCode::KEY_RETURN,
        "TAB" => VirtualKeyCode::KEY_TAB,
        "ESC" | "ESCAPE" => VirtualKeyCode::KEY_ESCAPE,
        "GRAVE" | "BACKTICK" => VirtualKeyCode::KEY_GRAVE,

        // Media keys
        "MUTE" => VirtualKeyCode::KEY_MUTE,
        "VOLUMEDOWN" => VirtualKeyCode::KEY_VOLUMEDOWN,
        "VOLUMEUP" => VirtualKeyCode::KEY_VOLUMEUP,

        // If not found, return error with macOS-specific suggestions
        _ => {
            return Err(HotkeyError::UnknownKey(format!(
                "{}. On macOS, try: F13-F20, FN, RIGHTOPTION, or CAPSLOCK. \
                 Note: SCROLLLOCK and PAUSE are not available on macOS keyboards.",
                name
            )));
        }
    };

    Ok(key)
}

/// Parse a modifier name to CGEventFlags
fn parse_modifier_name(name: &str) -> Result<CGEventFlags, HotkeyError> {
    let normalized: String = name
        .chars()
        .map(|c| match c {
            '-' | ' ' => '_',
            c => c.to_ascii_uppercase(),
        })
        .collect();

    let key_name = normalized.strip_prefix("KEY_").unwrap_or(&normalized);

    match key_name {
        "LEFTSHIFT" | "LSHIFT" | "RIGHTSHIFT" | "RSHIFT" | "SHIFT" => {
            Ok(CGEventFlags::CGEventFlagShift)
        }
        "LEFTCTRL" | "LCTRL" | "RIGHTCTRL" | "RCTRL" | "CONTROL" | "CTRL" => {
            Ok(CGEventFlags::CGEventFlagControl)
        }
        "LEFTALT" | "LALT" | "RIGHTALT" | "RALT" | "ALT" | "OPTION" | "LEFTOPTION"
        | "RIGHTOPTION" => Ok(CGEventFlags::CGEventFlagAlternate),
        "LEFTMETA" | "LMETA" | "RIGHTMETA" | "RMETA" | "SUPER" | "COMMAND" | "CMD" => {
            Ok(CGEventFlags::CGEventFlagCommand)
        }
        "FN" | "FUNCTION" => Ok(CGEventFlags::CGEventFlagSecondaryFn),
        _ => Err(HotkeyError::UnknownKey(format!(
            "Unknown modifier: {}. On macOS, use: SHIFT, CONTROL, OPTION (ALT), COMMAND, or FN",
            name
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_name() {
        assert_eq!(parse_key_name("F13").unwrap(), VirtualKeyCode::KEY_F13);
        assert_eq!(parse_key_name("f13").unwrap(), VirtualKeyCode::KEY_F13);
        assert_eq!(parse_key_name("KEY_F13").unwrap(), VirtualKeyCode::KEY_F13);
        assert_eq!(
            parse_key_name("RIGHTOPTION").unwrap(),
            VirtualKeyCode::KEY_RIGHTOPTION
        );
        assert_eq!(
            parse_key_name("RIGHTALT").unwrap(),
            VirtualKeyCode::KEY_RIGHTOPTION
        );
        assert_eq!(
            parse_key_name("CAPSLOCK").unwrap(),
            VirtualKeyCode::KEY_CAPSLOCK
        );
    }

    #[test]
    fn test_parse_key_name_scrolllock_error() {
        let result = parse_key_name("SCROLLLOCK");
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            HotkeyError::UnknownKey(msg) => {
                assert!(msg.contains("not available on macOS"));
            }
            _ => panic!("Expected UnknownKey error"),
        }
    }

    #[test]
    fn test_parse_modifier_name() {
        assert_eq!(
            parse_modifier_name("LEFTCTRL").unwrap(),
            CGEventFlags::CGEventFlagControl
        );
        assert_eq!(
            parse_modifier_name("CONTROL").unwrap(),
            CGEventFlags::CGEventFlagControl
        );
        assert_eq!(
            parse_modifier_name("OPTION").unwrap(),
            CGEventFlags::CGEventFlagAlternate
        );
        assert_eq!(
            parse_modifier_name("COMMAND").unwrap(),
            CGEventFlags::CGEventFlagCommand
        );
        assert_eq!(
            parse_modifier_name("SHIFT").unwrap(),
            CGEventFlags::CGEventFlagShift
        );
    }

    #[test]
    fn test_parse_modifier_name_error() {
        assert!(parse_modifier_name("INVALID_MOD").is_err());
    }

    #[test]
    fn test_check_modifiers_empty() {
        let current = CGEventFlags::CGEventFlagShift;
        let required = CGEventFlags::empty();
        assert!(check_modifiers(current, required));
    }

    #[test]
    fn test_check_modifiers_match() {
        let current = CGEventFlags::CGEventFlagShift | CGEventFlags::CGEventFlagControl;
        let required = CGEventFlags::CGEventFlagShift;
        assert!(check_modifiers(current, required));
    }

    #[test]
    fn test_check_modifiers_no_match() {
        let current = CGEventFlags::CGEventFlagShift;
        let required = CGEventFlags::CGEventFlagControl;
        assert!(!check_modifiers(current, required));
    }
}
