//! dotool-based text output
//!
//! Uses dotool to simulate keyboard input with proper keyboard layout support.
//! Unlike ydotool, dotool respects keyboard layouts via DOTOOL_XKB_LAYOUT.
//!
//! Requires:
//! - dotool installed (https://sr.ht/~geb/dotool/)
//! - User in 'input' group for uinput access
//! - DOTOOL_XKB_LAYOUT set for non-US layouts

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// dotool-based text output with keyboard layout support
pub struct DotoolOutput {
    /// Delay between keypresses in milliseconds
    type_delay_ms: u32,
    /// Delay before typing starts in milliseconds
    pre_type_delay_ms: u32,
    /// Whether to show a desktop notification
    notify: bool,
    /// Whether to send Enter key after output
    auto_submit: bool,
    /// Text to append after transcription (before auto_submit)
    append_text: Option<String>,
    /// Keyboard layout (e.g., "de" for German, "fr" for French)
    xkb_layout: Option<String>,
    /// Keyboard layout variant (e.g., "nodeadkeys")
    xkb_variant: Option<String>,
}

impl DotoolOutput {
    /// Create a new dotool output
    pub fn new(
        type_delay_ms: u32,
        pre_type_delay_ms: u32,
        notify: bool,
        auto_submit: bool,
        append_text: Option<String>,
        xkb_layout: Option<String>,
        xkb_variant: Option<String>,
    ) -> Self {
        if let Some(ref layout) = xkb_layout {
            tracing::debug!("dotool: using keyboard layout '{}'", layout);
        }
        Self {
            type_delay_ms,
            pre_type_delay_ms,
            notify,
            auto_submit,
            append_text,
            xkb_layout,
            xkb_variant,
        }
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

    /// Build the dotool command string to send via stdin
    fn build_commands(&self, text: &str) -> String {
        let mut commands = String::new();

        // Set delays if configured
        if self.type_delay_ms > 0 {
            commands.push_str(&format!("typedelay {}\n", self.type_delay_ms));
            commands.push_str(&format!("typehold {}\n", self.type_delay_ms));
        }

        // Type the text
        // Note: dotool's type command takes text on the same line
        commands.push_str(&format!("type {}\n", text));

        // Append text if configured (e.g., a space to separate sentences)
        if let Some(ref append) = self.append_text {
            commands.push_str(&format!("type {}\n", append));
        }

        // Send Enter key if auto_submit is enabled
        if self.auto_submit {
            commands.push_str("key enter\n");
        }

        commands
    }
}

#[async_trait::async_trait]
impl TextOutput for DotoolOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // Pre-typing delay if configured
        if self.pre_type_delay_ms > 0 {
            tracing::debug!(
                "dotool: sleeping {}ms before typing",
                self.pre_type_delay_ms
            );
            tokio::time::sleep(Duration::from_millis(self.pre_type_delay_ms as u64)).await;
        }

        let commands = self.build_commands(text);
        tracing::debug!(
            "dotool: sending commands for text: \"{}\"",
            text.chars().take(20).collect::<String>()
        );

        // Spawn dotool with stdin pipe
        let mut cmd = Command::new("dotool");
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        // Set keyboard layout environment variables if configured
        if let Some(ref layout) = self.xkb_layout {
            cmd.env("DOTOOL_XKB_LAYOUT", layout);
        }
        if let Some(ref variant) = self.xkb_variant {
            cmd.env("DOTOOL_XKB_VARIANT", variant);
        }

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                OutputError::DotoolNotFound
            } else {
                OutputError::InjectionFailed(format!("Failed to spawn dotool: {}", e))
            }
        })?;

        // Write commands to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(commands.as_bytes()).await.map_err(|e| {
                OutputError::InjectionFailed(format!("Failed to write to dotool stdin: {}", e))
            })?;
            // Close stdin to signal end of input
            drop(stdin);
        }

        // Wait for dotool to complete
        let output = child.wait_with_output().await.map_err(|e| {
            OutputError::InjectionFailed(format!("Failed to wait for dotool: {}", e))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Check for common errors
            if stderr.contains("uinput") || stderr.contains("permission") {
                return Err(OutputError::InjectionFailed(
                    "dotool: uinput permission denied. Is user in 'input' group?".to_string(),
                ));
            }

            return Err(OutputError::InjectionFailed(format!(
                "dotool exited with error: {}",
                stderr
            )));
        }

        tracing::info!("Text typed via dotool ({} chars)", text.len());

        // Send notification if enabled
        if self.notify {
            self.send_notification(text).await;
        }

        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if dotool exists in PATH
        Command::new("which")
            .arg("dotool")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "dotool"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let output = DotoolOutput::new(10, 0, true, false, None, Some("de".to_string()), None);
        assert_eq!(output.type_delay_ms, 10);
        assert_eq!(output.pre_type_delay_ms, 0);
        assert!(output.notify);
        assert!(!output.auto_submit);
        assert_eq!(output.xkb_layout, Some("de".to_string()));
    }

    #[test]
    fn test_build_commands_simple() {
        let output = DotoolOutput::new(0, 0, false, false, None, None, None);
        let cmds = output.build_commands("Hello world");
        assert_eq!(cmds, "type Hello world\n");
    }

    #[test]
    fn test_build_commands_with_delay() {
        let output = DotoolOutput::new(10, 0, false, false, None, None, None);
        let cmds = output.build_commands("Test");
        assert!(cmds.contains("typedelay 10"));
        assert!(cmds.contains("typehold 10"));
        assert!(cmds.contains("type Test"));
    }

    #[test]
    fn test_build_commands_with_enter() {
        let output = DotoolOutput::new(0, 0, false, true, None, None, None);
        let cmds = output.build_commands("Test");
        assert!(cmds.contains("type Test"));
        assert!(cmds.contains("key enter"));
    }
}
