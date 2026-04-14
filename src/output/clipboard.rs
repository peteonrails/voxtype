//! Clipboard-based text output
//!
//! On Linux, uses wl-copy to copy text to the Wayland clipboard.
//! On macOS, uses pbcopy to copy text to the system clipboard.

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Get the clipboard copy command for the current platform
fn clipboard_copy_cmd() -> &'static str {
    if cfg!(target_os = "macos") {
        "pbcopy"
    } else {
        "wl-copy"
    }
}

/// Clipboard-based text output
pub struct ClipboardOutput {
    /// Whether to show a desktop notification
    notify: bool,
    /// Text to append after transcription
    append_text: Option<String>,
}

impl ClipboardOutput {
    /// Create a new clipboard output
    pub fn new(notify: bool, append_text: Option<String>) -> Self {
        Self {
            notify,
            append_text,
        }
    }

    /// Send a desktop notification
    async fn send_notification(&self, text: &str) {
        // Truncate preview for notification (use chars() to handle multi-byte UTF-8)
        let preview = if text.chars().count() > 80 {
            format!("{}...", text.chars().take(80).collect::<String>())
        } else {
            text.to_string()
        };

        super::send_desktop_notification("Copied to clipboard", &preview).await;
    }
}

#[async_trait::async_trait]
impl TextOutput for ClipboardOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // Prepare text with optional append
        let text = if let Some(ref append) = self.append_text {
            std::borrow::Cow::Owned(format!("{}{}", text, append))
        } else {
            std::borrow::Cow::Borrowed(text)
        };

        let cmd = clipboard_copy_cmd();

        // Spawn clipboard command with stdin pipe
        let mut child = Command::new(cmd)
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
                format!("{} exited with error", cmd),
            ));
        }

        // Send notification if enabled
        if self.notify {
            self.send_notification(&text).await;
        }

        tracing::info!("Text copied to clipboard ({} chars)", text.len());
        Ok(())
    }

    async fn is_available(&self) -> bool {
        let cmd = clipboard_copy_cmd();
        Command::new("which")
            .arg(cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            "clipboard (pbcopy)"
        } else {
            "clipboard (wl-copy)"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = ClipboardOutput::new(true, None);
        assert!(output.notify);

        let output = ClipboardOutput::new(false, None);
        assert!(!output.notify);
    }
}
