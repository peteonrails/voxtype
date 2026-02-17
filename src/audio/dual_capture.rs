//! Dual audio capture for meeting mode
//!
//! Captures both microphone input (user's voice) and system audio loopback
//! (remote participants) simultaneously for speaker attribution.

use super::cpal_capture::CpalCapture;
use super::AudioCapture;
use crate::config::AudioConfig;
use crate::error::AudioError;

/// Audio source identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSourceType {
    /// Microphone input (local user)
    Microphone,
    /// System audio loopback (remote participants)
    Loopback,
}

/// A sample with its source identified
#[derive(Debug, Clone)]
pub struct SourcedSample {
    /// The audio sample value
    pub sample: f32,
    /// Which source this sample came from
    pub source: AudioSourceType,
    /// Timestamp in samples (at 16kHz)
    pub timestamp: u64,
}

/// Dual audio capture for mic + loopback
pub struct DualCapture {
    /// Microphone capture
    mic_capture: CpalCapture,
    /// Loopback capture (system audio)
    loopback_capture: Option<CpalCapture>,
    /// Whether loopback is enabled
    loopback_enabled: bool,
    /// Sample counter for timestamps
    sample_counter: u64,
}

impl DualCapture {
    /// Create a new dual capture instance
    pub fn new(
        mic_config: &AudioConfig,
        loopback_device: Option<&str>,
    ) -> Result<Self, AudioError> {
        let mic_capture = CpalCapture::new(mic_config)?;

        // Try to create loopback capture if device is specified
        let (loopback_capture, loopback_enabled) = if let Some(device) = loopback_device {
            if device == "auto" {
                // Try to find a monitor/loopback device
                match Self::find_loopback_device() {
                    Some(device_name) => {
                        let mut loopback_config = mic_config.clone();
                        loopback_config.device = device_name;
                        match CpalCapture::new(&loopback_config) {
                            Ok(capture) => {
                                tracing::info!("Loopback capture enabled");
                                (Some(capture), true)
                            }
                            Err(e) => {
                                tracing::warn!("Failed to create loopback capture: {}", e);
                                (None, false)
                            }
                        }
                    }
                    None => {
                        tracing::warn!("No loopback device found, using mic only");
                        (None, false)
                    }
                }
            } else if device == "disabled" || device.is_empty() {
                (None, false)
            } else {
                // Use specified device
                let mut loopback_config = mic_config.clone();
                loopback_config.device = device.to_string();
                match CpalCapture::new(&loopback_config) {
                    Ok(capture) => {
                        tracing::info!("Loopback capture enabled: {}", device);
                        (Some(capture), true)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to create loopback capture for '{}': {}", device, e);
                        (None, false)
                    }
                }
            }
        } else {
            (None, false)
        };

        Ok(Self {
            mic_capture,
            loopback_capture,
            loopback_enabled,
            sample_counter: 0,
        })
    }

    /// Try to find a loopback/monitor device automatically
    fn find_loopback_device() -> Option<String> {
        use cpal::traits::{DeviceTrait, HostTrait};

        let host = cpal::default_host();
        let devices = host.input_devices().ok()?;

        for device in devices {
            if let Ok(name) = device.name() {
                let name_lower = name.to_lowercase();
                // Common loopback device name patterns
                if name_lower.contains("monitor")
                    || name_lower.contains("loopback")
                    || name_lower.contains("stereo mix")
                    || name_lower.contains("what u hear")
                {
                    tracing::debug!("Found loopback device: {}", name);
                    return Some(name);
                }
            }
        }

        None
    }

    /// Check if loopback capture is active
    pub fn has_loopback(&self) -> bool {
        self.loopback_enabled && self.loopback_capture.is_some()
    }

    /// Start both captures
    pub async fn start(&mut self) -> Result<(), AudioError> {
        // Start mic capture
        let _mic_rx = self.mic_capture.start().await?;

        // Start loopback capture if available
        if let Some(ref mut loopback) = self.loopback_capture {
            let _loopback_rx = loopback.start().await?;
        }

        Ok(())
    }

    /// Stop both captures and return all samples
    pub async fn stop(&mut self) -> Result<DualSamples, AudioError> {
        let mic_samples = self.mic_capture.stop().await?;

        let loopback_samples = if let Some(ref mut loopback) = self.loopback_capture {
            loopback.stop().await.unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(DualSamples {
            mic: mic_samples,
            loopback: loopback_samples,
        })
    }

    /// Get current samples without stopping (for continuous recording)
    pub async fn get_samples(&mut self) -> DualSamples {
        let mic = self.mic_capture.get_samples().await;

        let loopback = if let Some(ref mut loopback) = self.loopback_capture {
            loopback.get_samples().await
        } else {
            Vec::new()
        };

        DualSamples { mic, loopback }
    }

    /// Get sourced samples with timestamps for diarization
    pub async fn get_sourced_samples(&mut self) -> Vec<SourcedSample> {
        let dual = self.get_samples().await;
        let mut result = Vec::with_capacity(dual.mic.len() + dual.loopback.len());

        // Add mic samples
        for sample in dual.mic {
            result.push(SourcedSample {
                sample,
                source: AudioSourceType::Microphone,
                timestamp: self.sample_counter,
            });
            self.sample_counter += 1;
        }

        // Add loopback samples (interleaved timing approximation)
        for sample in dual.loopback {
            result.push(SourcedSample {
                sample,
                source: AudioSourceType::Loopback,
                timestamp: self.sample_counter,
            });
            self.sample_counter += 1;
        }

        result
    }
}

/// Samples from both sources
#[derive(Debug, Clone, Default)]
pub struct DualSamples {
    /// Microphone samples
    pub mic: Vec<f32>,
    /// Loopback samples
    pub loopback: Vec<f32>,
}

impl DualSamples {
    /// Check if there are any samples
    pub fn is_empty(&self) -> bool {
        self.mic.is_empty() && self.loopback.is_empty()
    }

    /// Total number of samples
    pub fn len(&self) -> usize {
        self.mic.len() + self.loopback.len()
    }

    /// Merge samples into a single stream (for transcription)
    /// Prioritizes mic when both have audio, mixes otherwise
    pub fn merge(&self) -> Vec<f32> {
        if self.loopback.is_empty() {
            return self.mic.clone();
        }
        if self.mic.is_empty() {
            return self.loopback.clone();
        }

        // Mix both streams
        let max_len = self.mic.len().max(self.loopback.len());
        let mut merged = Vec::with_capacity(max_len);

        for i in 0..max_len {
            let mic_sample = self.mic.get(i).copied().unwrap_or(0.0);
            let loopback_sample = self.loopback.get(i).copied().unwrap_or(0.0);
            // Simple mix with slight preference to mic
            merged.push(mic_sample * 0.6 + loopback_sample * 0.4);
        }

        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dual_samples_merge_mic_only() {
        let samples = DualSamples {
            mic: vec![0.5, 0.6, 0.7],
            loopback: vec![],
        };
        assert_eq!(samples.merge(), vec![0.5, 0.6, 0.7]);
    }

    #[test]
    fn test_dual_samples_merge_loopback_only() {
        let samples = DualSamples {
            mic: vec![],
            loopback: vec![0.3, 0.4],
        };
        assert_eq!(samples.merge(), vec![0.3, 0.4]);
    }

    #[test]
    fn test_dual_samples_merge_both() {
        let samples = DualSamples {
            mic: vec![1.0, 1.0],
            loopback: vec![1.0, 1.0],
        };
        let merged = samples.merge();
        // 1.0 * 0.6 + 1.0 * 0.4 = 1.0
        assert!((merged[0] - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_dual_samples_is_empty() {
        let empty = DualSamples::default();
        assert!(empty.is_empty());

        let with_mic = DualSamples {
            mic: vec![0.1],
            loopback: vec![],
        };
        assert!(!with_mic.is_empty());
    }
}
