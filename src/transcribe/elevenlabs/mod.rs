//! ElevenLabs Scribe cloud transcription.
//!
//! Batch mode uploads the finished recording (`batch`). Realtime mode streams
//! audio over a WebSocket and turns the provider's transcript snapshots into
//! streaming events (`realtime`).

mod batch;
mod realtime;

use std::sync::OnceLock;

use reqwest::header::{HeaderName, HeaderValue};

use super::{StreamingTranscriber, Transcriber};
use crate::{
    config::{ElevenLabsConfig, ElevenLabsRegion},
    error::TranscribeError,
};
use batch::validate_batch_input;

const SAMPLE_RATE: u32 = 16_000;
const MAX_PROVIDER_ERROR_CHARS: usize = 512;
const API_KEY_HEADER: HeaderName = HeaderName::from_static("xi-api-key");

fn api_origin(region: ElevenLabsRegion) -> &'static str {
    match region {
        ElevenLabsRegion::Global => "api.elevenlabs.io",
        ElevenLabsRegion::Us => "api.us.elevenlabs.io",
        ElevenLabsRegion::Eu => "api.eu.residency.elevenlabs.io",
        ElevenLabsRegion::India => "api.in.residency.elevenlabs.io",
        ElevenLabsRegion::Singapore => "api.sg.residency.elevenlabs.io",
    }
}

fn sensitive_api_key_header(api_key: &str) -> Result<HeaderValue, TranscribeError> {
    let mut value = HeaderValue::from_str(api_key).map_err(|_| {
        TranscribeError::ConfigError(
            "ElevenLabs API key contains bytes that cannot be used in an HTTP header".to_string(),
        )
    })?;
    value.set_sensitive(true);
    Ok(value)
}

pub struct ElevenLabsTranscriber {
    config: ElevenLabsConfig,
    api_key: String,
    batch_client: OnceLock<reqwest::Client>,
}

impl ElevenLabsTranscriber {
    pub fn new(config: &ElevenLabsConfig) -> Result<Self, TranscribeError> {
        config.validate().map_err(TranscribeError::ConfigError)?;
        let api_key = config.api_key.clone().ok_or_else(|| {
            TranscribeError::ConfigError(
                "ElevenLabs API key required: set [elevenlabs] api_key or ELEVENLABS_API_KEY"
                    .to_string(),
            )
        })?;
        sensitive_api_key_header(&api_key)?;

        let mut config = config.clone();
        config.api_key = None;

        Ok(Self {
            config,
            api_key,
            batch_client: OnceLock::new(),
        })
    }
}

impl Transcriber for ElevenLabsTranscriber {
    /// Run one synchronous batch transcription against ElevenLabs Scribe.
    ///
    /// Calls made inside Tokio must use a multi-thread runtime because the
    /// bridge uses `block_in_place`. Calls without an ambient runtime use a
    /// private current-thread runtime.
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        validate_batch_input(samples)?;

        let run = self.batch_transcribe(samples);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| handle.block_on(run)),
            Err(_) => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| {
                        TranscribeError::InferenceFailed(format!(
                            "Could not create ElevenLabs batch runtime: {error}"
                        ))
                    })?;
                runtime.block_on(run)
            }
        }
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        self.config.uses_realtime_api().then_some(self as _)
    }
}

fn f32_to_pcm16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn f32_to_pcm16le(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * size_of::<i16>());
    for &sample in samples {
        bytes.extend_from_slice(&f32_to_pcm16(sample).to_le_bytes());
    }
    bytes
}

fn redact_api_key(text: &str, api_key: &str) -> String {
    if api_key.is_empty() {
        text.to_string()
    } else {
        text.replace(api_key, "[REDACTED]")
    }
}

fn redact_and_bound(text: &str, api_key: &str) -> String {
    let redacted = redact_api_key(text, api_key);
    if redacted.chars().count() <= MAX_PROVIDER_ERROR_CHARS {
        return redacted;
    }

    let mut bounded: String = redacted.chars().take(MAX_PROVIDER_ERROR_CHARS).collect();
    bounded.push_str("...");
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    const API_KEY: &str = "elevenlabs-secret-key";

    #[test]
    fn converts_f32_samples_to_clipped_pcm16le() {
        let samples = [-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0];
        let actual = f32_to_pcm16le(&samples);
        let expected_samples = [-32767i16, -32767, -16384, 0, 16384, 32767, 32767];
        let expected: Vec<u8> = expected_samples
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();

        assert_eq!(actual, expected);
        assert_eq!(&actual[4..6], &(-16384i16).to_le_bytes());
        assert_eq!(&actual[8..10], &16384i16.to_le_bytes());
    }

    #[test]
    fn redacts_all_key_occurrences_and_bounds_provider_errors() {
        let long_error = format!("{API_KEY} middle {API_KEY} {}", "x".repeat(600));
        let surfaced = redact_and_bound(&long_error, API_KEY);

        assert!(!surfaced.contains(API_KEY));
        assert_eq!(surfaced.matches("[REDACTED]").count(), 2);
        assert_eq!(surfaced.chars().count(), MAX_PROVIDER_ERROR_CHARS + 3);
        assert!(surfaced.ends_with("..."));
    }
}
