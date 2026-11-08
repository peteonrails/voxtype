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
//! the processed text on stdout. On any failure, the original text is used,
//! and the user gets a desktop notice saying cleanup did not run (see
//! [`crate::output::deferred_notice`]).

use crate::config::PostProcessConfig;
use crate::output::deferred_notice::{self, Notice};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

/// Post-processor that runs an external command on transcribed text
pub struct PostProcessor {
    command: String,
    timeout: Duration,
    trim: bool,
    fallback_on_empty: bool,
    origin: Origin,
}

/// Where the command came from, so a notice names the setting to change.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Origin {
    /// `[output.post_process]`
    Output,
    /// `[profiles.<name>]`, whose keys are `post_process_command` and
    /// `post_process_timeout_ms`.
    Profile(String),
}

impl Origin {
    fn command_setting(&self) -> String {
        match self {
            Origin::Output => "`command` in [output.post_process]".to_string(),
            Origin::Profile(name) => format!("`post_process_command` in [profiles.{name}]"),
        }
    }

    /// Key prefix for the notice rate limiter. Per origin, so a working
    /// default command doesn't re-arm the notice of a broken profile one.
    fn notice_source(&self) -> String {
        match self {
            Origin::Output => "post_process:output:".to_string(),
            Origin::Profile(name) => format!("post_process:profile:{name}:"),
        }
    }

    fn timeout_fix(&self) -> String {
        match self {
            Origin::Output => "Raise `timeout_ms` in [output.post_process] \
                 (`voxtype config set output.post_process.timeout_ms 60000`)"
                .to_string(),
            Origin::Profile(name) => {
                format!("Raise `post_process_timeout_ms` in [profiles.{name}]")
            }
        }
    }
}

const USED_AS_IS: &str = "The text was used as transcribed, without cleanup.";

impl PostProcessor {
    /// Create a new post-processor from configuration
    pub fn new(config: &PostProcessConfig) -> Self {
        Self {
            command: config.command.clone(),
            timeout: Duration::from_millis(config.timeout_ms),
            trim: config.trim,
            fallback_on_empty: config.fallback_on_empty,
            origin: Origin::Output,
        }
    }

    /// Mark this processor as running a profile's `post_process_command`, so
    /// failure notices point at the profile's settings.
    pub fn for_profile(mut self, name: &str) -> Self {
        self.origin = Origin::Profile(name.to_string());
        self
    }

    fn report(&self, kind: &str, notice: Notice) {
        deferred_notice::report(&format!("{}{kind}", self.origin.notice_source()), notice);
    }

    /// The notice for a failed run, with the failure kind it is rate-limited by.
    fn failure_notice(&self, err: &PostProcessError) -> (&'static str, Notice) {
        let fix_command = format!(
            "Run the command by hand to see why, or fix {}.",
            self.origin.command_setting()
        );
        match err {
            // `sh -c` reports a missing program as exit 127, and one it
            // cannot execute as 126.
            PostProcessError::NonZeroExit {
                code: Some(code @ (126 | 127)),
                stderr,
            } => {
                let what = if stderr.is_empty() {
                    format!(
                        "`{}` was not found (exit {code})",
                        deferred_notice::detail(&self.command)
                    )
                } else {
                    deferred_notice::detail(stderr)
                };
                (
                    "not_found",
                    Notice {
                        title: "Voxtype: post-processing command not found".to_string(),
                        body: format!(
                            "{what}. {USED_AS_IS} Install the program or fix {}.",
                            self.origin.command_setting()
                        ),
                    },
                )
            }
            PostProcessError::NonZeroExit { code, stderr } => {
                let status = match code {
                    Some(code) => format!("exited with code {code}"),
                    None => "was killed by a signal".to_string(),
                };
                let reason = if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {}", deferred_notice::detail(stderr))
                };
                (
                    "exit",
                    Notice {
                        title: "Voxtype: post-processing failed".to_string(),
                        body: format!("The command {status}{reason}. {USED_AS_IS} {fix_command}"),
                    },
                )
            }
            PostProcessError::Timeout(ms) => (
                "timeout",
                Notice {
                    title: "Voxtype: post-processing timed out".to_string(),
                    body: format!(
                        "The command did not finish within {ms} ms. {USED_AS_IS} {} \
                         or use a faster command or model.",
                        self.origin.timeout_fix()
                    ),
                },
            ),
            other => (
                "other",
                Notice {
                    title: "Voxtype: post-processing failed".to_string(),
                    body: format!(
                        "{}. {USED_AS_IS} {fix_command}",
                        deferred_notice::detail(&other.to_string())
                    ),
                },
            ),
        }
    }

    fn empty_output_notice(&self) -> Notice {
        let intended = match self.origin {
            Origin::Output => {
                " If empty output is intended, set `fallback_on_empty = false` \
                 in [output.post_process]."
            }
            Origin::Profile(_) => "",
        };
        Notice {
            title: "Voxtype: post-processing returned nothing".to_string(),
            body: format!(
                "The command printed nothing on stdout. {USED_AS_IS} \
                 Check that it writes the cleaned text to stdout.{intended}"
            ),
        }
    }

    /// Process text with optional context from a previous chunk
    ///
    /// When context is provided, it is passed via the VOXTYPE_CONTEXT environment
    /// variable so the post-processing command can use it for continuity.
    /// Stdin always contains only the current text, keeping existing scripts compatible.
    /// Returns the processed text on success, or the original text on any failure.
    pub async fn process_with_context(&self, text: &str, context: Option<&str>) -> String {
        match self.execute_command_with_env(text, context).await {
            Ok(processed) => {
                if processed.is_empty() && self.fallback_on_empty {
                    tracing::warn!(
                        "Post-process command returned empty output, using original text"
                    );
                    self.report("empty", self.empty_output_notice());
                    text.to_string()
                } else if processed.is_empty() {
                    tracing::debug!("Post-process command returned empty output");
                    deferred_notice::succeeded(&self.origin.notice_source());
                    String::new()
                } else {
                    deferred_notice::succeeded(&self.origin.notice_source());
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
                let (kind, notice) = self.failure_notice(&e);
                self.report(kind, notice);
                text.to_string()
            }
        }
    }

    /// Process text through the external command
    ///
    /// Returns the processed text on success, or the original text on any failure.
    /// This ensures voice-to-text always produces output even when post-processing fails.
    pub async fn process(&self, text: &str) -> String {
        self.process_with_context(text, None).await
    }

    async fn execute_command_with_env(
        &self,
        text: &str,
        context: Option<&str>,
    ) -> Result<String, PostProcessError> {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &self.command])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Always clear to prevent inheriting stale context from parent environment
        cmd.env_remove("VOXTYPE_CONTEXT");
        if let Some(ctx) = context {
            cmd.env("VOXTYPE_CONTEXT", ctx);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| PostProcessError::SpawnFailed(e.to_string()))?;

        // Write text to stdin
        if let Some(mut stdin) = child.stdin.take() {
            // Ignore write errors: the command may not read stdin or may exit
            // before we finish writing (e.g., `echo` or `head -1`). The command's
            // exit code and stdout output determine success, not whether it
            // consumed all of stdin.
            let _ = stdin.write_all(text.as_bytes()).await;
            drop(stdin);
        }

        // Wait for completion with timeout
        let output = timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| PostProcessError::Timeout(self.timeout.as_millis() as u64))?
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

        if self.trim {
            Ok(processed.trim().to_string())
        } else {
            // Only strip trailing newlines (artifact of shell output), preserve other whitespace
            Ok(processed.trim_end_matches('\n').to_string())
        }
    }
}

/// Errors that can occur during post-processing
#[derive(Debug)]
pub enum PostProcessError {
    /// Failed to spawn the command process
    SpawnFailed(String),
    /// Failed to write text to stdin
    WriteFailed(String),
    /// Command timed out (the timeout, in milliseconds)
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
            Self::Timeout(ms) => write!(f, "command timed out after {}ms", ms),
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
            trim: true,
            fallback_on_empty: true,
        }
    }

    #[test]
    fn missing_command_notice_says_not_found_and_names_the_setting() {
        let p = PostProcessor::new(&make_config("ollama run x", 5000));
        let (kind, n) = p.failure_notice(&PostProcessError::NonZeroExit {
            code: Some(127),
            stderr: "sh: line 1: ollama: command not found".into(),
        });
        assert_eq!(kind, "not_found");
        assert_eq!(n.title, "Voxtype: post-processing command not found");
        assert!(n.body.starts_with("sh: line 1: ollama: command not found."));
        assert!(n.body.contains("without cleanup"));
        assert!(n.body.contains("`command` in [output.post_process]"));
    }

    #[test]
    fn non_zero_exit_notice_quotes_first_stderr_line() {
        let p = PostProcessor::new(&make_config("cleanup.sh", 5000));
        let (kind, n) = p.failure_notice(&PostProcessError::NonZeroExit {
            code: Some(2),
            stderr: "model not loaded\nstack trace".into(),
        });
        assert_eq!(kind, "exit");
        assert!(n
            .body
            .starts_with("The command exited with code 2: model not loaded."));
        assert!(!n.body.contains("stack trace"));
    }

    #[test]
    fn timeout_notice_gives_the_timeout_and_how_to_raise_it() {
        let p = PostProcessor::new(&make_config("cleanup.sh", 5000));
        let (kind, n) = p.failure_notice(&PostProcessError::Timeout(5000));
        assert_eq!(kind, "timeout");
        assert!(n.body.contains("within 5000 ms"));
        assert!(n
            .body
            .contains("voxtype config set output.post_process.timeout_ms 60000"));
    }

    #[test]
    fn profile_notices_point_at_the_profile_keys() {
        let p = PostProcessor::new(&make_config("cleanup.sh", 5000)).for_profile("slack");
        let (_, n) = p.failure_notice(&PostProcessError::Timeout(5000));
        assert!(n
            .body
            .contains("`post_process_timeout_ms` in [profiles.slack]"));
        let (_, n) = p.failure_notice(&PostProcessError::NonZeroExit {
            code: Some(1),
            stderr: String::new(),
        });
        assert!(n
            .body
            .contains("`post_process_command` in [profiles.slack]"));
        assert!(!p.empty_output_notice().body.contains("fallback_on_empty"));
    }

    #[test]
    fn empty_output_notice_mentions_fallback_on_empty() {
        let p = PostProcessor::new(&make_config("true", 5000));
        let n = p.empty_output_notice();
        assert_eq!(n.title, "Voxtype: post-processing returned nothing");
        assert!(n.body.contains("fallback_on_empty = false"));
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
        // printf '' outputs nothing, which should trigger fallback
        // (echo -n is not portable across platforms)
        let config = make_config("printf ''", 5000);
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
        // Use printf with \n to be portable across platforms
        let config = make_config("printf 'hello\\n'", 5000);
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

    #[tokio::test]
    async fn test_no_trim_preserves_trailing_space() {
        // When trim = false, trailing spaces from the command should be preserved
        let config = PostProcessConfig {
            command: "printf '%s ' \"$( cat )\"".to_string(),
            timeout_ms: 5000,
            trim: false,
            fallback_on_empty: true,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("hello world.").await;
        assert_eq!(result, "hello world. ");
    }

    #[tokio::test]
    async fn test_no_trim_still_strips_trailing_newlines() {
        // Even with trim = false, trailing newlines (shell artifacts) are stripped
        let config = PostProcessConfig {
            command: "echo 'hello'".to_string(),
            timeout_ms: 5000,
            trim: false,
            fallback_on_empty: true,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("ignored").await;
        assert_eq!(result, "hello");
    }

    #[tokio::test]
    async fn test_no_fallback_on_empty_returns_empty() {
        // When fallback_on_empty = false, empty output is returned as-is
        let config = PostProcessConfig {
            command: "printf ''".to_string(),
            timeout_ms: 5000,
            trim: true,
            fallback_on_empty: false,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result, "");
    }

    #[tokio::test]
    async fn test_fallback_on_empty_default_returns_original() {
        // Default behavior: empty output falls back to original text
        let config = make_config("printf ''", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result, "original text");
    }

    #[tokio::test]
    async fn test_no_trim_no_fallback_combination() {
        // Both options off: whatever the command emits is returned verbatim,
        // empty included.
        let config = PostProcessConfig {
            command: "printf ''".to_string(),
            timeout_ms: 5000,
            trim: false,
            fallback_on_empty: false,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result, "");
    }

    #[tokio::test]
    async fn test_trim_then_empty_triggers_fallback() {
        // Whitespace-only output should be considered empty after trimming,
        // and trigger the fallback when fallback_on_empty is on.
        let config = PostProcessConfig {
            command: "printf '   \\n  '".to_string(),
            timeout_ms: 5000,
            trim: true,
            fallback_on_empty: true,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result, "original text");
    }

    #[tokio::test]
    async fn test_trim_then_empty_no_fallback_returns_empty() {
        // Same scenario but fallback off: empty string surfaces.
        let config = PostProcessConfig {
            command: "printf '   \\n  '".to_string(),
            timeout_ms: 5000,
            trim: true,
            fallback_on_empty: false,
        };
        let processor = PostProcessor::new(&config);
        let result = processor.process("original text").await;
        assert_eq!(result, "");
    }

    #[tokio::test]
    async fn test_context_passed_via_env_var() {
        // Command prints VOXTYPE_CONTEXT env var, stdin is current text
        let config = make_config("echo \"context:$VOXTYPE_CONTEXT stdin:$(cat)\"", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor
            .process_with_context("current text", Some("previous text"))
            .await;
        assert_eq!(result, "context:previous text stdin:current text");
    }

    #[tokio::test]
    async fn test_no_context_env_var_when_none() {
        // VOXTYPE_CONTEXT should not be set when context is None
        let config = make_config(
            "echo \"context:${VOXTYPE_CONTEXT:-unset} stdin:$(cat)\"",
            5000,
        );
        let processor = PostProcessor::new(&config);
        let result = processor.process_with_context("current text", None).await;
        assert_eq!(result, "context:unset stdin:current text");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_context_env_not_inherited_from_parent() {
        // Even if VOXTYPE_CONTEXT is set in parent env, it should be cleared when context is None.
        // Uses current_thread runtime because std::env::set_var is not thread-safe
        // and will become unsafe in Rust edition 2024.
        std::env::set_var("VOXTYPE_CONTEXT", "stale parent context");
        let config = make_config("echo \"${VOXTYPE_CONTEXT:-unset}\"", 5000);
        let processor = PostProcessor::new(&config);
        let result = processor.process_with_context("text", None).await;
        std::env::remove_var("VOXTYPE_CONTEXT");
        assert_eq!(result, "unset");
    }
}
