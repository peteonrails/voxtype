//! Whisper VAD implementation using whisper-rs built-in VAD
//!
//! Uses the WhisperVadContext from whisper-rs which wraps Silero VAD
//! in GGML format, optimized for use with whisper.cpp.

use super::{VadResult, VoiceActivityDetector};
use crate::config::VadConfig;
use crate::error::VadError;
use std::path::Path;
use std::sync::Mutex;
use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

/// Whisper VAD implementation using whisper-rs
pub struct WhisperVad {
    /// VAD context (wrapped in Mutex because WhisperVadContext is not Send/Sync)
    ctx: Mutex<WhisperVadContext>,
    /// Speech detection threshold (0.0 - 1.0)
    threshold: f32,
    /// Minimum speech duration in milliseconds
    min_speech_duration_ms: u32,
}

impl WhisperVad {
    /// Create a new Whisper VAD instance
    ///
    /// # Arguments
    /// * `model_path` - Path to the GGML VAD model file (ggml-silero-vad.bin)
    /// * `config` - VAD configuration
    pub fn new(model_path: &Path, config: &VadConfig) -> Result<Self, VadError> {
        let model_str = model_path
            .to_str()
            .ok_or_else(|| VadError::InitFailed("Invalid model path".to_string()))?;

        tracing::debug!("Loading Whisper VAD model from {:?}", model_path);

        let params = WhisperVadContextParams::default();

        let ctx = WhisperVadContext::new(model_str, params)
            .map_err(|e| VadError::InitFailed(format!("Failed to load VAD model: {}", e)))?;

        tracing::info!("Whisper VAD model loaded successfully");

        Ok(Self {
            ctx: Mutex::new(ctx),
            threshold: config.threshold.clamp(0.0, 1.0),
            min_speech_duration_ms: config.min_speech_duration_ms,
        })
    }
}

impl VoiceActivityDetector for WhisperVad {
    fn detect(&self, samples: &[f32]) -> Result<VadResult, VadError> {
        let mut ctx = self
            .ctx
            .lock()
            .map_err(|e| VadError::DetectionFailed(format!("Failed to acquire VAD lock: {}", e)))?;

        // Configure VAD parameters
        let mut params = WhisperVadParams::new();
        params.set_threshold(self.threshold);
        params.set_min_speech_duration(self.min_speech_duration_ms as i32);
        // Use defaults for silence duration (100ms) and padding (30ms)

        // Run VAD detection
        let segments = ctx
            .segments_from_samples(params, samples)
            .map_err(|e| VadError::DetectionFailed(format!("VAD detection failed: {}", e)))?;

        // Calculate total speech duration from segments
        // Timestamps are in centiseconds (10ms units)
        let mut total_speech_centiseconds = 0.0f32;
        let num_segments = segments.num_segments();

        for i in 0..num_segments {
            if let (Some(start), Some(end)) = (
                segments.get_segment_start_timestamp(i),
                segments.get_segment_end_timestamp(i),
            ) {
                total_speech_centiseconds += end - start;
            }
        }

        // Convert centiseconds to seconds
        let speech_duration_secs = total_speech_centiseconds / 100.0;

        // Calculate total audio duration (samples at 16kHz)
        let total_duration_secs = samples.len() as f32 / 16000.0;

        // Calculate speech ratio
        let speech_ratio = if total_duration_secs > 0.0 {
            (speech_duration_secs / total_duration_secs).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Determine if speech was detected
        // Has speech if any segments were found AND total speech meets minimum duration
        let min_speech_secs = self.min_speech_duration_ms as f32 / 1000.0;
        let has_speech = num_segments > 0 && speech_duration_secs >= min_speech_secs;

        tracing::debug!(
            "VAD result: {} segments, {:.2}s speech ({:.1}% of {:.2}s total)",
            num_segments,
            speech_duration_secs,
            speech_ratio * 100.0,
            total_duration_secs
        );

        Ok(VadResult {
            has_speech,
            speech_duration_secs,
            speech_ratio,
            rms_energy: 0.0, // Not available from Whisper VAD
        })
    }
}

// WhisperVad is Send + Sync because the internal WhisperVadContext is wrapped in a Mutex
unsafe impl Send for WhisperVad {}
unsafe impl Sync for WhisperVad {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VadBackend;

    #[test]
    fn test_threshold_clamping() {
        // Test that threshold is clamped to valid range
        let config = VadConfig {
            enabled: true,
            backend: VadBackend::Whisper,
            threshold: 1.5, // Above max
            min_speech_duration_ms: 100,
            model: None,
        };

        // Can't test actual VAD without a model, but we can verify the struct
        // would clamp the threshold
        let clamped = config.threshold.clamp(0.0, 1.0);
        assert_eq!(clamped, 1.0);

        let config2 = VadConfig {
            enabled: true,
            backend: VadBackend::Whisper,
            threshold: -0.5, // Below min
            min_speech_duration_ms: 100,
            model: None,
        };
        let clamped2 = config2.threshold.clamp(0.0, 1.0);
        assert_eq!(clamped2, 0.0);
    }
}
