//! Anthropic API client for post-processing transcriptions
//!
//! Sends transcribed text to a Claude model (e.g., Haiku) for cleanup:
//! fixing transcription errors, removing filler words, improving conciseness.
//!
//! # Configuration
//!
//! ```toml
//! [output.post_process]
//! anthropic_api_key_file = ".env"  # or set ANTHROPIC_API_KEY env var
//! anthropic_model = "claude-haiku-4-5-20251001"
//! anthropic_prompt = "Clean up this voice dictation..."
//! timeout_ms = 10000
//! ```

use std::time::Duration;
use tokio::time::timeout;

/// Anthropic API client for transcription cleanup
pub struct AnthropicPostProcessor {
    api_key: String,
    model: String,
    prompt: String,
    timeout: Duration,
}

impl AnthropicPostProcessor {
    /// Create a new Anthropic post-processor
    pub fn new(api_key: String, model: String, prompt: Option<String>, timeout_ms: u64) -> Self {
        let prompt = prompt.unwrap_or_else(|| {
            "You are a voice transcription post-processor. Clean up the following \
             voice dictation: fix transcription errors, remove filler words (um, uh, like), \
             fix punctuation and capitalization, and make it concise. \
             Output ONLY the cleaned text with no preamble or explanation. \
             If the input is already clean, output it unchanged."
                .to_string()
        });

        Self {
            api_key,
            model,
            prompt,
            timeout: Duration::from_millis(timeout_ms),
        }
    }

    /// Process transcribed text through the Anthropic API
    ///
    /// Returns cleaned text on success, or the original text on any failure.
    pub async fn process(&self, text: &str) -> String {
        if text.trim().is_empty() {
            return text.to_string();
        }

        match timeout(self.timeout, self.call_api(text)).await {
            Ok(Ok(processed)) => {
                if processed.is_empty() {
                    tracing::warn!(
                        "Anthropic API returned empty response, using original text"
                    );
                    text.to_string()
                } else {
                    tracing::debug!(
                        "Anthropic post-processed ({} -> {} chars)",
                        text.len(),
                        processed.len()
                    );
                    processed
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("Anthropic API call failed: {}, using original text", e);
                text.to_string()
            }
            Err(_) => {
                tracing::warn!(
                    "Anthropic API timed out after {}ms, using original text",
                    self.timeout.as_millis()
                );
                text.to_string()
            }
        }
    }

    async fn call_api(&self, text: &str) -> Result<String, AnthropicError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 1024,
            "messages": [
                {
                    "role": "user",
                    "content": format!("{}\n\n{}", self.prompt, text)
                }
            ]
        });

        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| AnthropicError::Request(e.to_string()))?;

        // Use ureq in a blocking task to avoid blocking the async runtime
        let api_key = self.api_key.clone();
        let timeout_secs = self.timeout.as_secs().max(5);
        let response: Result<String, AnthropicError> =
            tokio::task::spawn_blocking(move || {
                let agent = ureq::AgentBuilder::new()
                    .timeout(std::time::Duration::from_secs(timeout_secs))
                    .build();
                let resp = agent
                    .post("https://api.anthropic.com/v1/messages")
                    .set("x-api-key", &api_key)
                    .set("anthropic-version", "2023-06-01")
                    .set("content-type", "application/json")
                    .send_bytes(&body_bytes)
                    .map_err(|e| AnthropicError::Request(e.to_string()))?;

                resp.into_string()
                    .map_err(|e| AnthropicError::Response(e.to_string()))
            })
            .await
            .map_err(|e| AnthropicError::Request(e.to_string()))?;

        let response_str = response?;
        let response_body: serde_json::Value = serde_json::from_str(&response_str)
            .map_err(|e| AnthropicError::Response(e.to_string()))?;

        // Extract text from response: .content[0].text
        let result = response_body
            .get("content")
            .and_then(|c: &serde_json::Value| c.as_array())
            .and_then(|arr: &Vec<serde_json::Value>| arr.first())
            .and_then(|block: &serde_json::Value| block.get("text"))
            .and_then(|t: &serde_json::Value| t.as_str())
            .ok_or_else(|| {
                let body_str = response_body.to_string();
                let truncated = if body_str.len() > 200 {
                    format!("{}...", &body_str[..200])
                } else {
                    body_str
                };
                AnthropicError::Response(format!(
                    "unexpected response structure: {}",
                    truncated
                ))
            })?;

        Ok(result.trim().to_string())
    }
}

/// Load API key from a .env file (looks for anthropic_api_key or ANTHROPIC_API_KEY)
pub fn load_api_key_from_env_file(path: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_lowercase();
            if key == "anthropic_api_key" {
                let value = value.trim().trim_matches('"').trim_matches('\'');
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Resolve API key from (in priority order):
/// 1. Explicit config value
/// 2. Environment variable ANTHROPIC_API_KEY
/// 3. Specified env file path
/// 4. .env in config directory (~/.config/voxtype/.env)
/// 5. .env in home directory (~/.env)
/// 6. .env in current directory
pub fn resolve_api_key(explicit: Option<&str>, env_file: Option<&str>) -> Option<String> {
    // 1. Explicit value
    if let Some(key) = explicit {
        if !key.is_empty() {
            return Some(key.to_string());
        }
    }

    // 2. Environment variable
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }

    // 3. Specified env file
    if let Some(path) = env_file {
        if let Some(key) = load_api_key_from_env_file(path) {
            return Some(key);
        }
    }

    // 4. .env in config directory
    if let Some(config_dir) = directories::ProjectDirs::from("", "", "voxtype") {
        let env_path = config_dir.config_dir().join(".env");
        if let Some(key) = load_api_key_from_env_file(&env_path.to_string_lossy()) {
            return Some(key);
        }
    }

    // 5. .env in home directory
    if let Ok(home) = std::env::var("HOME") {
        let env_path = format!("{}/.env", home);
        if let Some(key) = load_api_key_from_env_file(&env_path) {
            return Some(key);
        }
    }

    // 6. .env in current directory
    if let Some(key) = load_api_key_from_env_file(".env") {
        return Some(key);
    }

    None
}

#[derive(Debug)]
pub enum AnthropicError {
    Request(String),
    Response(String),
}

impl std::fmt::Display for AnthropicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(e) => write!(f, "API request failed: {}", e),
            Self::Response(e) => write!(f, "API response error: {}", e),
        }
    }
}

impl std::error::Error for AnthropicError {}
