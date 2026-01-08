//! wtype-based text output
//!
//! Uses wtype to simulate keyboard input on Wayland. This is the preferred
//! method on Wayland because:
//! - No daemon required (unlike ydotool)
//! - Better Unicode/CJK support
//!
//! Requires:
//! - wtype installed
//! - Running on Wayland (WAYLAND_DISPLAY set)

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::process::Command;

/// wtype-based text output
pub struct WtypeOutput {
    /// Whether to show a desktop notification
    notify: bool,
    /// Whether to send Enter key after output
    auto_submit: bool,
    /// Custom message template for transcription complete notification
    /// Use {text} as placeholder for the transcribed text
    message_template: Option<String>,
}

impl WtypeOutput {
    /// Create a new wtype output
    pub fn new(notify: bool, auto_submit: bool, message_template: Option<String>) -> Self {
        Self {
            notify,
            auto_submit,
            message_template,
        }
    }

    /// Format the notification message using template or default
    fn format_message(&self, text: &str) -> String {
        match &self.message_template {
            Some(template) => template.replace("{text}", text),
            None => {
                // Default: truncate preview for notification
                let preview: String = text.chars().take(100).collect();
                if text.chars().count() > 100 {
                    format!("{}...", preview)
                } else {
                    preview
                }
            }
        }
    }

    /// Send a desktop notification
    async fn send_notification(&self, text: &str) {
        let message = self.format_message(text);

        let _ = Command::new("notify-send")
            .args([
                "--app-name=Voxtype",
                "--expire-time=3000",
                "Transcribed",
                &message,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
}

#[async_trait::async_trait]
impl TextOutput for WtypeOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        let output = Command::new("wtype")
            .arg("--")
            .arg(text)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    OutputError::WtypeNotFound
                } else {
                    OutputError::InjectionFailed(e.to_string())
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OutputError::InjectionFailed(format!(
                "wtype failed: {}",
                stderr
            )));
        }

        // Send Enter key if configured
        if self.auto_submit {
            let enter_output = Command::new("wtype")
                .args(["-k", "Return"])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|e| OutputError::InjectionFailed(format!("wtype Enter failed: {}", e)))?;

            if !enter_output.status.success() {
                let stderr = String::from_utf8_lossy(&enter_output.stderr);
                tracing::warn!("Failed to send Enter key: {}", stderr);
            }
        }

        // Send notification if enabled
        if self.notify {
            self.send_notification(text).await;
        }

        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Just check if wtype exists in PATH
        // Don't check WAYLAND_DISPLAY - systemd services may not have it
        // wtype will fail naturally if Wayland isn't available
        Command::new("which")
            .arg("wtype")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "wtype"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = WtypeOutput::new(true, false, None);
        assert!(output.notify);
        assert!(!output.auto_submit);
        assert!(output.message_template.is_none());
    }

    #[test]
    fn test_new_with_enter() {
        let output = WtypeOutput::new(false, true, None);
        assert!(!output.notify);
        assert!(output.auto_submit);
    }

    #[test]
    fn test_custom_message_template() {
        let output = WtypeOutput::new(true, false, Some("You said: {text}".to_string()));
        assert_eq!(output.format_message("hello world"), "You said: hello world");
    }

    #[test]
    fn test_default_message_truncation() {
        let output = WtypeOutput::new(true, false, None);
        let long_text = "a".repeat(150);
        let formatted = output.format_message(&long_text);
        assert!(formatted.ends_with("..."));
        assert!(formatted.len() < 150);
    }
}
