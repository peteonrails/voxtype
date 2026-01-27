//! macOS text output via osascript/AppleScript
//!
//! Uses System Events to simulate keyboard input on macOS.
//! Requires Accessibility permissions for the terminal/app running voxtype.
//!
//! This is the primary typing method on macOS.

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::process::Command;

/// macOS text output using osascript
pub struct OsascriptOutput {
    /// Whether to show a desktop notification
    notify: bool,
    /// Whether to send Enter key after text
    auto_submit: bool,
    /// Delay before typing starts (ms)
    pre_type_delay_ms: u32,
}

impl OsascriptOutput {
    /// Create a new osascript output
    pub fn new(notify: bool, auto_submit: bool, pre_type_delay_ms: u32) -> Self {
        Self {
            notify,
            auto_submit,
            pre_type_delay_ms,
        }
    }

    /// Send a desktop notification using osascript
    async fn send_notification(&self, text: &str) {
        // Truncate preview for notification
        let preview = if text.chars().count() > 80 {
            format!("{}...", text.chars().take(80).collect::<String>())
        } else {
            text.to_string()
        };

        // Escape for AppleScript string
        let escaped_preview = preview.replace('\\', "\\\\").replace('"', "\\\"");

        let script = format!(
            r#"display notification "{}" with title "Voxtype""#,
            escaped_preview
        );

        let _ = Command::new("osascript")
            .args(["-e", &script])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }

    /// Escape text for AppleScript string literal
    fn escape_for_applescript(text: &str) -> String {
        text.replace('\\', "\\\\").replace('"', "\\\"")
    }
}

/// Wait for all modifier keys (Option, Command, Control, Shift) to be released
/// This prevents typing garbage characters when hotkey uses a modifier
async fn wait_for_modifiers_release() {
    // Simple fixed delay - the AppleScript check was causing issues
    // 150ms is enough for the Option key to fully release
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
}

#[async_trait::async_trait]
impl TextOutput for OsascriptOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // Wait for modifier keys to be released (prevents Option-key garbage)
        wait_for_modifiers_release().await;

        // Additional pre-type delay if configured
        if self.pre_type_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                self.pre_type_delay_ms as u64,
            ))
            .await;
        }

        // Escape text for AppleScript
        let escaped_text = Self::escape_for_applescript(text);

        // Build AppleScript to type text
        // Using "keystroke" which types the text character by character
        let mut script = format!(
            r#"tell application "System Events" to keystroke "{}""#,
            escaped_text
        );

        // Add Enter key if auto_submit is enabled
        if self.auto_submit {
            script.push_str(r#"
tell application "System Events" to key code 36"#); // 36 = Return key
        }

        let output = Command::new("osascript")
            .args(["-e", &script])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    OutputError::InjectionFailed("osascript not found".to_string())
                } else {
                    OutputError::InjectionFailed(e.to_string())
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Check for common permission error
            if stderr.contains("not allowed") || stderr.contains("accessibility") {
                return Err(OutputError::InjectionFailed(
                    "Accessibility permission required. Grant access in System Settings > Privacy & Security > Accessibility".to_string()
                ));
            }
            return Err(OutputError::InjectionFailed(format!(
                "osascript failed: {}",
                stderr
            )));
        }

        // Send notification if enabled
        if self.notify {
            self.send_notification(text).await;
        }

        tracing::info!("Text typed via osascript ({} chars)", text.len());
        Ok(())
    }

    async fn is_available(&self) -> bool {
        // osascript is always available on macOS
        cfg!(target_os = "macos")
            && Command::new("which")
                .arg("osascript")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "osascript (macOS)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = OsascriptOutput::new(true, false, 0);
        assert!(output.notify);
        assert!(!output.auto_submit);
        assert_eq!(output.pre_type_delay_ms, 0);
    }

    #[test]
    fn test_escape_for_applescript() {
        assert_eq!(
            OsascriptOutput::escape_for_applescript(r#"hello "world""#),
            r#"hello \"world\""#
        );
        assert_eq!(
            OsascriptOutput::escape_for_applescript(r#"path\to\file"#),
            r#"path\\to\\file"#
        );
    }
}
