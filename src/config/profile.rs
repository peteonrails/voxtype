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

#[cfg(test)]
mod tests {
    use crate::config::{Config, OutputMode};

    #[test]
    fn test_profiles_default_empty() {
        let config = Config::default();
        assert!(config.profiles.is_empty());
        assert!(config.profile_names().is_empty());
        assert!(config.get_profile("slack").is_none());
    }

    #[test]
    fn test_parse_profiles_from_toml() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [profiles.slack]
            post_process_command = "cleanup-for-slack.sh"

            [profiles.code]
            post_process_command = "cleanup-for-code.sh"
            output_mode = "clipboard"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.profiles.len(), 2);

        let slack = config.get_profile("slack").unwrap();
        assert_eq!(
            slack.post_process_command,
            Some("cleanup-for-slack.sh".to_string())
        );
        assert!(slack.output_mode.is_none());

        let code = config.get_profile("code").unwrap();
        assert_eq!(
            code.post_process_command,
            Some("cleanup-for-code.sh".to_string())
        );
        assert_eq!(code.output_mode, Some(OutputMode::Clipboard));
    }

    #[test]
    fn test_parse_profile_with_timeout() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [profiles.slow]
            post_process_command = "slow-llm-command"
            post_process_timeout_ms = 60000
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let slow = config.get_profile("slow").unwrap();
        assert_eq!(
            slow.post_process_command,
            Some("slow-llm-command".to_string())
        );
        assert_eq!(slow.post_process_timeout_ms, Some(60000));
    }

    #[test]
    fn test_profile_names() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [profiles.alpha]
            post_process_command = "alpha-cmd"

            [profiles.beta]
            post_process_command = "beta-cmd"

            [profiles.gamma]
            post_process_command = "gamma-cmd"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let names: Vec<&str> = config.profile_names().iter().map(|s| s.as_str()).collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
        assert!(names.contains(&"gamma"));
    }

    #[test]
    fn test_profile_without_post_process_command() {
        // A profile can have only output_mode override without post_process_command
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [profiles.clipboard_only]
            output_mode = "clipboard"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let profile = config.get_profile("clipboard_only").unwrap();
        assert!(profile.post_process_command.is_none());
        assert_eq!(profile.output_mode, Some(OutputMode::Clipboard));
    }

    #[test]
    fn test_config_without_profiles_section() {
        // Config without [profiles] section should work (backwards compatibility)
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.profiles.is_empty());
    }
}
