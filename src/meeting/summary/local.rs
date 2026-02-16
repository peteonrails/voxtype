//! Local LLM summarization using Ollama
//!
//! Integrates with a locally running Ollama instance for meeting summarization.
//! Requires Ollama to be installed and running.

use super::{generate_prompt, parse_summary_response, Summarizer, SummaryConfig, SummaryError};
use crate::meeting::data::{MeetingData, MeetingSummary};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Ollama-based summarizer
pub struct OllamaSummarizer {
    /// Ollama API endpoint
    url: String,
    /// Model name
    model: String,
    /// Request timeout
    timeout: Duration,
}

impl OllamaSummarizer {
    /// Create a new Ollama summarizer
    pub fn new(config: &SummaryConfig) -> Self {
        Self {
            url: config.ollama_url.clone(),
            model: config.ollama_model.clone(),
            timeout: Duration::from_secs(config.timeout_secs),
        }
    }

    /// Check if Ollama is running and the model is available
    pub fn check_availability(&self) -> Result<(), SummaryError> {
        let client = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .build();

        // Check Ollama is running
        let tags_url = format!("{}/api/tags", self.url);
        let response = client
            .get(&tags_url)
            .call()
            .map_err(|e| SummaryError::OllamaUnavailable(format!("{}: {}", self.url, e)))?;

        // Parse available models
        #[derive(Deserialize)]
        struct TagsResponse {
            models: Option<Vec<ModelInfo>>,
        }

        #[derive(Deserialize)]
        struct ModelInfo {
            name: String,
        }

        let tags: TagsResponse = response
            .into_json()
            .map_err(|e| SummaryError::Parse(format!("Failed to parse tags response: {}", e)))?;

        // Check if our model is available
        let models = tags.models.unwrap_or_default();
        let model_base = self.model.split(':').next().unwrap_or(&self.model);

        let model_available = models.iter().any(|m| {
            let m_base = m.name.split(':').next().unwrap_or(&m.name);
            m_base == model_base || m.name == self.model
        });

        if !model_available {
            tracing::warn!(
                "Model '{}' not found in Ollama. Available models: {:?}",
                self.model,
                models.iter().map(|m| &m.name).collect::<Vec<_>>()
            );
            // Don't fail - Ollama might pull the model on first use
        }

        Ok(())
    }

    /// Call Ollama generate API
    fn generate(&self, prompt: &str) -> Result<String, SummaryError> {
        let client = ureq::AgentBuilder::new().timeout(self.timeout).build();

        let generate_url = format!("{}/api/generate", self.url);

        #[derive(Serialize)]
        struct GenerateRequest<'a> {
            model: &'a str,
            prompt: &'a str,
            stream: bool,
            format: &'a str,
        }

        let request = GenerateRequest {
            model: &self.model,
            prompt,
            stream: false,
            format: "json",
        };

        tracing::debug!("Calling Ollama generate API with model: {}", self.model);

        let response = client
            .post(&generate_url)
            .send_json(&request)
            .map_err(|e| match e {
                ureq::Error::Transport(ref t) => {
                    let msg = t.to_string();
                    if msg.contains("timed out") || msg.contains("timeout") {
                        SummaryError::Request("Request timed out - try a shorter transcript".into())
                    } else if msg.contains("connection") {
                        SummaryError::OllamaUnavailable(format!("{}: connection failed", self.url))
                    } else {
                        SummaryError::Request(e.to_string())
                    }
                }
                _ => SummaryError::Request(e.to_string()),
            })?;

        #[derive(Deserialize)]
        struct GenerateResponse {
            response: String,
            #[allow(dead_code)]
            done: bool,
        }

        let gen_response: GenerateResponse = response.into_json().map_err(|e| {
            SummaryError::Parse(format!("Failed to parse generate response: {}", e))
        })?;

        Ok(gen_response.response)
    }
}

impl Summarizer for OllamaSummarizer {
    fn summarize(&self, meeting: &MeetingData) -> Result<MeetingSummary, SummaryError> {
        // Check transcript is not empty
        if meeting.transcript.segments.is_empty() {
            return Err(SummaryError::EmptyTranscript);
        }

        // Generate prompt
        let prompt = generate_prompt(meeting);
        tracing::debug!(
            "Generated summarization prompt ({} chars, {} segments)",
            prompt.len(),
            meeting.transcript.segments.len()
        );

        // Call Ollama
        let response = self.generate(&prompt)?;
        tracing::debug!("Received response ({} chars)", response.len());

        // Parse response
        let summary = parse_summary_response(&response, Some(self.model.clone()))?;

        Ok(summary)
    }

    fn name(&self) -> &'static str {
        "ollama"
    }

    fn is_available(&self) -> bool {
        self.check_availability().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_from_config() {
        let config = SummaryConfig {
            ollama_url: "http://test:11434".to_string(),
            ollama_model: "mistral".to_string(),
            timeout_secs: 60,
            ..Default::default()
        };

        let summarizer = OllamaSummarizer::new(&config);
        assert_eq!(summarizer.url, "http://test:11434");
        assert_eq!(summarizer.model, "mistral");
        assert_eq!(summarizer.timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_name() {
        let config = SummaryConfig::default();
        let summarizer = OllamaSummarizer::new(&config);
        assert_eq!(summarizer.name(), "ollama");
    }
}
