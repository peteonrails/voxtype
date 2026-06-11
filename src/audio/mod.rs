//! Audio capture module
//!
//! Provides audio recording capabilities using cpal, which works with
//! PipeWire, PulseAudio, and ALSA backends.

pub mod cpal_capture;
pub mod dual_capture;
#[cfg(feature = "onnx-common")]
pub mod enhance;
pub mod feedback;
pub mod levels;
pub mod media;

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

/// Wait until the capture stream delivers a chunk containing real signal
/// (any non-zero sample).
///
/// Input devices resuming from idle suspend (PipeWire/WirePlumber suspends
/// sources after ~5s idle) deliver exact digital zeros for ~0.5s before
/// real samples flow. Audio spoken into that window is never captured, so
/// the daemon gates its "listening" cues (start sound, OSD, notification)
/// on this returning. A live mic always has a noise floor, so any non-zero
/// sample means the device is warm.
///
/// Returns the chunk that contained the signal once one is seen. Callers
/// whose downstream consumes this channel as audio input (the streaming
/// transcriber) must forward that chunk — it may hold the onset of speech
/// when the user started talking during warm-up. Returns `None` on timeout
/// (some virtual sources, e.g. noise suppressors, emit exact zeros in a
/// quiet room — recording must start anyway) or if the channel closes.
pub async fn wait_for_signal(
    chunk_rx: &mut mpsc::Receiver<Vec<f32>>,
    timeout: std::time::Duration,
) -> Option<Vec<f32>> {
    let deadline = tokio::time::Instant::now() + timeout;
    let started = std::time::Instant::now();

    loop {
        match tokio::time::timeout_at(deadline, chunk_rx.recv()).await {
            Ok(Some(chunk)) => {
                if chunk.iter().any(|&s| s != 0.0) {
                    tracing::debug!(
                        "Audio device warm after {:.0}ms",
                        started.elapsed().as_secs_f32() * 1000.0
                    );
                    return Some(chunk);
                }
            }
            Ok(None) => {
                tracing::warn!("Audio stream closed while waiting for device warm-up");
                return None;
            }
            Err(_) => {
                // INFO, not WARN: this fires on every recording start for
                // sources that emit exact digital silence (by design the
                // gate must give up and start anyway), so WARN would be
                // perpetual noise rather than something actionable.
                tracing::info!(
                    "No signal from audio device within {}ms (suspended source still \
                     resuming, or a source that emits exact silence); starting anyway",
                    timeout.as_millis()
                );
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn wait_for_signal_returns_the_first_nonzero_chunk() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(vec![0.0, 0.01, 0.0]).await.unwrap();

        let chunk = wait_for_signal(&mut rx, Duration::from_secs(5)).await;
        assert_eq!(chunk, Some(vec![0.0, 0.01, 0.0]));
    }

    #[tokio::test]
    async fn wait_for_signal_skips_leading_silent_chunks() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(vec![0.0; 160]).await.unwrap();
        tx.send(vec![0.0; 160]).await.unwrap();
        tx.send(vec![0.0, -0.02]).await.unwrap();

        let chunk = wait_for_signal(&mut rx, Duration::from_secs(5)).await;
        assert_eq!(chunk, Some(vec![0.0, -0.02]));
    }

    #[tokio::test]
    async fn wait_for_signal_times_out_on_pure_silence() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(vec![0.0; 160]).await.unwrap();
        // Keep tx alive so the channel doesn't close; only the timeout
        // can end the wait.
        let result = wait_for_signal(&mut rx, Duration::from_millis(100)).await;
        drop(tx);

        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn wait_for_signal_returns_none_when_channel_closes() {
        let (tx, mut rx) = mpsc::channel::<Vec<f32>>(8);
        drop(tx);

        assert_eq!(wait_for_signal(&mut rx, Duration::from_secs(5)).await, None);
    }
}
