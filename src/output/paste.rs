//! Paste-based text output
//!
//! Uses wl-copy to copy text to clipboard, then simulates Ctrl+V with ydotool.
//! This works around non-US keyboard layout issues by avoiding direct typing.
//!
//! Requires:
//! - wl-copy installed (for clipboard access)
//! - ydotool installed (for Ctrl+V simulation)
//! - ydotoold daemon running (systemctl --user start ydotool)

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Paste-based text output (clipboard + Ctrl+V)
pub struct PasteOutput {
    /// Whether to show a desktop notification
    notify: bool,
}

impl PasteOutput {
    /// Create a new paste output
    pub fn new(notify: bool) -> Self {
        Self { notify }
    }

    /// Send a desktop notification
    async fn send_notification(&self, text: &str) {
        // Truncate preview for notification
        let preview = if text.len() > 80 {
            format!("{}...", &text[..80])
        } else {
            text.to_string()
        };

        let _ = Command::new("notify-send")
            .args([
                "--app-name=Voxtype",
                "--urgency=low",
                "--expire-time=3000",
                "Pasted text",
                &preview,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }

    /// Copy text to clipboard using wl-copy
    async fn copy_to_clipboard(&self, text: &str) -> Result<(), OutputError> {
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

        Ok(())
    }

    /// Simulate Ctrl+V key combination using ydotool
    async fn simulate_ctrl_v(&self) -> Result<(), OutputError> {
        // Use ydotool to press Ctrl+V (Left Ctrl + V)
        // 29 = KEY_LEFTCTRL, 47 = KEY_V
        // Format: key_code:1 (press) then key_code:0 (release)
        let output = Command::new("ydotool")
            .args(["key", "29:1", "47:1", "47:0", "29:0"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    OutputError::YdotoolNotFound
                } else {
                    OutputError::CtrlVFailed(e.to_string())
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Check for common errors
            if stderr.contains("socket") || stderr.contains("connect") || stderr.contains("daemon")
            {
                return Err(OutputError::YdotoolNotRunning);
            }

            return Err(OutputError::CtrlVFailed(stderr.to_string()));
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl TextOutput for PasteOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // Step 1: Copy to clipboard
        self.copy_to_clipboard(text).await?;

        // Small delay to ensure clipboard is set before pasting
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Step 2: Simulate Ctrl+V
        self.simulate_ctrl_v().await?;

        // Send notification if enabled
        if self.notify {
            self.send_notification(text).await;
        }

        tracing::info!("Text pasted via clipboard + Ctrl+V ({} chars)", text.len());
        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if wl-copy exists
        let wl_copy_available = Command::new("which")
            .arg("wl-copy")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if !wl_copy_available {
            return false;
        }

        // Check if ydotool exists
        let ydotool_available = Command::new("which")
            .arg("ydotool")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if !ydotool_available {
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
        "paste (clipboard + Ctrl+V)"
    }
}
