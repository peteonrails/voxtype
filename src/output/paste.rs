//! Paste-based text output
//!
//! Uses wl-copy to copy text to clipboard, then simulates a paste keystroke.
//! This works around non-US keyboard layout issues by avoiding direct typing.
//!
//! Requires:
//! - wl-copy installed (for clipboard access)
//! - wtype OR ydotool installed (for keystroke simulation)
//!   - wtype: Wayland-native, no daemon needed (preferred)
//!   - ydotool: Works on X11/Wayland/TTY, requires ydotoold daemon

use super::TextOutput;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Parsed paste keystroke (modifiers + key)
#[derive(Debug, Clone)]
struct ParsedKeystroke {
    /// Modifier keys (e.g., ["ctrl"], ["shift"], ["ctrl", "shift"])
    modifiers: Vec<String>,
    /// The main key (e.g., "v", "insert")
    key: String,
}

impl ParsedKeystroke {
    /// Parse a keystroke string like "ctrl+v" or "shift+insert"
    fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();

        if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
            return Err("Invalid keystroke format".to_string());
        }

        if parts.len() == 1 {
            // Just a key, no modifiers
            return Ok(Self {
                modifiers: vec![],
                key: parts[0].to_lowercase(),
            });
        }

        // Last part is the key, rest are modifiers
        let key = parts.last().unwrap().to_lowercase();
        let modifiers: Vec<String> = parts[..parts.len() - 1]
            .iter()
            .map(|s| s.to_lowercase())
            .collect();

        Ok(Self { modifiers, key })
    }

    /// Convert to wtype arguments
    /// e.g., "ctrl+v" -> ["-M", "ctrl", "-k", "v", "-m", "ctrl"]
    fn to_wtype_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        // Press modifiers
        for modifier in &self.modifiers {
            args.push("-M".to_string());
            args.push(modifier.clone());
        }

        // Tap the key
        args.push("-k".to_string());
        args.push(self.key.clone());

        // Release modifiers (reverse order)
        for modifier in self.modifiers.iter().rev() {
            args.push("-m".to_string());
            args.push(modifier.clone());
        }

        args
    }

    /// Convert to ydotool key arguments using evdev codes
    /// e.g., "ctrl+v" -> ["29:1", "47:1", "47:0", "29:0"]
    fn to_ydotool_args(&self) -> Result<Vec<String>, String> {
        let mut args = Vec::new();

        // Get evdev codes for modifiers
        let modifier_codes: Vec<u16> = self
            .modifiers
            .iter()
            .map(|m| key_name_to_evdev(m))
            .collect::<Result<Vec<_>, _>>()?;

        // Get evdev code for main key
        let key_code = key_name_to_evdev(&self.key)?;

        // Press modifiers
        for code in &modifier_codes {
            args.push(format!("{}:1", code));
        }

        // Press and release the key
        args.push(format!("{}:1", key_code));
        args.push(format!("{}:0", key_code));

        // Release modifiers (reverse order)
        for code in modifier_codes.iter().rev() {
            args.push(format!("{}:0", code));
        }

        Ok(args)
    }
}

/// Convert a key name to its evdev code
fn key_name_to_evdev(name: &str) -> Result<u16, String> {
    match name.to_lowercase().as_str() {
        // Modifiers
        "ctrl" | "control" | "leftctrl" => Ok(29),  // KEY_LEFTCTRL
        "rightctrl" => Ok(97),                       // KEY_RIGHTCTRL
        "shift" | "leftshift" => Ok(42),            // KEY_LEFTSHIFT
        "rightshift" => Ok(54),                      // KEY_RIGHTSHIFT
        "alt" | "leftalt" => Ok(56),                // KEY_LEFTALT
        "rightalt" | "altgr" => Ok(100),            // KEY_RIGHTALT
        "super" | "meta" | "leftmeta" | "win" => Ok(125), // KEY_LEFTMETA

        // Common keys
        "v" => Ok(47),                               // KEY_V
        "insert" | "ins" => Ok(110),                // KEY_INSERT
        "enter" | "return" => Ok(28),               // KEY_ENTER

        // Letters (for completeness)
        "a" => Ok(30),
        "b" => Ok(48),
        "c" => Ok(46),
        "d" => Ok(32),
        "e" => Ok(18),
        "f" => Ok(33),
        "g" => Ok(34),
        "h" => Ok(35),
        "i" => Ok(23),
        "j" => Ok(36),
        "k" => Ok(37),
        "l" => Ok(38),
        "m" => Ok(50),
        "n" => Ok(49),
        "o" => Ok(24),
        "p" => Ok(25),
        "q" => Ok(16),
        "r" => Ok(19),
        "s" => Ok(31),
        "t" => Ok(20),
        "u" => Ok(22),
        "w" => Ok(17),
        "x" => Ok(45),
        "y" => Ok(21),
        "z" => Ok(44),

        other => Err(format!("Unknown key: {}", other)),
    }
}

/// Paste-based text output (clipboard + paste keystroke)
pub struct PasteOutput {
    /// Whether to show a desktop notification
    notify: bool,
    /// Whether to send Enter key after output
    auto_submit: bool,
    /// Parsed paste keystroke
    keystroke: ParsedKeystroke,
    /// Delay between key events in milliseconds (from config type_delay_ms)
    key_delay_ms: u32,
}

impl PasteOutput {
    /// Create a new paste output
    pub fn new(notify: bool, auto_submit: bool, paste_keys: Option<String>, key_delay_ms: u32) -> Self {
        let keystroke_str = paste_keys.as_deref().unwrap_or("ctrl+v");
        let keystroke = ParsedKeystroke::parse(keystroke_str).unwrap_or_else(|e| {
            tracing::warn!("Invalid paste_keys '{}': {}, using ctrl+v", keystroke_str, e);
            ParsedKeystroke::parse("ctrl+v").unwrap()
        });

        tracing::debug!("Paste keystroke configured: {:?}", keystroke);

        Self {
            notify,
            auto_submit,
            keystroke,
            key_delay_ms,
        }
    }

    /// Send a desktop notification
    async fn send_notification(&self, text: &str) {
        // Truncate preview for notification (use chars() to handle multi-byte UTF-8)
        let preview = if text.chars().count() > 80 {
            format!("{}...", text.chars().take(80).collect::<String>())
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

    /// Check if wtype is available
    async fn is_wtype_available(&self) -> bool {
        // Check if wtype exists
        let wtype_installed = Command::new("which")
            .arg("wtype")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if !wtype_installed {
            return false;
        }

        // Check if we're on Wayland
        std::env::var("WAYLAND_DISPLAY").is_ok()
    }

    /// Check if ydotool is available (installed and daemon running)
    async fn is_ydotool_available(&self) -> bool {
        // Check if ydotool exists
        let ydotool_installed = Command::new("which")
            .arg("ydotool")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if !ydotool_installed {
            return false;
        }

        // Check if ydotoold is running by trying a no-op
        Command::new("ydotool")
            .args(["type", ""])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Simulate paste keystroke using wtype
    async fn simulate_paste_wtype(&self) -> Result<(), OutputError> {
        let args = self.keystroke.to_wtype_args();
        tracing::debug!("Running: wtype {}", args.join(" "));

        let output = Command::new("wtype")
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    OutputError::WtypeNotFound
                } else {
                    OutputError::CtrlVFailed(e.to_string())
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OutputError::CtrlVFailed(format!("wtype failed: {}", stderr)));
        }

        Ok(())
    }

    /// Simulate paste keystroke using ydotool
    async fn simulate_paste_ydotool(&self) -> Result<(), OutputError> {
        let args = self.keystroke.to_ydotool_args().map_err(|e| {
            OutputError::CtrlVFailed(format!("Cannot convert keystroke for ydotool: {}", e))
        })?;

        tracing::debug!("Running: ydotool key {}, {}ms", args.join(" "), self.key_delay_ms);

        let mut cmd = Command::new("ydotool");
        cmd.arg("key");

        // Only add delay parameter if configured
        if self.key_delay_ms > 0 {
            cmd.arg(format!("-d {}", self.key_delay_ms));
        }

        let output = cmd
            .args(&args)
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

    /// Simulate paste keystroke, trying wtype first then ydotool
    async fn simulate_paste_keystroke(&self) -> Result<(), OutputError> {
        // Try wtype first (preferred - no daemon needed)
        if self.is_wtype_available().await {
            match self.simulate_paste_wtype().await {
                Ok(()) => {
                    tracing::debug!("Paste keystroke sent via wtype");
                    return Ok(());
                }
                Err(e) => {
                    tracing::debug!("wtype paste failed: {}, trying ydotool", e);
                }
            }
        }

        // Fall back to ydotool
        if self.is_ydotool_available().await {
            match self.simulate_paste_ydotool().await {
                Ok(()) => {
                    tracing::debug!("Paste keystroke sent via ydotool");
                    return Ok(());
                }
                Err(e) => {
                    tracing::debug!("ydotool paste failed: {}", e);
                    return Err(e);
                }
            }
        }

        Err(OutputError::CtrlVFailed(
            "Neither wtype nor ydotool available for paste keystroke".to_string(),
        ))
    }

    /// Send Enter key after paste
    async fn send_enter(&self) -> Result<(), OutputError> {
        // Try wtype first
        if self.is_wtype_available().await {
            let output = Command::new("wtype")
                .args(["-k", "Return"])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .await;

            if let Ok(out) = output {
                if out.status.success() {
                    return Ok(());
                }
            }
        }

        // Fall back to ydotool
        if self.is_ydotool_available().await {
            let output = Command::new("ydotool")
                .args(["key", "28:1", "28:0"])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .await;

            if let Ok(out) = output {
                if out.status.success() {
                    return Ok(());
                }
            }
        }

        // Best effort - don't fail the whole operation for Enter
        tracing::warn!("Failed to send Enter key");
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
        // Increased from 100ms to 200ms to improve reliability with ydotool
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Step 2: Simulate paste keystroke
        self.simulate_paste_keystroke().await?;

        // Send Enter key if configured
        if self.auto_submit {
            self.send_enter().await?;
        }

        // Send notification if enabled
        if self.notify {
            self.send_notification(text).await;
        }

        tracing::info!(
            "Text pasted via clipboard + {} ({} chars)",
            self.keystroke.modifiers.join("+")
                + if !self.keystroke.modifiers.is_empty() { "+" } else { "" }
                + &self.keystroke.key,
            text.len()
        );
        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if wl-copy exists (required for clipboard)
        let wl_copy_available = Command::new("which")
            .arg("wl-copy")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if !wl_copy_available {
            tracing::debug!("paste mode unavailable: wl-copy not found");
            return false;
        }

        // Check if EITHER wtype OR ydotool is available for keystroke simulation
        let wtype_available = self.is_wtype_available().await;
        let ydotool_available = self.is_ydotool_available().await;

        if !wtype_available && !ydotool_available {
            tracing::debug!(
                "paste mode unavailable: neither wtype nor ydotool available \
                (wtype needs WAYLAND_DISPLAY, ydotool needs daemon running)"
            );
            return false;
        }

        tracing::debug!(
            "paste mode available (wtype: {}, ydotool: {})",
            wtype_available,
            ydotool_available
        );
        true
    }

    fn name(&self) -> &'static str {
        "paste (clipboard + keystroke)"
    }
}
