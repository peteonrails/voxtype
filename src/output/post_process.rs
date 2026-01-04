//! Post-processing command execution
//!
//! Pipes transcribed text through an external command for cleanup/formatting.
//! Commonly used with local LLMs (Ollama, llama.cpp) or text processing tools.
//!
//! # Example Configuration
//!
//! ```toml
//! [output.post_process]
//! command = "ollama run llama3.2:1b 'Clean up this dictation:'"
//! timeout_ms = 30000
//! ```
//!
//! The command receives the transcribed text on stdin and should output
//! the processed text on stdout. On any failure, the original text is used.

use crate::config::PostProcessConfig;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

/// Post-processor that runs an external command on transcribed text
pub struct PostProcessor {
    command: String,
    timeout: Duration,
}

impl PostProcessor {
    /// Create a new post-processor from configuration
    pub fn new(config: &PostProcessConfig) -> Self {
        Self {
            command: config.command.clone(),
            timeout: Duration::from_millis(config.timeout_ms),
        }
    }

    /// Process text through the external command
    ///
    /// Returns the processed text on success, or the original text on any failure.
    /// This ensures voice-to-text always produces output even when post-processing fails.
    pub async fn process(&self, text: &str) -> String {
        match self.execute_command(text).await {
            Ok(processed) => {
                if processed.is_empty() {
                    tracing::warn!(
                        "Post-process command returned empty output, using original text"
                    );
                    text.to_string()
                } else {
                    tracing::debug!(
                        "Post-processed ({} -> {} chars)",
                        text.len(),
                        processed.len()
                    );
                    processed
                }
            }
            Err(e) => {
                tracing::warn!("Post-process command failed: {}, using original text", e);
                text.to_string()
            }
        }
    }

    async fn execute_command(&self, text: &str) -> Result<String, PostProcessError> {
        // Spawn command via shell for proper parsing of complex commands
        let mut child = Command::new("sh")
            .args(["-c", &self.command])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PostProcessError::SpawnFailed(e.to_string()))?;

        // Write text to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .await
                .map_err(|e| PostProcessError::WriteFailed(e.to_string()))?;
            // Close stdin to signal EOF
            drop(stdin);
        }

        // Wait for completion with timeout
        let output = timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| PostProcessError::Timeout(self.timeout.as_secs()))?
            .map_err(|e| PostProcessError::WaitFailed(e.to_string()))?;

        // Check exit status
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PostProcessError::NonZeroExit {
                code: output.status.code(),
                stderr: stderr.trim().to_string(),
            });
        }

        // Parse stdout as UTF-8
        let processed = String::from_utf8(output.stdout)
            .map_err(|e| PostProcessError::InvalidUtf8(e.to_string()))?;

        Ok(processed.trim().to_string())
    }
}

/// Errors that can occur during post-processing
#[derive(Debug)]
pub enum PostProcessError {
    /// Failed to spawn the command process
    SpawnFailed(String),
    /// Failed to write text to stdin
    WriteFailed(String),
    /// Command timed out
    Timeout(u64),
    /// Failed to wait for command completion
    WaitFailed(String),
    /// Command exited with non-zero status
    NonZeroExit { code: Option<i32>, stderr: String },
    /// Command output was not valid UTF-8
    InvalidUtf8(String),
}

impl std::fmt::Display for PostProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpawnFailed(e) => write!(f, "failed to spawn command: {}", e),
            Self::WriteFailed(e) => write!(f, "failed to write to stdin: {}", e),
            Self::Timeout(secs) => write!(f, "command timed out after {}s", secs),
            Self::WaitFailed(e) => write!(f, "failed to wait for command: {}", e),
            Self::NonZeroExit { code, stderr } => {
                if stderr.is_empty() {
                    write!(f, "command exited with code {:?}", code)
                } else {
                    write!(f, "command exited with code {:?}: {}", code, stderr)
                }
            }
            Self::InvalidUtf8(e) => write!(f, "output is not valid UTF-8: {}", e),
        }
    }
}

impl std::error::Error for PostProcessError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(command: &str, timeout_ms: u64) -> PostProcessConfig {
        PostProcessConfig {
            command: command.to_string(),
            timeout_ms,
        }
    }

    #[tokio::test]
    async fn test_simple_passthrough() {
        let config = make_config("cat", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("hello world").await;
        assert_eq!(result, "hello world");
    }

    #[tokio::test]
    async fn test_sed_transformation() {
        let config = make_config("sed 's/foo/bar/g'", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("foo bar foo").await;
        assert_eq!(result, "bar bar bar");
    }

    #[tokio::test]
    async fn test_tr_uppercase() {
        let config = make_config("tr '[:lower:]' '[:upper:]'", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("hello world").await;
        assert_eq!(result, "HELLO WORLD");
    }

    #[tokio::test]
    async fn test_timeout_fallback() {
        let config = make_config("sleep 10", 100); // 100ms timeout
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result, "original text"); // Falls back to original
    }

    #[tokio::test]
    async fn test_command_failure_fallback() {
        let config = make_config("exit 1", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result, "original text"); // Falls back to original
    }

    #[tokio::test]
    async fn test_empty_output_fallback() {
        // echo -n outputs nothing, which should trigger fallback
        let config = make_config("echo -n ''", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result, "original text"); // Falls back to original
    }

    #[tokio::test]
    async fn test_command_not_found_fallback() {
        let config = make_config("nonexistent_command_xyz_12345", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result, "original text"); // Falls back to original
    }

    #[tokio::test]
    async fn test_multiline_input() {
        let config = make_config("cat", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("line one\nline two\nline three").await;
        assert_eq!(result, "line one\nline two\nline three");
    }

    #[tokio::test]
    async fn test_unicode_handling() {
        let config = make_config("cat", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("Hello 世界! 🎉").await;
        assert_eq!(result, "Hello 世界! 🎉");
    }

    #[tokio::test]
    async fn test_whitespace_trimming() {
        // Output has trailing newline which should be trimmed
        let config = make_config("echo 'hello'", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("ignored").await;
        assert_eq!(result, "hello");
    }

    #[tokio::test]
    async fn test_complex_shell_command() {
        // Test that complex shell commands work (pipes, quotes, etc.)
        let config = make_config("echo 'prefix:' && cat", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("test input").await;
        assert_eq!(result, "prefix:\ntest input");
    }
}
