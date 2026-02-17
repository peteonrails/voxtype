//! Remote API summarization backend
//!
//! Integrates with a remote summarization service for meetings.
//! Useful for corporate deployments with centralized AI infrastructure.

use super::{generate_prompt, parse_summary_response, Summarizer, SummaryConfig, SummaryError};
use crate::meeting::data::{MeetingData, MeetingSummary};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Remote API-based summarizer
pub struct RemoteSummarizer {
    /// API endpoint URL
    endpoint: String,
    /// API key for authentication
    api_key: Option<String>,
    /// Request timeout
    timeout: Duration,
}

impl RemoteSummarizer {
    /// Create a new remote summarizer
    pub fn new(config: &SummaryConfig) -> Self {
        Self {
            endpoint: config
                .remote_endpoint
                .clone()
                .unwrap_or_else(|| "http://localhost:8080/api/summarize".to_string()),
            api_key: config.remote_api_key.clone(),
            timeout: Duration::from_secs(config.timeout_secs),
        }
    }

    /// Call the remote summarization API
    fn call_api(&self, prompt: &str) -> Result<String, SummaryError> {
        let client = ureq::AgentBuilder::new().timeout(self.timeout).build();

        #[derive(Serialize)]
        struct SummarizeRequest<'a> {
            prompt: &'a str,
        }

        let mut request = client.post(&self.endpoint);

        // Add API key if configured
        if let Some(ref api_key) = self.api_key {
            request = request.set("Authorization", &format!("Bearer {}", api_key));
        }

        request = request.set("Content-Type", "application/json");

        let body = SummarizeRequest { prompt };

        tracing::debug!("Calling remote summarization API: {}", self.endpoint);

        let response = request.send_json(&body).map_err(|e| match e {
            ureq::Error::Transport(ref t) => {
                let msg = t.to_string();
                if msg.contains("timed out") || msg.contains("timeout") {
                    SummaryError::Request("Request timed out - try a shorter transcript".into())
                } else {
                    SummaryError::Request(e.to_string())
                }
            }
            ureq::Error::Status(status, _) => {
                SummaryError::Request(format!("API returned status {}", status))
            }
        })?;

        #[derive(Deserialize)]
        struct ApiResponse {
            summary: Option<String>,
            response: Option<String>,
            error: Option<String>,
        }

        let api_response: ApiResponse = response
            .into_json()
            .map_err(|e| SummaryError::Parse(format!("Failed to parse API response: {}", e)))?;

        if let Some(error) = api_response.error {
            return Err(SummaryError::Request(error));
        }

        api_response
            .summary
            .or(api_response.response)
            .ok_or_else(|| SummaryError::Parse("API response missing summary field".into()))
    }
}

impl Summarizer for RemoteSummarizer {
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

        // Call remote API
        let response = self.call_api(&prompt)?;
        tracing::debug!("Received response ({} chars)", response.len());

        // Parse response
        let summary = parse_summary_response(&response, Some("remote".to_string()))?;

        Ok(summary)
    }

    fn name(&self) -> &'static str {
        "remote"
    }

    fn is_available(&self) -> bool {
        // Try a simple health check
        let client = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .build();

        // Try to reach the endpoint
        client.head(&self.endpoint).call().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_from_config() {
        let config = SummaryConfig {
            remote_endpoint: Some("https://api.example.com/summarize".to_string()),
            remote_api_key: Some("test-key".to_string()),
            timeout_secs: 90,
            ..Default::default()
        };

        let summarizer = RemoteSummarizer::new(&config);
        assert_eq!(summarizer.endpoint, "https://api.example.com/summarize");
        assert_eq!(summarizer.api_key, Some("test-key".to_string()));
        assert_eq!(summarizer.timeout, Duration::from_secs(90));
    }

    #[test]
    fn test_name() {
        let config = SummaryConfig::default();
        let summarizer = RemoteSummarizer::new(&config);
        assert_eq!(summarizer.name(), "remote");
    }
}
