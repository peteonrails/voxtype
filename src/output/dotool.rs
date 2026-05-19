//! dotool-based text output
//!
//! Uses dotool to simulate keyboard input with proper keyboard layout support.
//! Unlike ydotool, dotool respects keyboard layouts via DOTOOL_XKB_LAYOUT.
//!
//! ## Fast path: dotoold + dotoolc
//!
//! dotool ships a daemon/client pair specifically for low-latency repeated
//! typing. When `dotoold` is running, voxtype detects its FIFO and routes
//! every output() call through `dotoolc` — which simply relays commands
//! to the long-lived daemon. The ~700ms uinput device setup is paid once
//! at daemon startup, not on every typed segment. Sub-10ms per call.
//!
//! Strongly recommended for streaming backends (Parakeet, Soniox), where
//! 60+ output() calls land per session — without the daemon, the first
//! call alone stalls for nearly a second. Voxtype's Arch package
//! installs `dotoold` as a dependency; setup is out of scope here.
//!
//! Keyboard layout (`DOTOOL_XKB_LAYOUT`) applies to **the daemon, not the
//! client**. Set the env var on dotoold's startup; voxtype's
//! `dotool_xkb_layout` config setting cannot override a running daemon.
//!
//! ## Fallback path: direct dotool
//!
//! When `dotoold` isn't running, voxtype spawns `dotool` directly per
//! call. This is correct but pays the full uinput init cost (~700ms) on
//! every typed segment — fine for one-shot batch transcription, painful
//! for streaming.
//!
//! ## Requirements
//!
//! - dotool installed (https://sr.ht/~geb/dotool/)
//! - User in 'input' group for uinput access
//! - DOTOOL_XKB_LAYOUT set (on dotoold or in voxtype config) for non-US
//!   keyboard layouts

use super::TextOutput;
use crate::error::OutputError;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// dotool-based text output with keyboard layout support.
pub struct DotoolOutput {
    /// Delay between keypresses in milliseconds
    type_delay_ms: u32,
    /// Delay before typing starts in milliseconds
    pre_type_delay_ms: u32,
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
            auto_submit,
            append_text,
            xkb_layout,
            xkb_variant,
        }
    }

    /// Public wrapper for the FIFO-detection helper so backspace paths
    /// (in `output/streaming.rs`) can decide whether to use `dotoolc` too.
    pub fn live_daemon_pipe_path() -> Option<PathBuf> {
        Self::daemon_pipe_path()
    }

    /// Detect whether `dotoold` is actually running and accepting input.
    /// Returns the FIFO path only when it exists, is a FIFO, AND opening
    /// it `O_WRONLY | O_NONBLOCK` succeeds — i.e. some process is reading
    /// the other end. A crashed daemon leaves the FIFO on disk; the
    /// kernel returns ENXIO from a non-blocking write-open in that case,
    /// so we cleanly fall back to direct `dotool`.
    fn daemon_pipe_path() -> Option<PathBuf> {
        use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
        let path = std::env::var("DOTOOL_PIPE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp/dotool-pipe"));
        let meta = std::fs::metadata(&path).ok()?;
        if !meta.file_type().is_fifo() {
            return None;
        }
        std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .ok()?;
        Some(path)
    }

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
        let pipe = Self::daemon_pipe_path();
        let binary = if pipe.is_some() { "dotoolc" } else { "dotool" };
        let set_layout_env = pipe.is_none();
        tracing::debug!(target: "voxtype::dotool::wire", "-> {:?}", commands);

        let mut cmd = Command::new(binary);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(ref pipe) = pipe {
            cmd.env("DOTOOL_PIPE", pipe);
        }
        if set_layout_env {
            if let Some(ref layout) = self.xkb_layout {
                cmd.env("DOTOOL_XKB_LAYOUT", layout);
            }
            if let Some(ref variant) = self.xkb_variant {
                cmd.env("DOTOOL_XKB_VARIANT", variant);
            }
        }

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                OutputError::DotoolNotFound
            } else {
                OutputError::InjectionFailed(format!("Failed to spawn {}: {}", binary, e))
            }
        })?;

        // Write commands to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(commands.as_bytes()).await.map_err(|e| {
                OutputError::InjectionFailed(format!("Failed to write to {} stdin: {}", binary, e))
            })?;
            // Close stdin to signal end of input
            drop(stdin);
        }

        // Wait for dotool to complete
        let output = child.wait_with_output().await.map_err(|e| {
            OutputError::InjectionFailed(format!("Failed to wait for {}: {}", binary, e))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Check for common errors
            if stderr.contains("uinput") || stderr.contains("permission") {
                return Err(OutputError::InjectionFailed(format!(
                    "{}: uinput permission denied. Is user in 'input' group?",
                    binary
                )));
            }
            return Err(OutputError::InjectionFailed(format!(
                "{} exited with error: {}",
                binary, stderr
            )));
        }

        tracing::info!("Text typed via {} ({} chars)", binary, text.chars().count());
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
        let output = DotoolOutput::new(10, 0, false, None, Some("de".to_string()), None);
        assert_eq!(output.type_delay_ms, 10);
        assert_eq!(output.pre_type_delay_ms, 0);
        assert!(!output.auto_submit);
        assert_eq!(output.xkb_layout, Some("de".to_string()));
    }

    #[test]
    fn build_commands_basic() {
        let output = DotoolOutput::new(0, 0, false, None, None, None);
        let cmds = output.build_commands("Hello world");
        assert_eq!(cmds, "type Hello world\n");
    }

    #[test]
    fn build_commands_with_delay() {
        let output = DotoolOutput::new(17, 0, false, None, None, None);
        let cmds = output.build_commands("Test");
        assert!(cmds.contains("typedelay 17"));
        assert!(cmds.contains("typehold 17"));
        assert!(cmds.contains("type Test"));
    }

    #[test]
    fn build_commands_auto_submit_appends_enter() {
        let output = DotoolOutput::new(0, 0, true, None, None, None);
        let cmds = output.build_commands("hi");
        assert!(cmds.contains("key enter"));
    }

    #[test]
    fn build_commands_appends_text_before_enter() {
        let output = DotoolOutput::new(0, 0, true, Some(".".to_string()), None, None);
        let cmds = output.build_commands("hi");
        let dot_pos = cmds.find("type .\n").unwrap();
        let enter_pos = cmds.find("key enter\n").unwrap();
        assert!(dot_pos < enter_pos);
    }

    #[test]
    fn daemon_pipe_detection_respects_env_var() {
        std::env::set_var("DOTOOL_PIPE", "/nonexistent/dotool-pipe-test");
        assert!(DotoolOutput::daemon_pipe_path().is_none());
        std::env::remove_var("DOTOOL_PIPE");
    }
}
