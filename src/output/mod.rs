//! Text output module
//!
//! Provides text output via keyboard simulation or clipboard.
//!
//! Fallback chain for `mode = "type"`:
//! 1. wtype - Wayland-native via virtual-keyboard protocol, best Unicode/CJK support, no daemon needed
//! 2. eitype - Wayland via libei/EI protocol, works on GNOME/KDE (no virtual-keyboard support)
//! 3. dotool - Works on X11/Wayland/TTY, supports keyboard layouts, no daemon needed
//! 4. ydotool - Works on X11/Wayland/TTY, requires daemon
//! 5. clipboard (wl-copy) - Wayland clipboard fallback
//! 6. xclip - X11 clipboard fallback
//!
//! Paste mode (clipboard + Ctrl+V) helps with system with non US keyboard layouts.

pub mod clipboard;
pub mod dotool;
pub mod eitype;
pub mod paste;
pub mod post_process;
pub mod wtype;
pub mod xclip;
pub mod ydotool;

use crate::config::{OutputConfig, OutputDriver};
use crate::error::OutputError;
use std::borrow::Cow;
use std::fs;
use std::process::Stdio;
use tokio::process::Command;

/// Normalize Unicode curly quotes to ASCII equivalents.
///
/// Whisper sometimes outputs curly/smart quotes which can cause issues with
/// keyboard simulation tools (wtype, dotool, ydotool). This function converts
/// them to standard ASCII quotes to prevent unexpected line breaks or other
/// typing artifacts.
fn normalize_quotes(text: &str) -> Cow<'_, str> {
    // Quick check to avoid allocation if no normalization needed
    let needs_normalization = text.chars().any(|c| {
        matches!(
            c,
            '\u{2018}'  // LEFT SINGLE QUOTATION MARK
            | '\u{2019}'  // RIGHT SINGLE QUOTATION MARK (curly apostrophe)
            | '\u{201B}'  // SINGLE HIGH-REVERSED-9 QUOTATION MARK
            | '\u{2032}'  // PRIME
            | '\u{201C}'  // LEFT DOUBLE QUOTATION MARK
            | '\u{201D}'  // RIGHT DOUBLE QUOTATION MARK
            | '\u{201F}'  // DOUBLE HIGH-REVERSED-9 QUOTATION MARK
            | '\u{2033}' // DOUBLE PRIME
        )
    });

    if !needs_normalization {
        return Cow::Borrowed(text);
    }

    Cow::Owned(
        text.chars()
            .map(|c| match c {
                // Single quotes/apostrophes -> ASCII apostrophe
                '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' => '\'',
                // Double quotes -> ASCII double quote
                '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' => '"',
                other => other,
            })
            .collect(),
    )
}

/// Path to the voxtype symlink
const VOXTYPE_BIN: &str = "/usr/lib/voxtype/voxtype";

/// Check if the active binary is a Parakeet build
pub fn is_parakeet_binary_active() -> bool {
    if let Ok(link_target) = fs::read_link(VOXTYPE_BIN) {
        if let Some(target_name) = link_target.file_name() {
            if let Some(name) = target_name.to_str() {
                return name.contains("parakeet");
            }
        }
    }
    // If we can't read the symlink, check if parakeet feature is enabled
    #[cfg(feature = "parakeet")]
    {
        return true;
    }
    #[cfg(not(feature = "parakeet"))]
    {
        false
    }
}

/// Get the engine icon for notifications based on configured engine
/// Returns 🦜 for Parakeet, 🗣️ for Whisper
pub fn engine_icon(engine: crate::config::TranscriptionEngine) -> &'static str {
    match engine {
        crate::config::TranscriptionEngine::Parakeet => "🦜",
        crate::config::TranscriptionEngine::Whisper => "🗣️",
    }
}

/// Send a transcription notification with optional engine icon
pub async fn send_transcription_notification(
    text: &str,
    show_engine_icon: bool,
    engine: crate::config::TranscriptionEngine,
) {
    // Truncate preview for notification (use chars() to handle multi-byte UTF-8)
    let preview = if text.chars().count() > 80 {
        format!("{}...", text.chars().take(80).collect::<String>())
    } else {
        text.to_string()
    };

    let title = if show_engine_icon {
        format!("{} Transcribed", engine_icon(engine))
    } else {
        "Transcribed".to_string()
    };

    let _ = Command::new("notify-send")
        .args([
            "--app-name=Voxtype",
            "--urgency=low",
            "--expire-time=3000",
            &title,
            &preview,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

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

/// Default driver order for type mode
const DEFAULT_DRIVER_ORDER: &[OutputDriver] = &[
    OutputDriver::Wtype,
    OutputDriver::Eitype,
    OutputDriver::Dotool,
    OutputDriver::Ydotool,
    OutputDriver::Clipboard,
    OutputDriver::Xclip,
];

/// Create a TextOutput implementation for a specific driver
fn create_driver_output(
    driver: OutputDriver,
    config: &OutputConfig,
    pre_type_delay_ms: u32,
    is_first: bool,
) -> Box<dyn TextOutput> {
    // Only the first driver in the chain should show notifications
    let show_notification = is_first && config.notification.on_transcription;

    match driver {
        OutputDriver::Wtype => Box::new(wtype::WtypeOutput::new(
            config.auto_submit,
            config.type_delay_ms,
            pre_type_delay_ms,
            config.shift_enter_newlines,
        )),
        OutputDriver::Eitype => Box::new(eitype::EitypeOutput::new(
            config.auto_submit,
            config.type_delay_ms,
            pre_type_delay_ms,
            config.shift_enter_newlines,
        )),
        OutputDriver::Dotool => Box::new(dotool::DotoolOutput::new(
            config.type_delay_ms,
            pre_type_delay_ms,
            show_notification,
            config.auto_submit,
            config.dotool_xkb_layout.clone(),
            config.dotool_xkb_variant.clone(),
        )),
        OutputDriver::Ydotool => Box::new(ydotool::YdotoolOutput::new(
            config.type_delay_ms,
            pre_type_delay_ms,
            show_notification,
            config.auto_submit,
        )),
        OutputDriver::Clipboard => Box::new(clipboard::ClipboardOutput::new(show_notification)),
        OutputDriver::Xclip => Box::new(xclip::XclipOutput::new(show_notification)),
    }
}

/// Factory function that returns a fallback chain of output methods
pub fn create_output_chain(config: &OutputConfig) -> Vec<Box<dyn TextOutput>> {
    create_output_chain_with_override(config, None)
}

/// Factory function that returns a fallback chain of output methods with an optional driver override
pub fn create_output_chain_with_override(
    config: &OutputConfig,
    driver_override: Option<&[OutputDriver]>,
) -> Vec<Box<dyn TextOutput>> {
    let mut chain: Vec<Box<dyn TextOutput>> = Vec::new();

    // Get effective pre_type_delay_ms (handles deprecated wtype_delay_ms)
    let pre_type_delay_ms = config.effective_pre_type_delay_ms();

    match config.mode {
        crate::config::OutputMode::Type => {
            // Determine driver order: CLI override > config > default
            let driver_order: &[OutputDriver] = driver_override
                .or(config.driver_order.as_deref())
                .unwrap_or(DEFAULT_DRIVER_ORDER);

            if let Some(custom_order) = driver_override.or(config.driver_order.as_deref()) {
                tracing::info!(
                    "Using custom driver order: {}",
                    custom_order
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(" -> ")
                );
            }

            // Build chain based on driver order
            for (i, driver) in driver_order.iter().enumerate() {
                // Skip clipboard if it's in the middle and fallback_to_clipboard is false
                // (clipboard should only be added if explicitly in the order OR fallback is enabled and it's last)
                let is_last = i == driver_order.len() - 1;
                if *driver == OutputDriver::Clipboard && !is_last && !config.fallback_to_clipboard {
                    continue;
                }

                chain.push(create_driver_output(
                    *driver,
                    config,
                    pre_type_delay_ms,
                    i == 0,
                ));
            }

            // If fallback_to_clipboard is true but clipboard wasn't in the custom order, add it
            if config.fallback_to_clipboard
                && config.driver_order.is_some()
                && !driver_order.contains(&OutputDriver::Clipboard)
            {
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
                config.auto_submit,
                config.paste_keys.clone(),
                config.type_delay_ms,
                pre_type_delay_ms,
            )));
        }
        crate::config::OutputMode::File => {
            // File output is handled in the daemon before reaching the output chain.
            // If we get here, it means mode = "file" but no file_path is configured.
            tracing::warn!(
                "Output mode is 'file' but no file_path configured. Falling back to clipboard."
            );
            chain.push(Box::new(clipboard::ClipboardOutput::new(
                config.notification.on_transcription,
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
    // Normalize curly quotes to ASCII to prevent line break issues with keyboard tools
    let normalized_text = normalize_quotes(text);

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

        match output.output(&normalized_text).await {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_quotes_no_change() {
        let text = "Hello, world! It's a test.";
        let result = normalize_quotes(text);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, text);
    }

    #[test]
    fn test_normalize_quotes_curly_apostrophe() {
        let text = "It\u{2019}s a test";
        let result = normalize_quotes(text);
        assert!(matches!(result, Cow::Owned(_)));
        assert_eq!(result, "It's a test");
    }

    #[test]
    fn test_normalize_quotes_all_single() {
        let text = "\u{2018}hello\u{2019} \u{201B}world\u{2032}";
        let result = normalize_quotes(text);
        assert_eq!(result, "'hello' 'world'");
    }

    #[test]
    fn test_normalize_quotes_all_double() {
        let text = "\u{201C}hello\u{201D} \u{201F}world\u{2033}";
        let result = normalize_quotes(text);
        assert_eq!(result, "\"hello\" \"world\"");
    }

    #[test]
    fn test_normalize_quotes_mixed() {
        let text = "\u{201C}Don\u{2019}t worry,\u{201D} she said.";
        let result = normalize_quotes(text);
        assert_eq!(result, "\"Don't worry,\" she said.");
    }

    #[test]
    fn test_normalize_quotes_empty() {
        let text = "";
        let result = normalize_quotes(text);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "");
    }

    #[test]
    fn test_normalize_quotes_unicode_preserved() {
        let text = "Café \u{2019} emoji 😀";
        let result = normalize_quotes(text);
        assert_eq!(result, "Café ' emoji 😀");
    }
}
