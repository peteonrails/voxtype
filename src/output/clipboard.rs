//! Clipboard-based text output
//!
//! Uses wl-copy to copy text to the Wayland clipboard.
//! This is the most reliable fallback as it works on all Wayland compositors.
//!
//! Requires: wl-clipboard package installed

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Clipboard-based text output
pub struct ClipboardOutput {
    /// Whether to show a desktop notification
    notify: bool,
    /// Custom message template for transcription complete notification
    message_template: Option<String>,
}

impl ClipboardOutput {
    /// Create a new clipboard output
    pub fn new(notify: bool, message_template: Option<String>) -> Self {
        Self { notify, message_template }
    }

    /// Format the notification message using template or default
    fn format_message(&self, text: &str) -> String {
        match &self.message_template {
            Some(template) => template.replace("{text}", text),
            None => {
                // Truncate preview for notification (use chars() to handle multi-byte UTF-8)
                if text.chars().count() > 80 {
                    format!("{}...", text.chars().take(80).collect::<String>())
                } else {
                    text.to_string()
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
                "--urgency=low",
                "--expire-time=3000",
                "Copied to clipboard",
                &message,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
}

#[async_trait::async_trait]
impl TextOutput for ClipboardOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // Spawn wl-copy with stdin pipe
        let mut child = Command::new("wl-copy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    OutputError::WlCopyNotFound
                } else {
                    OutputError::InjectionFailed(e.to_string())
                }
            })?;

        // Write text to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .await
                .map_err(|e| OutputError::InjectionFailed(e.to_string()))?;
            
            // Close stdin to signal EOF
            drop(stdin);
        }

        // Wait for completion
        let status = child
            .wait()
            .await
            .map_err(|e| OutputError::InjectionFailed(e.to_string()))?;

        if !status.success() {
            return Err(OutputError::InjectionFailed(
                "wl-copy exited with error".to_string(),
            ));
        }

        // Send notification if enabled
        if self.notify {
            self.send_notification(text).await;
        }

        tracing::info!("Text copied to clipboard ({} chars)", text.len());
        Ok(())
    }

    async fn is_available(&self) -> bool {
        Command::new("which")
            .arg("wl-copy")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "clipboard (wl-copy)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = ClipboardOutput::new(true, None);
        assert!(output.notify);
        assert!(output.message_template.is_none());

        let output = ClipboardOutput::new(false, None);
        assert!(!output.notify);
    }

    #[test]
    fn test_custom_message_template() {
        let output = ClipboardOutput::new(true, Some("Copied: {text}".to_string()));
        assert_eq!(output.format_message("test"), "Copied: test");
    }
}
