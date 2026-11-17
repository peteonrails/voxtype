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
use crate::output::find_ydotool_socket;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// ydotool-based text output
/// Smallest delay that keeps KWin from dropping modifier key-up events.
const KDE_MIN_TYPE_DELAY_MS: u32 = 1;

/// KDE pacing rule, matching the one eitype uses (#552).
///
/// With `type_delay_ms = 0`, ydotool blasts press/release pairs through
/// uinput and KWin coalesces or drops the KEY_UP for modifiers. A stuck
/// Shift or Super then combines with every physical keypress until ydotoold
/// is restarted — the desktop becomes unusable, which is a far worse outcome
/// than one millisecond per key (#538).
///
/// An explicit non-zero delay is the user's choice and is left alone.
fn effective_type_delay_ms(configured_delay_ms: u32, current_desktop: Option<&str>) -> u32 {
    let is_kde = current_desktop.is_some_and(|desktop| {
        desktop
            .split(':')
            .any(|component| component.eq_ignore_ascii_case("kde"))
    });

    if is_kde && configured_delay_ms == 0 {
        KDE_MIN_TYPE_DELAY_MS
    } else {
        configured_delay_ms
    }
}

pub struct YdotoolOutput {
    /// Delay between keypresses in milliseconds
    type_delay_ms: u32,
    /// Delay before typing starts in milliseconds
    pre_type_delay_ms: u32,
    /// Whether ydotool supports --key-hold flag (added in newer versions)
    supports_key_hold: bool,
    /// Whether to send Enter key after output
    auto_submit: bool,
    /// Text to append after transcription (before auto_submit)
    append_text: Option<String>,
    /// Path to ydotoold socket, if found at a non-default location
    socket_path: Option<PathBuf>,
}

impl YdotoolOutput {
    /// Create a new ydotool output
    ///
    /// Detects ydotool capabilities at construction time.
    pub fn new(
        type_delay_ms: u32,
        pre_type_delay_ms: u32,
        auto_submit: bool,
        append_text: Option<String>,
    ) -> Self {
        let supports_key_hold = Self::detect_key_hold_support();
        if supports_key_hold {
            tracing::debug!("ydotool supports --key-hold flag");
        } else {
            tracing::debug!("ydotool does not support --key-hold flag, using --key-delay only");
        }
        let socket_path = find_ydotool_socket();
        Self {
            type_delay_ms,
            pre_type_delay_ms,
            supports_key_hold,
            auto_submit,
            append_text,
            socket_path,
        }
    }

    /// Apply the discovered socket path to a ydotool Command, if any.
    fn apply_socket_env(&self, cmd: &mut Command) {
        if let Some(ref path) = self.socket_path {
            cmd.env("YDOTOOL_SOCKET", path);
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
        self.apply_socket_env(&mut cmd);
        cmd.arg("type");

        // Always set delay explicitly (ydotool defaults to 12ms if not specified).
        // On KDE a configured zero is raised to 1ms so KWin does not drop
        // modifier key-up events and leave Shift or Super stuck (#538).
        let delay = effective_type_delay_ms(
            self.type_delay_ms,
            std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        );
        if delay != self.type_delay_ms {
            tracing::debug!(
                "ydotool: raising key delay from {}ms to {}ms on KDE to avoid stuck modifiers",
                self.type_delay_ms,
                delay
            );
        }
        cmd.arg("--key-delay").arg(delay.to_string());

        // Use --key-hold only if supported (older versions silently ignore unknown flags)
        if self.supports_key_hold {
            cmd.arg("--key-hold").arg(delay.to_string());
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

        // Append text if configured (e.g., a space to separate sentences)
        if let Some(ref append) = self.append_text {
            let mut append_cmd = Command::new("ydotool");
            self.apply_socket_env(&mut append_cmd);
            append_cmd.arg("type");
            append_cmd
                .arg("--key-delay")
                .arg(self.type_delay_ms.to_string());
            if self.supports_key_hold {
                append_cmd
                    .arg("--key-hold")
                    .arg(self.type_delay_ms.to_string());
            }
            append_cmd.arg("--").arg(append);

            let append_output = append_cmd
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|e| {
                    OutputError::InjectionFailed(format!("ydotool append text failed: {}", e))
                })?;

            if !append_output.status.success() {
                let stderr = String::from_utf8_lossy(&append_output.stderr);
                tracing::warn!("Failed to append text: {}", stderr);
            }
        }

        // Send Enter key if configured
        // ydotool key uses evdev key codes: 28 is KEY_ENTER
        // Format: keycode:press (1) then keycode:release (0)
        if self.auto_submit {
            let mut enter_cmd = Command::new("ydotool");
            self.apply_socket_env(&mut enter_cmd);
            let enter_output = enter_cmd
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
        let mut cmd = Command::new("ydotool");
        self.apply_socket_env(&mut cmd);
        cmd.args(["type", ""])
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

    /// #538: a configured zero on KDE leaves Shift or Super stuck, which
    /// takes the whole desktop with it until ydotoold is restarted.
    #[test]
    fn kde_zero_delay_is_raised() {
        assert_eq!(effective_type_delay_ms(0, Some("KDE")), 1);
        assert_eq!(effective_type_delay_ms(0, Some("ubuntu:KDE")), 1);
        assert_eq!(effective_type_delay_ms(0, Some("kde")), 1);
    }

    /// An explicit delay is the user's choice, including on KDE.
    #[test]
    fn explicit_delay_is_never_overridden() {
        assert_eq!(effective_type_delay_ms(5, Some("KDE")), 5);
        assert_eq!(effective_type_delay_ms(12, Some("KDE")), 12);
    }

    /// Everywhere else keeps zero, which is faster and causes no trouble.
    #[test]
    fn other_desktops_keep_zero() {
        assert_eq!(effective_type_delay_ms(0, Some("Hyprland")), 0);
        assert_eq!(effective_type_delay_ms(0, Some("GNOME")), 0);
        assert_eq!(effective_type_delay_ms(0, None), 0);
    }
    use super::*;

    #[test]
    fn test_new() {
        let output = YdotoolOutput::new(10, 0, false, None);
        assert_eq!(output.type_delay_ms, 10);
        assert_eq!(output.pre_type_delay_ms, 0);
        assert!(!output.auto_submit);
        // supports_key_hold depends on system ydotool version, so we just check it's set
        let _ = output.supports_key_hold;
    }

    #[test]
    fn test_new_with_enter() {
        let output = YdotoolOutput::new(0, 0, true, None);
        assert_eq!(output.type_delay_ms, 0);
        assert!(output.auto_submit);
    }

    #[test]
    fn test_new_with_pre_type_delay() {
        let output = YdotoolOutput::new(0, 200, false, None);
        assert_eq!(output.type_delay_ms, 0);
        assert_eq!(output.pre_type_delay_ms, 200);
    }

    #[test]
    fn test_detect_key_hold_support() {
        // This test will pass regardless of ydotool version - it just shouldn't panic
        let _supports = YdotoolOutput::detect_key_hold_support();
    }
}
