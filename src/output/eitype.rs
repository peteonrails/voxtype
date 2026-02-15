//! eitype-based text output
//!
//! Uses eitype to simulate keyboard input via the Emulated Input (EI) protocol.
//! This works on compositors that support libei, including GNOME/Mutter and KDE,
//! which do not support the virtual-keyboard protocol used by wtype.
//!
//! Requires:
//! - eitype installed
//! - Compositor with EI protocol support (GNOME, KDE, Sway with libei)

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::process::Command;

/// eitype-based text output
pub struct EitypeOutput {
    /// Whether to send Enter key after output
    auto_submit: bool,
    /// Delay between key events in milliseconds
    type_delay_ms: u32,
    /// Delay before typing starts (ms)
    pre_type_delay_ms: u32,
    /// Convert newlines to Shift+Enter (for apps where Enter submits)
    shift_enter_newlines: bool,
}

impl EitypeOutput {
    /// Create a new eitype output
    pub fn new(
        auto_submit: bool,
        type_delay_ms: u32,
        pre_type_delay_ms: u32,
        shift_enter_newlines: bool,
    ) -> Self {
        Self {
            auto_submit,
            type_delay_ms,
            pre_type_delay_ms,
            shift_enter_newlines,
        }
    }

    /// Type a string of text using eitype
    async fn type_text(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // eitype doesn't have a pre-type delay flag, so sleep if needed
        if self.pre_type_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                self.pre_type_delay_ms as u64,
            ))
            .await;
        }

        let mut cmd = Command::new("eitype");
        let mut debug_args = vec!["eitype".to_string()];

        // Add inter-keystroke delay if configured
        if self.type_delay_ms > 0 {
            cmd.arg("-d").arg(self.type_delay_ms.to_string());
            debug_args.push(format!("-d {}", self.type_delay_ms));
        }

        debug_args.push(format!("\"{}\"", text.chars().take(20).collect::<String>()));
        tracing::debug!("Running: {}", debug_args.join(" "));

        let output = cmd
            .arg(text)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    OutputError::EitypeNotFound
                } else {
                    OutputError::InjectionFailed(e.to_string())
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OutputError::InjectionFailed(format!(
                "eitype failed: {}",
                stderr
            )));
        }

        Ok(())
    }

    /// Send Shift+Enter key combination using eitype
    async fn send_shift_enter(&self) -> Result<(), OutputError> {
        let output = Command::new("eitype")
            .args(["-M", "shift", "-k", "return"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                OutputError::InjectionFailed(format!("eitype Shift+Enter failed: {}", e))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("Failed to send Shift+Enter: {}", stderr);
        }

        Ok(())
    }

    /// Send Enter key using eitype
    async fn send_enter(&self) -> Result<(), OutputError> {
        let output = Command::new("eitype")
            .args(["-k", "return"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| OutputError::InjectionFailed(format!("eitype Enter failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("Failed to send Enter key: {}", stderr);
        }

        Ok(())
    }

    /// Output text with newlines converted to Shift+Enter
    async fn output_with_shift_enter_newlines(&self, text: &str) -> Result<(), OutputError> {
        let segments: Vec<&str> = text.split('\n').collect();

        for (i, segment) in segments.iter().enumerate() {
            // Type the text segment
            if !segment.is_empty() {
                self.type_text(segment).await?;
            }

            // Send Shift+Enter between segments (not after the last one)
            if i < segments.len() - 1 {
                self.send_shift_enter().await?;
            }
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl TextOutput for EitypeOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // If shift_enter_newlines is enabled, process text with Shift+Enter for newlines
        if self.shift_enter_newlines && text.contains('\n') {
            self.output_with_shift_enter_newlines(text).await?;
        } else {
            self.type_text(text).await?;
        }

        // Send Enter key if auto_submit is configured
        if self.auto_submit {
            self.send_enter().await?;
        }

        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if eitype exists in PATH
        Command::new("which")
            .arg("eitype")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "eitype"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = EitypeOutput::new(false, 0, 0, false);
        assert!(!output.auto_submit);
        assert_eq!(output.type_delay_ms, 0);
        assert_eq!(output.pre_type_delay_ms, 0);
        assert!(!output.shift_enter_newlines);
    }

    #[test]
    fn test_new_with_enter() {
        let output = EitypeOutput::new(true, 0, 0, false);
        assert!(output.auto_submit);
    }

    #[test]
    fn test_new_with_type_delay() {
        let output = EitypeOutput::new(false, 50, 0, false);
        assert_eq!(output.type_delay_ms, 50);
        assert_eq!(output.pre_type_delay_ms, 0);
    }

    #[test]
    fn test_new_with_pre_type_delay() {
        let output = EitypeOutput::new(false, 0, 200, false);
        assert_eq!(output.type_delay_ms, 0);
        assert_eq!(output.pre_type_delay_ms, 200);
    }

    #[test]
    fn test_new_with_shift_enter_newlines() {
        let output = EitypeOutput::new(false, 0, 0, true);
        assert!(output.shift_enter_newlines);
    }
}
