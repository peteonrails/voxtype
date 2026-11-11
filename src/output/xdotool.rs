//! xdotool-based text output
//!
//! Uses xdotool to simulate keyboard input on X11 via the XTEST extension.
//! No daemon required — xdotool talks to the X server directly, so it works
//! on any X11 session (also inside XWayland, for X11 windows).
//!
//! Requirements:
//! - xdotool installed
//! - An X11 display (DISPLAY set, X server reachable)

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// xdotool-based text output.
pub struct XdotoolOutput {
    /// Delay between keypresses in milliseconds
    type_delay_ms: u32,
    /// Delay before typing starts in milliseconds
    pre_type_delay_ms: u32,
    /// Whether to send Enter key after output
    auto_submit: bool,
    /// Text to append after transcription (before auto_submit)
    append_text: Option<String>,
}

impl XdotoolOutput {
    /// Create a new xdotool output
    pub fn new(
        type_delay_ms: u32,
        pre_type_delay_ms: u32,
        auto_submit: bool,
        append_text: Option<String>,
    ) -> Self {
        Self {
            type_delay_ms,
            pre_type_delay_ms,
            auto_submit,
            append_text,
        }
    }

    /// Run `xdotool type` for the given text.
    async fn type_text(&self, text: &str) -> Result<(), OutputError> {
        let mut cmd = Command::new("xdotool");
        cmd.arg("type")
            .arg("--delay")
            .arg(self.type_delay_ms.to_string())
            .arg("--clearmodifiers")
            .arg("--")
            .arg(text)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        tracing::debug!(
            "Running: xdotool type --delay {} --clearmodifiers -- \"{}\"",
            self.type_delay_ms,
            text.chars().take(20).collect::<String>()
        );

        let output = cmd.output().await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                OutputError::XdotoolNotFound
            } else {
                OutputError::InjectionFailed(e.to_string())
            }
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OutputError::InjectionFailed(format!(
                "xdotool type failed: {}",
                stderr.trim()
            )));
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl TextOutput for XdotoolOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }

        // Pre-typing delay if configured
        if self.pre_type_delay_ms > 0 {
            tracing::debug!(
                "xdotool: sleeping {}ms before typing",
                self.pre_type_delay_ms
            );
            tokio::time::sleep(Duration::from_millis(self.pre_type_delay_ms as u64)).await;
        }

        self.type_text(text).await?;

        // Append text if configured (e.g., a space to separate sentences)
        if let Some(append) = &self.append_text {
            self.type_text(append).await?;
        }

        // Send Enter key if configured
        if self.auto_submit {
            let mut enter_cmd = Command::new("xdotool");
            enter_cmd
                .args(["key", "--clearmodifiers", "Return"])
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            let enter_output = enter_cmd.output().await.map_err(|e| {
                OutputError::InjectionFailed(format!("xdotool Enter failed: {}", e))
            })?;
            if !enter_output.status.success() {
                let stderr = String::from_utf8_lossy(&enter_output.stderr);
                tracing::warn!("Failed to send Enter key: {}", stderr);
            }
        }

        tracing::info!("Text typed via xdotool ({} chars)", text.chars().count());

        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if xdotool exists in PATH
        let which_result = Command::new("which")
            .arg("xdotool")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        which_result.map(|s| s.success()).unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "xdotool"
    }
}
