//! Speech-to-text transcription module
//!
//! Provides local transcription using whisper.cpp via the whisper-rs crate.

pub mod whisper;

use crate::config::WhisperConfig;
use crate::error::TranscribeError;
use crate::setup::gpu;

/// Trait for speech-to-text implementations
pub trait Transcriber: Send + Sync {
    /// Transcribe audio samples to text
    /// Input: f32 samples, mono, 16kHz
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError>;
}

/// Factory function to create transcriber
pub fn create_transcriber(
    config: &WhisperConfig,
) -> Result<Box<dyn Transcriber>, TranscribeError> {
    // Apply GPU selection from VOXTYPE_VULKAN_DEVICE environment variable
    // This sets VK_LOADER_DRIVERS_SELECT to filter Vulkan drivers
    if let Some(vendor) = gpu::apply_gpu_selection() {
        tracing::info!("GPU selection: {} (via VOXTYPE_VULKAN_DEVICE)", vendor.display_name());
    }

    Ok(Box::new(whisper::WhisperTranscriber::new(config)?))
}
