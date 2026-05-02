//! macOS clipboard output via pbcopy
//!
//! Uses the native macOS pbcopy command for clipboard access.
//! This is the clipboard fallback on macOS.

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// macOS clipboard output using pbcopy
pub struct PbcopyOutput {
    /// Whether to show a desktop notification
    notify: bool,
}

impl PbcopyOutput {
    /// Create a new pbcopy output
    pub fn new(notify: bool) -> Self {
        Self { notify }
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
            r#"display notification "{}" with title "Copied to clipboard""#,
            escaped_preview
        );

        let _ = Command::new("osascript")
            .args(["-e", &script])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
}

#[async_trait::async_trait]
impl TextOutput for PbcopyOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // Spawn pbcopy with stdin pipe
        let mut child = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    OutputError::InjectionFailed("pbcopy not found".to_string())
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
                "pbcopy exited with error".to_string(),
            ));
        }

        // Send notification if enabled
        if self.notify {
            self.send_notification(text).await;
        }

        tracing::info!("Text copied to clipboard via pbcopy ({} chars)", text.len());
        Ok(())
    }

    async fn is_available(&self) -> bool {
        // pbcopy is always available on macOS
        cfg!(target_os = "macos")
            && Command::new("which")
                .arg("pbcopy")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "clipboard (pbcopy)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = PbcopyOutput::new(true);
        assert!(output.notify);

        let output = PbcopyOutput::new(false);
        assert!(!output.notify);
    }
}
