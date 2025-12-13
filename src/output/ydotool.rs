//! ydotool-based text output
//!
//! Uses ydotool to simulate keyboard input. This works on all Wayland
//! compositors because ydotool uses the uinput kernel interface.
//!
//! Requires:
//! - ydotool installed
//! - ydotoold daemon running (systemctl --user start ydotool)
//! - User in 'input' group

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::process::Command;

/// ydotool-based text output
pub struct YdotoolOutput {
    /// Delay between keypresses in milliseconds
    delay_ms: u32,
    /// Whether to show a desktop notification
    notify: bool,
}

impl YdotoolOutput {
    /// Create a new ydotool output
    pub fn new(delay_ms: u32, notify: bool) -> Self {
        Self { delay_ms, notify }
    }

    /// Send a desktop notification
    async fn send_notification(&self, text: &str) {
        // Truncate preview for notification
        let preview: String = text.chars().take(100).collect();
        let preview = if text.len() > 100 {
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
impl TextOutput for YdotoolOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        let mut cmd = Command::new("ydotool");
        cmd.arg("type");

        // Add delay if specified
        if self.delay_ms > 0 {
            cmd.arg("--key-delay").arg(self.delay_ms.to_string());
            cmd.arg("--key-hold").arg(self.delay_ms.to_string());
        }

        // The -- ensures text starting with - isn't treated as an option
        cmd.arg("--").arg(text);

        let output = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    OutputError::YdotoolNotFound
                } else {
                    OutputError::InjectionFailed(e.to_string())
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Check for common errors
            if stderr.contains("socket") || stderr.contains("connect") || stderr.contains("daemon")
            {
                return Err(OutputError::YdotoolNotRunning);
            }

            return Err(OutputError::InjectionFailed(stderr.to_string()));
        }

        // Send notification if enabled
        if self.notify {
            self.send_notification(text).await;
        }

        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if ydotool exists in PATH
        let which_result = Command::new("which")
            .arg("ydotool")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        if !which_result.map(|s| s.success()).unwrap_or(false) {
            return false;
        }

        // Check if ydotoold is running by trying a no-op
        // ydotool type "" should succeed quickly if daemon is running
        Command::new("ydotool")
            .args(["type", ""])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "ydotool"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = YdotoolOutput::new(10, true);
        assert_eq!(output.delay_ms, 10);
        assert!(output.notify);
    }
}
