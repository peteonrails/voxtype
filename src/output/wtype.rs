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
    /// Whether to convert newlines to Shift+Enter
    shift_enter_newlines: bool,
}

impl WtypeOutput {
    /// Create a new wtype output
    pub fn new(notify: bool, auto_submit: bool, shift_enter_newlines: bool) -> Self {
        Self {
            notify,
            auto_submit,
            shift_enter_newlines,
        }
    }

    /// Send a desktop notification
    async fn send_notification(&self, text: &str) {
        // Truncate preview for notification
        let preview: String = text.chars().take(100).collect();
        let preview = if text.chars().count() > 100 {
            format!("{}...", preview)
        } else {
            preview
        };

        let _ = Command::new("notify-send")
            .args([
                "--app-name=Voxtype",
                "--expire-time=3000",
                "Transcribed",
                &preview,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }

    /// Send Shift+Enter keypress using wtype
    async fn send_shift_enter(&self) -> Result<(), OutputError> {
        let output = Command::new("wtype")
            .args(["-M", "shift", "-k", "Return", "-m", "shift"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                OutputError::InjectionFailed(format!("wtype Shift+Enter failed: {}", e))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OutputError::InjectionFailed(format!(
                "wtype Shift+Enter failed: {}",
                stderr
            )));
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl TextOutput for WtypeOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        if self.shift_enter_newlines {
            // Split text by newlines and output each segment with Shift+Enter between them
            let segments: Vec<&str> = text.split('\n').collect();
            
            for (i, segment) in segments.iter().enumerate() {
                // Output the segment (even if empty, to preserve newline positions)
                if !segment.is_empty() {
                    let output = Command::new("wtype")
                        .arg("--")
                        .arg(segment)
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
                }

                // Send Shift+Enter after each segment except the last one
                if i < segments.len() - 1 {
                    if let Err(e) = self.send_shift_enter().await {
                        tracing::warn!("Failed to send Shift+Enter: {}", e);
                        // Continue anyway - best effort
                    }
                }
            }
        } else {
            // Original behavior: output text as-is
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
        let output = WtypeOutput::new(true, false, false);
        assert!(output.notify);
        assert!(!output.auto_submit);
        assert!(!output.shift_enter_newlines);
    }

    #[test]
    fn test_new_with_enter() {
        let output = WtypeOutput::new(false, true, false);
        assert!(!output.notify);
        assert!(output.auto_submit);
        assert!(!output.shift_enter_newlines);
    }

    #[test]
    fn test_new_with_shift_enter_newlines() {
        let output = WtypeOutput::new(false, false, true);
        assert!(!output.notify);
        assert!(!output.auto_submit);
        assert!(output.shift_enter_newlines);
    }
}
