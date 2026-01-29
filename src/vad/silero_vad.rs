//! Silero VAD implementation using voice_activity_detector crate
//!
//! Uses the voice_activity_detector crate which bundles the Silero VAD model.
//! This implementation is used with the Parakeet transcription engine.

use super::{VadResult, VoiceActivityDetector};
use crate::config::VadConfig;
use crate::error::VadError;
use std::path::Path;
use std::sync::Mutex;
use voice_activity_detector::VoiceActivityDetector as VadDetector;

/// Silero VAD implementation using voice_activity_detector crate
pub struct SileroVad {
    /// VAD detector instance (wrapped in Mutex for thread safety)
    detector: Mutex<VadDetector>,
    /// Speech detection threshold (0.0 - 1.0)
    threshold: f32,
    /// Minimum speech duration in milliseconds
    min_speech_duration_ms: u32,
}

impl SileroVad {
    /// Create a new Silero VAD instance
    ///
    /// # Arguments
    /// * `_model_path` - Ignored, the crate uses a bundled model
    /// * `config` - VAD configuration
    pub fn new(_model_path: &Path, config: &VadConfig) -> Result<Self, VadError> {
        tracing::debug!("Initializing Silero VAD (bundled model)");

        // voice_activity_detector requires 512 samples per chunk at 16kHz
        let detector = VadDetector::builder()
            .sample_rate(16000)
            .chunk_size(512usize)
            .build()
            .map_err(|e| VadError::InitFailed(format!("Failed to create VAD detector: {}", e)))?;

        tracing::info!("Silero VAD initialized successfully");

        Ok(Self {
            detector: Mutex::new(detector),
            threshold: config.threshold.clamp(0.0, 1.0),
            min_speech_duration_ms: config.min_speech_duration_ms,
        })
    }
}

impl VoiceActivityDetector for SileroVad {
    fn detect(&self, samples: &[f32]) -> Result<VadResult, VadError> {
        let mut detector = self
            .detector
            .lock()
            .map_err(|e| VadError::DetectionFailed(format!("Failed to acquire VAD lock: {}", e)))?;

        // Process audio in chunks of 512 samples (required by voice_activity_detector)
        const CHUNK_SIZE: usize = 512;
        let chunk_duration_secs = CHUNK_SIZE as f32 / 16000.0; // 32ms per chunk

        let mut speech_chunks = 0;
        let mut total_chunks = 0;

        // Process audio in chunks
        for chunk in samples.chunks(CHUNK_SIZE) {
            // Pad last chunk if needed
            let prob = if chunk.len() < CHUNK_SIZE {
                let mut padded: Vec<f32> = chunk.to_vec();
                padded.resize(CHUNK_SIZE, 0.0);
                detector.predict(padded)
            } else {
                detector.predict(chunk.to_vec())
            };

            if prob >= self.threshold {
                speech_chunks += 1;
            }
            total_chunks += 1;
        }

        // Reset detector for next use
        detector.reset();

        // Calculate speech duration and ratio
        let speech_duration_secs = speech_chunks as f32 * chunk_duration_secs;
        let total_duration_secs = samples.len() as f32 / 16000.0;
        let speech_ratio = if total_chunks > 0 {
            speech_chunks as f32 / total_chunks as f32
        } else {
            0.0
        };

        // Determine if speech was detected
        let min_speech_secs = self.min_speech_duration_ms as f32 / 1000.0;
        let has_speech = speech_duration_secs >= min_speech_secs;

        tracing::debug!(
            "VAD result: {}/{} chunks with speech ({:.2}s, {:.1}% of {:.2}s total)",
            speech_chunks,
            total_chunks,
            speech_duration_secs,
            speech_ratio * 100.0,
            total_duration_secs
        );

        Ok(VadResult {
            has_speech,
            speech_duration_secs,
            speech_ratio,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_threshold_clamping() {
        assert_eq!(1.5f32.clamp(0.0, 1.0), 1.0);
        assert_eq!((-0.5f32).clamp(0.0, 1.0), 0.0);
        assert_eq!(0.5f32.clamp(0.0, 1.0), 0.5);
    }
}
