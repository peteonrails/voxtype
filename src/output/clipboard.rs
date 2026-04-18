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
    /// Text to append after transcription
    append_text: Option<String>,
}

impl ClipboardOutput {
    /// Create a new clipboard output
    pub fn new(append_text: Option<String>) -> Self {
        Self { append_text }
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
        let output = ClipboardOutput::new(None);
        assert!(output.append_text.is_none());

        let output = ClipboardOutput::new(Some(" ".to_string()));
        assert_eq!(output.append_text, Some(" ".to_string()));
    }
}
