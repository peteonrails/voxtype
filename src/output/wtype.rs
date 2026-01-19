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
    /// Delay between keystrokes in milliseconds
    type_delay_ms: u32,
    /// Delay before typing starts (ms), allows virtual keyboard to initialize
    pre_type_delay_ms: u32,
}

impl WtypeOutput {
    /// Create a new wtype output
    pub fn new(
        notify: bool,
        auto_submit: bool,
        type_delay_ms: u32,
        pre_type_delay_ms: u32,
    ) -> Self {
        Self {
            notify,
            auto_submit,
            type_delay_ms,
            pre_type_delay_ms,
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
}

#[async_trait::async_trait]
impl TextOutput for WtypeOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        let mut cmd = Command::new("wtype");
        let mut debug_args = vec!["wtype".to_string()];

        // Add pre-typing delay if configured (helps prevent first character drop)
        if self.pre_type_delay_ms > 0 {
            cmd.arg("-s").arg(self.pre_type_delay_ms.to_string());
            debug_args.push(format!("-s {}", self.pre_type_delay_ms));
        }

        // Add inter-keystroke delay if configured
        if self.type_delay_ms > 0 {
            cmd.arg("-d").arg(self.type_delay_ms.to_string());
            debug_args.push(format!("-d {}", self.type_delay_ms));
        }

        debug_args.push("--".to_string());
        debug_args.push(format!("\"{}\"", text.chars().take(20).collect::<String>()));
        tracing::debug!("Running: {}", debug_args.join(" "));

        let output = cmd
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
        let output = WtypeOutput::new(true, false, 0, 0);
        assert!(output.notify);
        assert!(!output.auto_submit);
        assert_eq!(output.type_delay_ms, 0);
        assert_eq!(output.pre_type_delay_ms, 0);
    }

    #[test]
    fn test_new_with_enter() {
        let output = WtypeOutput::new(false, true, 0, 0);
        assert!(!output.notify);
        assert!(output.auto_submit);
    }

    #[test]
    fn test_new_with_type_delay() {
        let output = WtypeOutput::new(false, false, 50, 0);
        assert!(!output.notify);
        assert!(!output.auto_submit);
        assert_eq!(output.type_delay_ms, 50);
        assert_eq!(output.pre_type_delay_ms, 0);
    }

    #[test]
    fn test_new_with_pre_type_delay() {
        let output = WtypeOutput::new(false, false, 0, 200);
        assert_eq!(output.type_delay_ms, 0);
        assert_eq!(output.pre_type_delay_ms, 200);
    }
}
