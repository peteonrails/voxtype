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
            )));

            // Fallback: ydotool (works on X11/TTY, requires daemon)
            chain.push(Box::new(ydotool::YdotoolOutput::new(
                config.type_delay_ms,
                false, // no notification, wtype handles it if available
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
            )));
        }
    }

    chain
}

/// Try each output method in the chain until one succeeds
pub async fn output_with_fallback(
    chain: &[Box<dyn TextOutput>],
    text: &str,
) -> Result<(), OutputError> {
    for output in chain {
        if !output.is_available().await {
            tracing::debug!("{} not available, trying next", output.name());
            continue;
        }

        match output.output(text).await {
            Ok(()) => {
                tracing::debug!("Text output via {}", output.name());
                return Ok(());
            }
            Err(e) => {
                tracing::warn!("{} failed: {}, trying next", output.name(), e);
            }
        }
    }

    Err(OutputError::AllMethodsFailed)
}
