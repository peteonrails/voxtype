//! Audio capture module
//!
//! Provides audio recording capabilities using cpal, which works with
//! PipeWire, PulseAudio, and ALSA backends.

pub mod cpal_capture;
pub mod dual_capture;
pub mod feedback;

pub use dual_capture::{AudioSourceType, DualCapture, DualSamples, SourcedSample};

use crate::config::AudioConfig;
use crate::error::AudioError;
use tokio::sync::mpsc;

/// Trait for audio capture implementations
#[async_trait::async_trait]
pub trait AudioCapture: Send + Sync {
    /// Start capturing audio
    /// Returns a channel receiver for audio chunks (f32 samples, mono, 16kHz)
    async fn start(&mut self) -> Result<mpsc::Receiver<Vec<f32>>, AudioError>;

    /// Stop capturing and return all recorded samples
    async fn stop(&mut self) -> Result<Vec<f32>, AudioError>;

    /// Get current samples without stopping (for continuous recording modes)
    /// This drains the internal buffer and returns samples collected since the last call.
    /// Returns an empty Vec if not yet started or already stopped.
    async fn get_samples(&mut self) -> Vec<f32>;
}

/// Factory function to create audio capture
pub fn create_capture(config: &AudioConfig) -> Result<Box<dyn AudioCapture>, AudioError> {
    Ok(Box::new(cpal_capture::CpalCapture::new(config)?))
}
