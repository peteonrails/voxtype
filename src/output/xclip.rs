//! xclip-based text output for X11
//!
//! Uses xclip to copy text to the X11 clipboard.
//! This is a fallback for X11 environments where wl-copy isn't available.
//!
//! Requires: xclip package installed

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// xclip-based text output for X11
pub struct XclipOutput {
    /// Text to append after transcription
    append_text: Option<String>,
}

impl XclipOutput {
    /// Create a new xclip output
    pub fn new(append_text: Option<String>) -> Self {
        Self { append_text }
    }
}

#[async_trait::async_trait]
impl TextOutput for XclipOutput {
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

        // Spawn xclip with stdin pipe, targeting the clipboard selection
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

        tracing::info!("Text copied to X11 clipboard ({} chars)", text.len());
        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if xclip is installed and DISPLAY is set (X11 environment)
        let xclip_installed = Command::new("which")
            .arg("xclip")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        // Only available if X11 DISPLAY is set
        let display_set = std::env::var("DISPLAY").is_ok();

        xclip_installed && display_set
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
        let output = XclipOutput::new(None);
        assert!(output.append_text.is_none());

        let output = XclipOutput::new(Some(" ".to_string()));
        assert_eq!(output.append_text, Some(" ".to_string()));
    }
}
