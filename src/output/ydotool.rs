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
use std::time::Duration;
use tokio::process::Command;

/// ydotool-based text output
pub struct YdotoolOutput {
    /// Delay between keypresses in milliseconds
    type_delay_ms: u32,
    /// Delay before typing starts in milliseconds
    pre_type_delay_ms: u32,
    /// Whether to show a desktop notification
    notify: bool,
    /// Whether ydotool supports --key-hold flag (added in newer versions)
    supports_key_hold: bool,
    /// Whether to send Enter key after output
    auto_submit: bool,
}

impl YdotoolOutput {
    /// Create a new ydotool output
    ///
    /// Detects ydotool capabilities at construction time.
    pub fn new(
        type_delay_ms: u32,
        pre_type_delay_ms: u32,
        notify: bool,
        auto_submit: bool,
    ) -> Self {
        let supports_key_hold = Self::detect_key_hold_support();
        if supports_key_hold {
            tracing::debug!("ydotool supports --key-hold flag");
        } else {
            tracing::debug!("ydotool does not support --key-hold flag, using --key-delay only");
        }
        Self {
            type_delay_ms,
            pre_type_delay_ms,
            notify,
            supports_key_hold,
            auto_submit,
        }
    }

    /// Detect if ydotool supports the --key-hold flag
    ///
    /// Older versions of ydotool don't have this flag and silently ignore it
    /// (exiting with code 0), which can cause subtle issues.
    fn detect_key_hold_support() -> bool {
        std::process::Command::new("ydotool")
            .args(["type", "--help"])
            .output()
            .map(|output| {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                stdout.contains("--key-hold") || stderr.contains("--key-hold")
            })
            .unwrap_or(false)
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

        // Pre-typing delay if configured
        if self.pre_type_delay_ms > 0 {
            tracing::debug!(
                "ydotool: sleeping {}ms before typing",
                self.pre_type_delay_ms
            );
            tokio::time::sleep(Duration::from_millis(self.pre_type_delay_ms as u64)).await;
        }

        let mut cmd = Command::new("ydotool");
        cmd.arg("type");

        // Always set delay explicitly (ydotool defaults to 12ms if not specified)
        cmd.arg("--key-delay").arg(self.type_delay_ms.to_string());

        // Use --key-hold only if supported (older versions silently ignore unknown flags)
        if self.supports_key_hold {
            cmd.arg("--key-hold").arg(self.type_delay_ms.to_string());
        }

        // The -- ensures text starting with - isn't treated as an option
        cmd.arg("--").arg(text);

        tracing::debug!(
            "Running: ydotool type --key-delay {} {} -- \"{}\"",
            self.type_delay_ms,
            if self.supports_key_hold {
                format!("--key-hold {}", self.type_delay_ms)
            } else {
                String::new()
            },
            text.chars().take(20).collect::<String>()
        );

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

        // Send Enter key if configured
        // ydotool key uses evdev key codes: 28 is KEY_ENTER
        // Format: keycode:press (1) then keycode:release (0)
        if self.auto_submit {
            let enter_output = Command::new("ydotool")
                .args(["key", "28:1", "28:0"])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|e| {
                    OutputError::InjectionFailed(format!("ydotool Enter failed: {}", e))
                })?;

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
        let output = YdotoolOutput::new(10, 0, true, false);
        assert_eq!(output.type_delay_ms, 10);
        assert_eq!(output.pre_type_delay_ms, 0);
        assert!(output.notify);
        assert!(!output.auto_submit);
        // supports_key_hold depends on system ydotool version, so we just check it's set
        let _ = output.supports_key_hold;
    }

    #[test]
    fn test_new_with_enter() {
        let output = YdotoolOutput::new(0, 0, false, true);
        assert_eq!(output.type_delay_ms, 0);
        assert!(!output.notify);
        assert!(output.auto_submit);
    }

    #[test]
    fn test_new_with_pre_type_delay() {
        let output = YdotoolOutput::new(0, 200, false, false);
        assert_eq!(output.type_delay_ms, 0);
        assert_eq!(output.pre_type_delay_ms, 200);
    }

    #[test]
    fn test_detect_key_hold_support() {
        // This test will pass regardless of ydotool version - it just shouldn't panic
        let _supports = YdotoolOutput::detect_key_hold_support();
    }
}
