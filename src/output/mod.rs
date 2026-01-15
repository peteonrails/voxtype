//! Text output module
//!
//! Provides text output via keyboard simulation or clipboard.
//!
//! Fallback chain for `mode = "type"`:
//! 1. wtype - Wayland-native, best Unicode/CJK support, no daemon needed
//! 2. ydotool - Works on X11/Wayland/TTY, requires daemon
//! 3. clipboard - Universal fallback via wl-copy
//!
//! Paste mode (clipboard + Ctrl+V) helps with system with non US keyboard layouts.

pub mod clipboard;
pub mod paste;
pub mod post_process;
pub mod wtype;
pub mod ydotool;

use crate::config::OutputConfig;
use crate::error::OutputError;
use std::process::Stdio;
use tokio::process::Command;

/// Trait for text output implementations
#[async_trait::async_trait]
pub trait TextOutput: Send + Sync {
    /// Output text (type it or copy to clipboard)
    async fn output(&self, text: &str) -> Result<(), OutputError>;

    /// Check if this output method is available
    async fn is_available(&self) -> bool;

    /// Human-readable name for logging
    fn name(&self) -> &'static str;
}

/// Factory function that returns a fallback chain of output methods
pub fn create_output_chain(config: &OutputConfig) -> Vec<Box<dyn TextOutput>> {
    let mut chain: Vec<Box<dyn TextOutput>> = Vec::new();

    match config.mode {
        crate::config::OutputMode::Type => {
            // Primary: wtype for Wayland (best Unicode/CJK support, no daemon)
            chain.push(Box::new(wtype::WtypeOutput::new(
                config.notification.on_transcription,
                config.auto_submit,
                config.wtype_delay_ms,
            )));

            // Fallback: ydotool (works on X11/TTY, requires daemon)
            chain.push(Box::new(ydotool::YdotoolOutput::new(
                config.type_delay_ms,
                false, // no notification, wtype handles it if available
                config.auto_submit,
            )));

            // Last resort: clipboard
            if config.fallback_to_clipboard {
                chain.push(Box::new(clipboard::ClipboardOutput::new(false)));
            }
        }
        crate::config::OutputMode::Clipboard => {
            // Only clipboard
            chain.push(Box::new(clipboard::ClipboardOutput::new(
                config.notification.on_transcription,
            )));
        }
        crate::config::OutputMode::Paste => {
            // Only paste mode (no fallback as requested)
            chain.push(Box::new(paste::PasteOutput::new(
                config.notification.on_transcription,
                config.auto_submit,
                config.paste_keys.clone(),
                config.type_delay_ms,
            )));
        }
    }

    chain
}

/// Run a shell command (for pre/post hooks)
pub async fn run_hook(command: &str, hook_name: &str) -> Result<(), String> {
    tracing::debug!("Running {} hook: {}", hook_name, command);

    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("{} hook failed to execute: {}", hook_name, e))?;

    if output.status.success() {
        tracing::info!("{} hook completed successfully", hook_name);
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("{} hook failed: {}", hook_name, stderr))
    }
}

/// Output configuration for the fallback chain
pub struct OutputOptions<'a> {
    pub pre_output_command: Option<&'a str>,
    pub post_output_command: Option<&'a str>,
}

/// Try each output method in the chain until one succeeds
/// Pre/post output commands are run before and after typing (for compositor integration).
pub async fn output_with_fallback(
    chain: &[Box<dyn TextOutput>],
    text: &str,
    options: OutputOptions<'_>,
) -> Result<(), OutputError> {
    // Run pre-output hook if configured (e.g., switch to modifier-suppressing submap)
    if let Some(cmd) = options.pre_output_command {
        if let Err(e) = run_hook(cmd, "pre_output").await {
            tracing::warn!("{}", e);
            // Continue anyway - best effort
        }
    }

    // Try each output method
    let mut result = Err(OutputError::AllMethodsFailed);
    for output in chain {
        if !output.is_available().await {
            tracing::debug!("{} not available, trying next", output.name());
            continue;
        }

        match output.output(text).await {
            Ok(()) => {
                tracing::debug!("Text output via {}", output.name());
                result = Ok(());
                break;
            }
            Err(e) => {
                tracing::warn!("{} failed: {}, trying next", output.name(), e);
            }
        }
    }

    // Run post-output hook if configured (e.g., reset submap)
    // Always run this, even on failure, to ensure cleanup
    if let Some(cmd) = options.post_output_command {
        if let Err(e) = run_hook(cmd, "post_output").await {
            tracing::warn!("{}", e);
        }
    }

    result
}
