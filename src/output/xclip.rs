//! xclip-based text output for X11
//!
//! Uses xclip to copy text to the X11 clipboard.
//! This is a fallback for X11 environments where wl-copy is unavailable.
//!
//! Requires: xclip package installed

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// xclip-based text output for X11
pub struct XclipOutput {
    /// Whether to show a desktop notification
    notify: bool,
}

impl XclipOutput {
    /// Create a new xclip output
    pub fn new(notify: bool) -> Self {
        Self { notify }
    }

    /// Send a desktop notification
    async fn send_notification(&self, text: &str) {
        // Truncate preview for notification (use chars() to handle multi-byte UTF-8)
        let preview = if text.chars().count() > 80 {
            format!("{}...", text.chars().take(80).collect::<String>())
        } else {
            text.to_string()
        };

        let _ = Command::new("notify-send")
            .args([
                "--app-name=Voxtype",
                "--urgency=low",
                "--expire-time=3000",
                "Copied to clipboard",
                &preview,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
}

#[async_trait::async_trait]
impl TextOutput for XclipOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // Spawn xclip with stdin pipe
        // -selection clipboard uses the CLIPBOARD selection (standard clipboard)
        let mut child = Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    OutputError::XclipNotFound
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
                "xclip exited with error".to_string(),
            ));
        }

        // Send notification if enabled
        if self.notify {
            self.send_notification(text).await;
        }

        tracing::info!("Text copied to clipboard via xclip ({} chars)", text.len());
        Ok(())
    }

    async fn is_available(&self) -> bool {
        Command::new("which")
            .arg("xclip")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "clipboard (xclip)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = XclipOutput::new(true);
        assert!(output.notify);

        let output = XclipOutput::new(false);
        assert!(!output.notify);
    }
}

