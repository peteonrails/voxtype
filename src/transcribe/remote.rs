//! Remote speech-to-text transcription via OpenAI-compatible API
//!
//! Sends audio to a remote whisper.cpp server or OpenAI-compatible endpoint
//! for transcription, enabling use of GPU servers for faster inference.
//!
//! Note: Remote APIs don't support language arrays. When a language array is
//! configured, the first/primary language is used.

use super::Transcriber;
use crate::config::{LanguageConfig, WhisperConfig};
use crate::error::TranscribeError;
use std::io::Cursor;
use std::time::Duration;
use ureq::serde_json;

/// Remote transcriber using OpenAI-compatible Whisper API
#[derive(Debug)]
pub struct RemoteTranscriber {
    /// Base endpoint URL (e.g., "http://192.168.1.100:8080")
    endpoint: String,
    /// Model name to send to server
    model: String,
    /// Language configuration
    language: LanguageConfig,
    /// Whether to translate to English
    translate: bool,
    /// Optional API key for authentication
    api_key: Option<String>,
    /// Request timeout
    timeout: Duration,
}

impl RemoteTranscriber {
    /// Create a new remote transcriber from config
    pub fn new(config: &WhisperConfig) -> Result<Self, TranscribeError> {
        let endpoint = config
            .remote_endpoint
            .as_ref()
            .ok_or_else(|| {
                TranscribeError::ConfigError(
                    "remote_endpoint is required when backend = 'remote'".into(),
                )
            })?
            .clone();

        // Validate endpoint URL format
        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
            return Err(TranscribeError::ConfigError(format!(
                "remote_endpoint must start with http:// or https://, got: {}",
                endpoint
            )));
        }

        // Warn about non-HTTPS for non-localhost endpoints
        if endpoint.starts_with("http://")
            && !endpoint.contains("localhost")
            && !endpoint.contains("127.0.0.1")
            && !endpoint.contains("[::1]")
        {
            tracing::warn!(
                "Remote endpoint uses HTTP without TLS. Audio data will be transmitted unencrypted!"
            );
        }

        // Check for API key in config or environment
        let api_key = config
            .remote_api_key
            .clone()
            .or_else(|| std::env::var("VOXTYPE_WHISPER_API_KEY").ok());

        let model = config
            .remote_model
            .clone()
            .unwrap_or_else(|| "whisper-1".to_string());

        let timeout = Duration::from_secs(config.remote_timeout_secs.unwrap_or(30));

        // Warn if language array is configured (remote APIs don't support arrays)
        if config.language.is_multiple() {
            tracing::warn!(
                "Remote backend doesn't support language arrays. Using primary language '{}' from {:?}",
                config.language.primary(),
                config.language.as_vec()
            );
        }

        tracing::info!(
            "Configured remote transcriber: endpoint={}, model={}, timeout={}s",
            endpoint,
            model,
            timeout.as_secs()
        );

        Ok(Self {
            endpoint,
            model,
            language: config.language.clone(),
            translate: config.translate,
            api_key,
            timeout,
        })
    }

    /// Encode f32 samples to WAV format
    fn encode_wav(&self, samples: &[f32]) -> Result<Vec<u8>, TranscribeError> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut buffer = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut buffer, spec).map_err(|e| {
            TranscribeError::AudioFormat(format!("Failed to create WAV writer: {}", e))
        })?;

        // Convert f32 [-1.0, 1.0] to i16
        for &sample in samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let scaled = (clamped * i16::MAX as f32) as i16;
            writer.write_sample(scaled).map_err(|e| {
                TranscribeError::AudioFormat(format!("Failed to write sample: {}", e))
            })?;
        }

        writer
            .finalize()
            .map_err(|e| TranscribeError::AudioFormat(format!("Failed to finalize WAV: {}", e)))?;

        Ok(buffer.into_inner())
    }

    /// Build the multipart form body for the API request
    fn build_multipart_body(&self, wav_data: &[u8]) -> (String, Vec<u8>) {
        let boundary = format!(
            "----VoxtypeBoundary{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        let mut body = Vec::new();

        // Add file field
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: audio/wav\r\n\r\n");
        body.extend_from_slice(wav_data);
        body.extend_from_slice(b"\r\n");

        // Add model field
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"model\"\r\n\r\n");
        body.extend_from_slice(self.model.as_bytes());
        body.extend_from_slice(b"\r\n");

        // Add language field (if not auto-detect mode)
        // For language arrays, use the primary language since remote APIs don't support arrays
        if !self.language.is_auto() {
            body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            body.extend_from_slice(b"Content-Disposition: form-data; name=\"language\"\r\n\r\n");
            body.extend_from_slice(self.language.primary().as_bytes());
            body.extend_from_slice(b"\r\n");
        }

        // Add response_format field
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"response_format\"\r\n\r\n");
        body.extend_from_slice(b"json\r\n");

        // End boundary
        body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

        (boundary, body)
    }
}

impl Transcriber for RemoteTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".into()));
        }

        let duration_secs = samples.len() as f32 / 16000.0;
        tracing::debug!(
            "Sending {:.2}s of audio to remote server ({} samples)",
            duration_secs,
            samples.len()
        );

        let start = std::time::Instant::now();

        // Encode audio to WAV
        let wav_data = self.encode_wav(samples)?;
        tracing::debug!("Encoded WAV: {} bytes", wav_data.len());

        // Build multipart form
        let (boundary, body) = self.build_multipart_body(&wav_data);

        // Determine the API path based on whether we're doing transcription or translation
        let path = if self.translate {
            "/v1/audio/translations"
        } else {
            "/v1/audio/transcriptions"
        };

        let url = format!("{}{}", self.endpoint.trim_end_matches('/'), path);

        // Build request
        let mut request = ureq::post(&url).timeout(self.timeout).set(
            "Content-Type",
            &format!("multipart/form-data; boundary={}", boundary),
        );

        // Add authorization if API key is configured
        if let Some(ref key) = self.api_key {
            request = request.set("Authorization", &format!("Bearer {}", key));
        }

        // Send request
        let response = request.send_bytes(&body).map_err(|e| match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                TranscribeError::RemoteError(format!("Server returned {}: {}", code, body))
            }
            ureq::Error::Transport(t) => {
                TranscribeError::NetworkError(format!("Request failed: {}", t))
            }
        })?;

        // Parse JSON response
        let json: serde_json::Value = response.into_json().map_err(|e| {
            TranscribeError::RemoteError(format!("Failed to parse response: {}", e))
        })?;

        // Extract text from response
        let text = json
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TranscribeError::RemoteError(format!("Response missing 'text' field: {}", json))
            })?
            .trim()
            .to_string();

        tracing::info!(
            "Remote transcription completed in {:.2}s: {:?}",
            start.elapsed().as_secs_f32(),
            if text.chars().count() > 50 {
                format!("{}...", text.chars().take(50).collect::<String>())
            } else {
                text.clone()
            }
        );

        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_wav_basic() {
        let config = WhisperConfig {
            backend: crate::config::WhisperBackend::Remote,
            model: "base.en".to_string(),
            language: LanguageConfig::Single("en".to_string()),
            translate: false,
            threads: None,
            on_demand_loading: false,
            gpu_isolation: false,
            context_window_optimization: true,
            remote_endpoint: Some("http://localhost:8080".to_string()),
            remote_model: None,
            remote_api_key: None,
            remote_timeout_secs: None,
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();

        // Create a simple sine wave
        let samples: Vec<f32> = (0..16000)
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin() * 0.5)
            .collect();

        let wav = transcriber.encode_wav(&samples).unwrap();

        // WAV header is 44 bytes, then 16000 samples * 2 bytes = 32000 bytes
        assert_eq!(wav.len(), 44 + 32000);

        // Check WAV magic
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn test_config_validation_missing_endpoint() {
        let config = WhisperConfig {
            backend: crate::config::WhisperBackend::Remote,
            model: "base.en".to_string(),
            language: LanguageConfig::Single("en".to_string()),
            translate: false,
            threads: None,
            on_demand_loading: false,
            gpu_isolation: false,
            context_window_optimization: true,
            remote_endpoint: None, // Missing!
            remote_model: None,
            remote_api_key: None,
            remote_timeout_secs: None,
        };

        let result = RemoteTranscriber::new(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("remote_endpoint"));
    }

    #[test]
    fn test_config_validation_invalid_url() {
        let config = WhisperConfig {
            backend: crate::config::WhisperBackend::Remote,
            model: "base.en".to_string(),
            language: LanguageConfig::Single("en".to_string()),
            translate: false,
            threads: None,
            on_demand_loading: false,
            gpu_isolation: false,
            context_window_optimization: true,
            remote_endpoint: Some("not-a-url".to_string()),
            remote_model: None,
            remote_api_key: None,
            remote_timeout_secs: None,
        };

        let result = RemoteTranscriber::new(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("http://"));
    }

    #[test]
    fn test_multipart_body_structure() {
        let config = WhisperConfig {
            backend: crate::config::WhisperBackend::Remote,
            model: "base.en".to_string(),
            language: LanguageConfig::Single("en".to_string()),
            translate: false,
            threads: None,
            on_demand_loading: false,
            gpu_isolation: false,
            context_window_optimization: true,
            remote_endpoint: Some("http://localhost:8080".to_string()),
            remote_model: Some("large-v3".to_string()),
            remote_api_key: None,
            remote_timeout_secs: None,
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        let wav_data = vec![0u8; 100]; // Dummy data

        let (boundary, body) = transcriber.build_multipart_body(&wav_data);

        let body_str = String::from_utf8_lossy(&body);

        // Verify boundary is used
        assert!(body_str.contains(&boundary));

        // Verify required fields
        assert!(body_str.contains("name=\"file\""));
        assert!(body_str.contains("filename=\"audio.wav\""));
        assert!(body_str.contains("name=\"model\""));
        assert!(body_str.contains("large-v3"));
        assert!(body_str.contains("name=\"language\""));
        assert!(body_str.contains("name=\"response_format\""));
        assert!(body_str.contains("json"));
    }

    #[test]
    fn test_translate_false_uses_transcriptions_endpoint() {
        let config = WhisperConfig {
            backend: crate::config::WhisperBackend::Remote,
            model: "base.en".to_string(),
            language: LanguageConfig::Single("en".to_string()),
            translate: false,
            threads: None,
            on_demand_loading: false,
            gpu_isolation: false,
            context_window_optimization: true,
            remote_endpoint: Some("http://localhost:8080".to_string()),
            remote_model: None,
            remote_api_key: None,
            remote_timeout_secs: None,
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();

        // Verify translate flag is stored correctly
        assert!(!transcriber.translate);

        // The endpoint path logic: if !translate, use /v1/audio/transcriptions
        let path = if transcriber.translate {
            "/v1/audio/translations"
        } else {
            "/v1/audio/transcriptions"
        };
        assert_eq!(path, "/v1/audio/transcriptions");
    }

    #[test]
    fn test_translate_true_uses_translations_endpoint() {
        let config = WhisperConfig {
            backend: crate::config::WhisperBackend::Remote,
            model: "base.en".to_string(),
            language: LanguageConfig::Single("auto".to_string()),
            translate: true,
            threads: None,
            on_demand_loading: false,
            gpu_isolation: false,
            context_window_optimization: true,
            remote_endpoint: Some("http://localhost:8080".to_string()),
            remote_model: None,
            remote_api_key: None,
            remote_timeout_secs: None,
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();

        // Verify translate flag is stored correctly
        assert!(transcriber.translate);

        // The endpoint path logic: if translate, use /v1/audio/translations
        let path = if transcriber.translate {
            "/v1/audio/translations"
        } else {
            "/v1/audio/transcriptions"
        };
        assert_eq!(path, "/v1/audio/translations");
    }

    #[test]
    fn test_api_key_from_config() {
        let config = WhisperConfig {
            backend: crate::config::WhisperBackend::Remote,
            model: "base.en".to_string(),
            language: LanguageConfig::Single("en".to_string()),
            translate: false,
            threads: None,
            on_demand_loading: false,
            gpu_isolation: false,
            context_window_optimization: true,
            remote_endpoint: Some("http://localhost:8080".to_string()),
            remote_model: None,
            remote_api_key: Some("sk-test-key-123".to_string()),
            remote_timeout_secs: None,
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        assert_eq!(transcriber.api_key, Some("sk-test-key-123".to_string()));
    }

    #[test]
    fn test_custom_timeout() {
        let config = WhisperConfig {
            backend: crate::config::WhisperBackend::Remote,
            model: "base.en".to_string(),
            language: LanguageConfig::Single("en".to_string()),
            translate: false,
            threads: None,
            on_demand_loading: false,
            gpu_isolation: false,
            context_window_optimization: true,
            remote_endpoint: Some("http://localhost:8080".to_string()),
            remote_model: None,
            remote_api_key: None,
            remote_timeout_secs: Some(60),
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        assert_eq!(transcriber.timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_default_timeout() {
        let config = WhisperConfig {
            backend: crate::config::WhisperBackend::Remote,
            model: "base.en".to_string(),
            language: LanguageConfig::Single("en".to_string()),
            translate: false,
            threads: None,
            on_demand_loading: false,
            gpu_isolation: false,
            context_window_optimization: true,
            remote_endpoint: Some("http://localhost:8080".to_string()),
            remote_model: None,
            remote_api_key: None,
            remote_timeout_secs: None,
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        assert_eq!(transcriber.timeout, Duration::from_secs(30));
    }
}
