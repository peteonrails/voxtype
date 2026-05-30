//! Profile and post-process configuration.

use serde::{Deserialize, Serialize};

use super::default_true;
use super::OutputMode;

/// Post-processing command configuration
///
/// Pipes transcribed text through an external command for cleanup/formatting.
/// Commonly used with local LLMs (Ollama, llama.cpp) or text processing tools.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PostProcessConfig {
    /// Shell command to execute
    /// Receives transcribed text on stdin, outputs processed text on stdout
    pub command: String,

    /// Timeout in milliseconds (default: 30000 = 30 seconds)
    #[serde(default = "default_post_process_timeout")]
    pub timeout_ms: u64,

    /// Whether to trim leading/trailing whitespace from command output (default: true)
    /// Set to false when the command intentionally produces significant whitespace,
    /// e.g. a trailing space after sentence-ending punctuation for dictation flow.
    #[serde(default = "default_true")]
    pub trim: bool,

    /// Whether to fall back to original text when command output is empty (default: true)
    /// Set to false when the command intentionally produces empty output,
    /// e.g. filtering out unwanted transcriptions like [BLANK_AUDIO].
    #[serde(default = "default_true")]
    pub fallback_on_empty: bool,
}

/// Named profile for context-specific settings
///
/// Profiles allow different post-processing commands (and other settings)
/// for different contexts like Slack, code editors, email, etc.
///
/// # Example Configuration
///
/// ```toml
/// [profiles.slack]
/// post_process_command = "cleanup-for-slack.sh"
///
/// [profiles.code]
/// post_process_command = "cleanup-for-code.sh"
/// ```
///
/// Use with: `voxtype record start --profile slack`
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Profile {
    /// Post-processing command for this profile
    /// Overrides [output.post_process.command] when the profile is active
    #[serde(default)]
    pub post_process_command: Option<String>,

    /// Timeout for post-processing in milliseconds (default: 30000)
    #[serde(default)]
    pub post_process_timeout_ms: Option<u64>,

    /// Output mode override for this profile
    #[serde(default)]
    pub output_mode: Option<OutputMode>,
}

fn default_post_process_timeout() -> u64 {
    30000 // 30 seconds - generous for LLM processing
}
