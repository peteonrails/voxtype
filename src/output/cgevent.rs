//! macOS text output via CGEvent API
//!
//! Uses Core Graphics events to simulate keyboard input on macOS.
//! This is the native, preferred method for text injection on macOS.
//!
//! Requires Accessibility permissions:
//!   System Settings > Privacy & Security > Accessibility
//!
//! Advantages over osascript:
//! - Native API, no subprocess spawning
//! - Direct Unicode support via CGEventKeyboardSetUnicodeString
//! - Lower latency and better reliability
//! - Proper keycode mapping with modifier support

use super::TextOutput;
use crate::error::OutputError;
use core_foundation::base::TCFType;
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

    /// Check if Accessibility permissions are granted
    fn check_accessibility_permission() -> bool {
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        unsafe { AXIsProcessTrusted() }
    }

    /// Request Accessibility permissions (shows system dialog)
    #[allow(dead_code)]
    fn request_accessibility_permission() {
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrustedWithOptions(options: core_foundation::base::CFTypeRef) -> bool;
        }

        use core_foundation::base::CFType;
        use core_foundation::boolean::CFBoolean;
        use core_foundation::dictionary::CFDictionary;
        use core_foundation::string::CFString;

        let key = CFString::new("AXTrustedCheckOptionPrompt");
        let value = CFBoolean::true_value();
        let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);

        unsafe {
            AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef() as _);
        }
    }

    /// Send a desktop notification using osascript
    async fn send_notification(&self, text: &str) {
        use std::process::Stdio;
        use tokio::process::Command;

        let preview: String = text.chars().take(80).collect();
        let preview = if text.chars().count() > 80 {
            format!("{}...", preview)
        } else {
            preview
        };

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

    /// Type text using CGEvent (blocking, for use in spawn_blocking)
    fn type_text_blocking(
        text: &str,
        type_delay_ms: u32,
        auto_submit: bool,
    ) -> Result<(), OutputError> {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| OutputError::InjectionFailed("Failed to create CGEventSource".into()))?;

        let delay = Duration::from_millis(type_delay_ms as u64);

        // Type text using Unicode string injection for reliability
        // This works with any keyboard layout and supports all characters
        for chunk in text.chars().collect::<Vec<_>>().chunks(20) {
            Self::type_unicode_string(&source, chunk)?;

            if type_delay_ms > 0 && !chunk.is_empty() {
                std::thread::sleep(delay);
            }
        }

        if auto_submit {
            std::thread::sleep(Duration::from_millis(50));
            Self::press_key(&source, KEYCODE_RETURN, CGEventFlags::empty())?;
        }

        Ok(())
    }

    /// Type a string using Unicode injection (handles any character)
    fn type_unicode_string(source: &CGEventSource, chars: &[char]) -> Result<(), OutputError> {
        if chars.is_empty() {
            return Ok(());
        }

        // Convert to UTF-16 for CGEvent
        let mut utf16_buf: Vec<u16> = Vec::with_capacity(chars.len() * 2);
        for ch in chars {
            let mut buf = [0u16; 2];
            let encoded = ch.encode_utf16(&mut buf);
            utf16_buf.extend_from_slice(encoded);
        }

        // Create key down event with Unicode string
        let event = CGEvent::new_keyboard_event(source.clone(), 0, true)
            .map_err(|_| OutputError::InjectionFailed("Failed to create keyboard event".into()))?;

        event.set_string_from_utf16_unchecked(&utf16_buf);
        event.post(CGEventTapLocation::HID);

        // Key up event
        let event_up = CGEvent::new_keyboard_event(source.clone(), 0, false)
            .map_err(|_| OutputError::InjectionFailed("Failed to create key up event".into()))?;
        event_up.post(CGEventTapLocation::HID);

        Ok(())
    }

    /// Press a single key with optional modifiers
    ///
    /// Always explicitly sets flags to prevent Caps Lock or stuck modifiers
    /// from interfering with text injection.
    fn press_key(
        source: &CGEventSource,
        keycode: CGKeyCode,
        flags: CGEventFlags,
    ) -> Result<(), OutputError> {
        let key_down = CGEvent::new_keyboard_event(source.clone(), keycode, true)
            .map_err(|_| OutputError::InjectionFailed("Failed to create key down event".into()))?;

        // Always set flags explicitly - use CGEventFlagNull when no modifiers needed
        // This prevents Caps Lock or stuck modifier keys from causing random capitalization
        key_down.set_flags(flags);
        key_down.post(CGEventTapLocation::HID);

        let key_up = CGEvent::new_keyboard_event(source.clone(), keycode, false)
            .map_err(|_| OutputError::InjectionFailed("Failed to create key up event".into()))?;
        key_up.set_flags(flags);
        key_up.post(CGEventTapLocation::HID);

        Ok(())
    }
}

// macOS virtual key codes (from Carbon HIToolbox Events.h)
const KEYCODE_RETURN: CGKeyCode = 0x24;

#[async_trait::async_trait]
impl TextOutput for CGEventOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // Check permissions first
        if !Self::check_accessibility_permission() {
            return Err(OutputError::InjectionFailed(
                "Accessibility permission required.\n\
                 Grant access in: System Settings > Privacy & Security > Accessibility\n\
                 Then restart voxtype."
                    .into(),
            ));
        }

        // Pre-typing delay
        if self.pre_type_delay_ms > 0 {
            tracing::debug!(
                "cgevent: waiting {}ms before typing",
                self.pre_type_delay_ms
            );
            tokio::time::sleep(Duration::from_millis(self.pre_type_delay_ms as u64)).await;
        }

        tracing::debug!("cgevent: typing {} chars", text.chars().count());

        // CGEventSource is not Send, so do all CGEvent work in spawn_blocking
        let text_owned = text.to_string();
        let type_delay_ms = self.type_delay_ms;
        let auto_submit = self.auto_submit;

        tokio::task::spawn_blocking(move || {
            Self::type_text_blocking(&text_owned, type_delay_ms, auto_submit)
        })
        .await
        .map_err(|e| OutputError::InjectionFailed(format!("Task join error: {}", e)))??;

        tracing::info!("Text typed via CGEvent ({} chars)", text.chars().count());

        if self.notify {
            self.send_notification(text).await;
        }

        Ok(())
    }

    async fn is_available(&self) -> bool {
        // CGEvent is available on macOS, return true to allow helpful error message
        // if permissions are denied
        true
    }

    fn name(&self) -> &'static str {
        "cgevent (macOS native)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = CGEventOutput::new(10, 100, true, false);
        assert_eq!(output.type_delay_ms, 10);
        assert_eq!(output.pre_type_delay_ms, 100);
        assert!(output.notify);
        assert!(!output.auto_submit);
    }

    #[test]
    fn test_new_with_auto_submit() {
        let output = CGEventOutput::new(0, 0, false, true);
        assert!(!output.notify);
        assert!(output.auto_submit);
    }
}
