//! CGEvent-based text output for macOS
//!
//! Uses the macOS CGEvent API to simulate keyboard input. This is the native
//! method for text injection on macOS.
//!
//! Requires:
//! - macOS 10.15 or later
//! - Accessibility permissions (System Preferences > Security & Privacy > Accessibility)
//!
//! The implementation supports:
//! - Standard US keyboard layout character-to-keycode mapping
//! - Unicode character support via CGEventKeyboardSetUnicodeString
//! - Configurable typing delays
//! - Auto-submit (Enter key) after output

use super::TextOutput;
use crate::error::OutputError;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::time::Duration;

/// CGEvent-based text output for macOS
pub struct CGEventOutput {
    /// Delay between keypresses in milliseconds
    type_delay_ms: u32,
    /// Delay before typing starts in milliseconds
    pre_type_delay_ms: u32,
    /// Whether to show a desktop notification
    notify: bool,
    /// Whether to send Enter key after output
    auto_submit: bool,
}

impl CGEventOutput {
    /// Create a new CGEvent output
    pub fn new(
        type_delay_ms: u32,
        pre_type_delay_ms: u32,
        notify: bool,
        auto_submit: bool,
    ) -> Self {
        Self {
            type_delay_ms,
            pre_type_delay_ms,
            notify,
            auto_submit,
        }
    }

    /// Check if we have Accessibility permissions
    ///
    /// On macOS, simulating keyboard events requires the app to be granted
    /// Accessibility permissions in System Preferences.
    fn check_accessibility_permission() -> bool {
        // Link against ApplicationServices framework for AXIsProcessTrusted
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        unsafe { AXIsProcessTrusted() }
    }

    /// Prompt the user to grant Accessibility permissions
    ///
    /// Logs instructions for granting permissions.
    fn prompt_accessibility_permission() {
        tracing::warn!(
            "Accessibility permission required. Please grant permission in:\n\
             System Preferences > Security & Privacy > Privacy > Accessibility\n\
             Then restart the application."
        );
    }

    /// Send a desktop notification using osascript
    async fn send_notification(&self, text: &str) {
        use std::process::Stdio;
        use tokio::process::Command;

        // Truncate preview for notification
        let preview: String = text.chars().take(100).collect();
        let preview = if text.chars().count() > 100 {
            format!("{}...", preview)
        } else {
            preview
        };

        // Escape quotes for AppleScript
        let escaped = preview.replace('\\', "\\\\").replace('"', "\\\"");

        let script = format!(
            "display notification \"{}\" with title \"Voxtype\" subtitle \"Transcribed\"",
            escaped
        );

        let _ = Command::new("osascript")
            .args(["-e", &script])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }

    /// Type a single character using CGEvent
    ///
    /// For ASCII characters, uses keycode mapping with proper modifiers.
    /// For Unicode characters, uses CGEventKeyboardSetUnicodeString.
    fn type_character(source: &CGEventSource, ch: char) -> Result<(), OutputError> {
        // Try to map to a keycode first (faster and more reliable for ASCII)
        if let Some((keycode, shift_needed)) = char_to_keycode(ch) {
            Self::press_key(source, keycode, shift_needed)?;
        } else {
            // Fall back to Unicode string injection for non-ASCII characters
            Self::type_unicode_char(source, ch)?;
        }
        Ok(())
    }

    /// Press a key with optional shift modifier
    fn press_key(
        source: &CGEventSource,
        keycode: CGKeyCode,
        shift_needed: bool,
    ) -> Result<(), OutputError> {
        // Create key down event
        let key_down = CGEvent::new_keyboard_event(source.clone(), keycode, true)
            .map_err(|_| OutputError::InjectionFailed("Failed to create key down event".into()))?;

        // Create key up event
        let key_up = CGEvent::new_keyboard_event(source.clone(), keycode, false)
            .map_err(|_| OutputError::InjectionFailed("Failed to create key up event".into()))?;

        // Set shift modifier if needed
        if shift_needed {
            key_down.set_flags(CGEventFlags::CGEventFlagShift);
            key_up.set_flags(CGEventFlags::CGEventFlagShift);
        }

        // Post the events
        key_down.post(CGEventTapLocation::HID);
        key_up.post(CGEventTapLocation::HID);

        Ok(())
    }

    /// Type a Unicode character using CGEventKeyboardSetUnicodeString
    fn type_unicode_char(source: &CGEventSource, ch: char) -> Result<(), OutputError> {
        // Create a keyboard event (keycode 0 is placeholder, we override with Unicode)
        let event = CGEvent::new_keyboard_event(source.clone(), 0, true)
            .map_err(|_| OutputError::InjectionFailed("Failed to create Unicode event".into()))?;

        // Convert char to UTF-16
        let mut utf16_buf = [0u16; 2];
        let utf16 = ch.encode_utf16(&mut utf16_buf);

        // Set the Unicode string on the event
        event.set_string_from_utf16_unchecked(utf16);

        // Post key down
        event.post(CGEventTapLocation::HID);

        // Create and post key up
        let event_up = CGEvent::new_keyboard_event(source.clone(), 0, false).map_err(|_| {
            OutputError::InjectionFailed("Failed to create Unicode up event".into())
        })?;
        event_up.set_string_from_utf16_unchecked(utf16);
        event_up.post(CGEventTapLocation::HID);

        Ok(())
    }

    /// Press the Enter/Return key
    fn press_enter(source: &CGEventSource) -> Result<(), OutputError> {
        Self::press_key(source, KEYCODE_RETURN, false)
    }

    /// Type text in a blocking manner (for use in spawn_blocking)
    ///
    /// This function handles all CGEvent operations synchronously,
    /// including inter-keystroke delays using std::thread::sleep.
    fn type_text_blocking(
        text: &str,
        type_delay_ms: u32,
        auto_submit: bool,
    ) -> Result<(), OutputError> {
        // Create event source
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| OutputError::InjectionFailed("Failed to create CGEventSource".into()))?;

        // Type each character
        let delay = Duration::from_millis(type_delay_ms as u64);
        for ch in text.chars() {
            Self::type_character(&source, ch)?;

            // Add delay between keystrokes if configured
            if type_delay_ms > 0 {
                std::thread::sleep(delay);
            }
        }

        // Send Enter key if configured
        if auto_submit {
            // Small delay before Enter to ensure all text is processed
            std::thread::sleep(Duration::from_millis(50));
            Self::press_enter(&source)?;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl TextOutput for CGEventOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // Check accessibility permissions first (can be done on main thread)
        if !Self::check_accessibility_permission() {
            Self::prompt_accessibility_permission();
            return Err(OutputError::InjectionFailed(
                "Accessibility permission denied. Grant permission in System Preferences > \
                Security & Privacy > Privacy > Accessibility, then restart the application."
                    .into(),
            ));
        }

        // Pre-typing delay if configured
        if self.pre_type_delay_ms > 0 {
            tracing::debug!(
                "cgevent: sleeping {}ms before typing",
                self.pre_type_delay_ms
            );
            tokio::time::sleep(Duration::from_millis(self.pre_type_delay_ms as u64)).await;
        }

        tracing::debug!(
            "cgevent: typing text: \"{}\"",
            text.chars().take(20).collect::<String>()
        );

        // CGEventSource is not Send, so we need to do all CGEvent work in a blocking task
        let text_owned = text.to_string();
        let type_delay_ms = self.type_delay_ms;
        let auto_submit = self.auto_submit;

        let result = tokio::task::spawn_blocking(move || {
            Self::type_text_blocking(&text_owned, type_delay_ms, auto_submit)
        })
        .await
        .map_err(|e| OutputError::InjectionFailed(format!("Task join error: {}", e)))??;

        tracing::info!("Text typed via CGEvent ({} chars)", text.len());

        // Send notification if enabled
        if self.notify {
            self.send_notification(text).await;
        }

        Ok(result)
    }

    async fn is_available(&self) -> bool {
        // CGEvent is always available on macOS, but we check for accessibility permissions
        // We return true here so the output method can provide a helpful error message
        // if permissions are denied
        true
    }

    fn name(&self) -> &'static str {
        "cgevent"
    }
}

// macOS virtual key codes for US keyboard layout
// Reference: /System/Library/Frameworks/Carbon.framework/Versions/A/Frameworks/HIToolbox.framework/Headers/Events.h

const KEYCODE_A: CGKeyCode = 0x00;
const KEYCODE_S: CGKeyCode = 0x01;
const KEYCODE_D: CGKeyCode = 0x02;
const KEYCODE_F: CGKeyCode = 0x03;
const KEYCODE_H: CGKeyCode = 0x04;
const KEYCODE_G: CGKeyCode = 0x05;
const KEYCODE_Z: CGKeyCode = 0x06;
const KEYCODE_X: CGKeyCode = 0x07;
const KEYCODE_C: CGKeyCode = 0x08;
const KEYCODE_V: CGKeyCode = 0x09;
const KEYCODE_B: CGKeyCode = 0x0B;
const KEYCODE_Q: CGKeyCode = 0x0C;
const KEYCODE_W: CGKeyCode = 0x0D;
const KEYCODE_E: CGKeyCode = 0x0E;
const KEYCODE_R: CGKeyCode = 0x0F;
const KEYCODE_Y: CGKeyCode = 0x10;
const KEYCODE_T: CGKeyCode = 0x11;
const KEYCODE_1: CGKeyCode = 0x12;
const KEYCODE_2: CGKeyCode = 0x13;
const KEYCODE_3: CGKeyCode = 0x14;
const KEYCODE_4: CGKeyCode = 0x15;
const KEYCODE_6: CGKeyCode = 0x16;
const KEYCODE_5: CGKeyCode = 0x17;
const KEYCODE_EQUAL: CGKeyCode = 0x18;
const KEYCODE_9: CGKeyCode = 0x19;
const KEYCODE_7: CGKeyCode = 0x1A;
const KEYCODE_MINUS: CGKeyCode = 0x1B;
const KEYCODE_8: CGKeyCode = 0x1C;
const KEYCODE_0: CGKeyCode = 0x1D;
const KEYCODE_RIGHT_BRACKET: CGKeyCode = 0x1E;
const KEYCODE_O: CGKeyCode = 0x1F;
const KEYCODE_U: CGKeyCode = 0x20;
const KEYCODE_LEFT_BRACKET: CGKeyCode = 0x21;
const KEYCODE_I: CGKeyCode = 0x22;
const KEYCODE_P: CGKeyCode = 0x23;
const KEYCODE_RETURN: CGKeyCode = 0x24;
const KEYCODE_L: CGKeyCode = 0x25;
const KEYCODE_J: CGKeyCode = 0x26;
const KEYCODE_QUOTE: CGKeyCode = 0x27;
const KEYCODE_K: CGKeyCode = 0x28;
const KEYCODE_SEMICOLON: CGKeyCode = 0x29;
const KEYCODE_BACKSLASH: CGKeyCode = 0x2A;
const KEYCODE_COMMA: CGKeyCode = 0x2B;
const KEYCODE_SLASH: CGKeyCode = 0x2C;
const KEYCODE_N: CGKeyCode = 0x2D;
const KEYCODE_M: CGKeyCode = 0x2E;
const KEYCODE_PERIOD: CGKeyCode = 0x2F;
const KEYCODE_TAB: CGKeyCode = 0x30;
const KEYCODE_SPACE: CGKeyCode = 0x31;
const KEYCODE_GRAVE: CGKeyCode = 0x32;

/// Map a character to a macOS virtual keycode and whether shift is needed
///
/// Returns Some((keycode, shift_needed)) for ASCII characters that can be
/// typed with the US keyboard layout, None for characters that need Unicode input.
pub fn char_to_keycode(ch: char) -> Option<(CGKeyCode, bool)> {
    match ch {
        // Lowercase letters (no shift)
        'a' => Some((KEYCODE_A, false)),
        'b' => Some((KEYCODE_B, false)),
        'c' => Some((KEYCODE_C, false)),
        'd' => Some((KEYCODE_D, false)),
        'e' => Some((KEYCODE_E, false)),
        'f' => Some((KEYCODE_F, false)),
        'g' => Some((KEYCODE_G, false)),
        'h' => Some((KEYCODE_H, false)),
        'i' => Some((KEYCODE_I, false)),
        'j' => Some((KEYCODE_J, false)),
        'k' => Some((KEYCODE_K, false)),
        'l' => Some((KEYCODE_L, false)),
        'm' => Some((KEYCODE_M, false)),
        'n' => Some((KEYCODE_N, false)),
        'o' => Some((KEYCODE_O, false)),
        'p' => Some((KEYCODE_P, false)),
        'q' => Some((KEYCODE_Q, false)),
        'r' => Some((KEYCODE_R, false)),
        's' => Some((KEYCODE_S, false)),
        't' => Some((KEYCODE_T, false)),
        'u' => Some((KEYCODE_U, false)),
        'v' => Some((KEYCODE_V, false)),
        'w' => Some((KEYCODE_W, false)),
        'x' => Some((KEYCODE_X, false)),
        'y' => Some((KEYCODE_Y, false)),
        'z' => Some((KEYCODE_Z, false)),

        // Uppercase letters (shift)
        'A' => Some((KEYCODE_A, true)),
        'B' => Some((KEYCODE_B, true)),
        'C' => Some((KEYCODE_C, true)),
        'D' => Some((KEYCODE_D, true)),
        'E' => Some((KEYCODE_E, true)),
        'F' => Some((KEYCODE_F, true)),
        'G' => Some((KEYCODE_G, true)),
        'H' => Some((KEYCODE_H, true)),
        'I' => Some((KEYCODE_I, true)),
        'J' => Some((KEYCODE_J, true)),
        'K' => Some((KEYCODE_K, true)),
        'L' => Some((KEYCODE_L, true)),
        'M' => Some((KEYCODE_M, true)),
        'N' => Some((KEYCODE_N, true)),
        'O' => Some((KEYCODE_O, true)),
        'P' => Some((KEYCODE_P, true)),
        'Q' => Some((KEYCODE_Q, true)),
        'R' => Some((KEYCODE_R, true)),
        'S' => Some((KEYCODE_S, true)),
        'T' => Some((KEYCODE_T, true)),
        'U' => Some((KEYCODE_U, true)),
        'V' => Some((KEYCODE_V, true)),
        'W' => Some((KEYCODE_W, true)),
        'X' => Some((KEYCODE_X, true)),
        'Y' => Some((KEYCODE_Y, true)),
        'Z' => Some((KEYCODE_Z, true)),

        // Numbers (no shift)
        '0' => Some((KEYCODE_0, false)),
        '1' => Some((KEYCODE_1, false)),
        '2' => Some((KEYCODE_2, false)),
        '3' => Some((KEYCODE_3, false)),
        '4' => Some((KEYCODE_4, false)),
        '5' => Some((KEYCODE_5, false)),
        '6' => Some((KEYCODE_6, false)),
        '7' => Some((KEYCODE_7, false)),
        '8' => Some((KEYCODE_8, false)),
        '9' => Some((KEYCODE_9, false)),

        // Shifted number row symbols
        '!' => Some((KEYCODE_1, true)),
        '@' => Some((KEYCODE_2, true)),
        '#' => Some((KEYCODE_3, true)),
        '$' => Some((KEYCODE_4, true)),
        '%' => Some((KEYCODE_5, true)),
        '^' => Some((KEYCODE_6, true)),
        '&' => Some((KEYCODE_7, true)),
        '*' => Some((KEYCODE_8, true)),
        '(' => Some((KEYCODE_9, true)),
        ')' => Some((KEYCODE_0, true)),

        // Punctuation (no shift)
        '-' => Some((KEYCODE_MINUS, false)),
        '=' => Some((KEYCODE_EQUAL, false)),
        '[' => Some((KEYCODE_LEFT_BRACKET, false)),
        ']' => Some((KEYCODE_RIGHT_BRACKET, false)),
        '\\' => Some((KEYCODE_BACKSLASH, false)),
        ';' => Some((KEYCODE_SEMICOLON, false)),
        '\'' => Some((KEYCODE_QUOTE, false)),
        ',' => Some((KEYCODE_COMMA, false)),
        '.' => Some((KEYCODE_PERIOD, false)),
        '/' => Some((KEYCODE_SLASH, false)),
        '`' => Some((KEYCODE_GRAVE, false)),

        // Punctuation (shift)
        '_' => Some((KEYCODE_MINUS, true)),
        '+' => Some((KEYCODE_EQUAL, true)),
        '{' => Some((KEYCODE_LEFT_BRACKET, true)),
        '}' => Some((KEYCODE_RIGHT_BRACKET, true)),
        '|' => Some((KEYCODE_BACKSLASH, true)),
        ':' => Some((KEYCODE_SEMICOLON, true)),
        '"' => Some((KEYCODE_QUOTE, true)),
        '<' => Some((KEYCODE_COMMA, true)),
        '>' => Some((KEYCODE_PERIOD, true)),
        '?' => Some((KEYCODE_SLASH, true)),
        '~' => Some((KEYCODE_GRAVE, true)),

        // Whitespace
        ' ' => Some((KEYCODE_SPACE, false)),
        '\t' => Some((KEYCODE_TAB, false)),
        '\n' => Some((KEYCODE_RETURN, false)),

        // All other characters need Unicode input
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = CGEventOutput::new(10, 0, true, false);
        assert_eq!(output.type_delay_ms, 10);
        assert_eq!(output.pre_type_delay_ms, 0);
        assert!(output.notify);
        assert!(!output.auto_submit);
    }

    #[test]
    fn test_new_with_auto_submit() {
        let output = CGEventOutput::new(0, 0, false, true);
        assert_eq!(output.type_delay_ms, 0);
        assert!(!output.notify);
        assert!(output.auto_submit);
    }

    #[test]
    fn test_new_with_pre_type_delay() {
        let output = CGEventOutput::new(0, 200, false, false);
        assert_eq!(output.type_delay_ms, 0);
        assert_eq!(output.pre_type_delay_ms, 200);
    }

    #[test]
    fn test_char_to_keycode_lowercase() {
        assert_eq!(char_to_keycode('a'), Some((KEYCODE_A, false)));
        assert_eq!(char_to_keycode('z'), Some((KEYCODE_Z, false)));
        assert_eq!(char_to_keycode('m'), Some((KEYCODE_M, false)));
    }

    #[test]
    fn test_char_to_keycode_uppercase() {
        assert_eq!(char_to_keycode('A'), Some((KEYCODE_A, true)));
        assert_eq!(char_to_keycode('Z'), Some((KEYCODE_Z, true)));
        assert_eq!(char_to_keycode('M'), Some((KEYCODE_M, true)));
    }

    #[test]
    fn test_char_to_keycode_numbers() {
        assert_eq!(char_to_keycode('0'), Some((KEYCODE_0, false)));
        assert_eq!(char_to_keycode('1'), Some((KEYCODE_1, false)));
        assert_eq!(char_to_keycode('9'), Some((KEYCODE_9, false)));
    }

    #[test]
    fn test_char_to_keycode_shifted_numbers() {
        assert_eq!(char_to_keycode('!'), Some((KEYCODE_1, true)));
        assert_eq!(char_to_keycode('@'), Some((KEYCODE_2, true)));
        assert_eq!(char_to_keycode('#'), Some((KEYCODE_3, true)));
        assert_eq!(char_to_keycode('$'), Some((KEYCODE_4, true)));
        assert_eq!(char_to_keycode('%'), Some((KEYCODE_5, true)));
        assert_eq!(char_to_keycode('^'), Some((KEYCODE_6, true)));
        assert_eq!(char_to_keycode('&'), Some((KEYCODE_7, true)));
        assert_eq!(char_to_keycode('*'), Some((KEYCODE_8, true)));
        assert_eq!(char_to_keycode('('), Some((KEYCODE_9, true)));
        assert_eq!(char_to_keycode(')'), Some((KEYCODE_0, true)));
    }

    #[test]
    fn test_char_to_keycode_punctuation() {
        assert_eq!(char_to_keycode('.'), Some((KEYCODE_PERIOD, false)));
        assert_eq!(char_to_keycode(','), Some((KEYCODE_COMMA, false)));
        assert_eq!(char_to_keycode(';'), Some((KEYCODE_SEMICOLON, false)));
        assert_eq!(char_to_keycode(':'), Some((KEYCODE_SEMICOLON, true)));
        assert_eq!(char_to_keycode('\''), Some((KEYCODE_QUOTE, false)));
        assert_eq!(char_to_keycode('"'), Some((KEYCODE_QUOTE, true)));
    }

    #[test]
    fn test_char_to_keycode_whitespace() {
        assert_eq!(char_to_keycode(' '), Some((KEYCODE_SPACE, false)));
        assert_eq!(char_to_keycode('\t'), Some((KEYCODE_TAB, false)));
        assert_eq!(char_to_keycode('\n'), Some((KEYCODE_RETURN, false)));
    }

    #[test]
    fn test_char_to_keycode_unicode() {
        // Unicode characters should return None (need Unicode input)
        assert_eq!(char_to_keycode('\u{00E9}'), None); // e with acute accent
        assert_eq!(char_to_keycode('\u{00F1}'), None); // n with tilde
        assert_eq!(char_to_keycode('\u{4E2D}'), None); // Chinese character
    }

    #[test]
    fn test_char_to_keycode_brackets() {
        assert_eq!(char_to_keycode('['), Some((KEYCODE_LEFT_BRACKET, false)));
        assert_eq!(char_to_keycode(']'), Some((KEYCODE_RIGHT_BRACKET, false)));
        assert_eq!(char_to_keycode('{'), Some((KEYCODE_LEFT_BRACKET, true)));
        assert_eq!(char_to_keycode('}'), Some((KEYCODE_RIGHT_BRACKET, true)));
    }
}
