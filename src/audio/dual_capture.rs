//! Dual audio capture for meeting mode
//!
//! Captures both microphone input (user's voice) and system audio loopback
//! (remote participants) simultaneously for speaker attribution.
//!
//! Mic capture uses cpal (ALSA). Loopback capture uses `parec` (PulseAudio
//! recording client) which works with PipeWire's PulseAudio compatibility
//! layer and can access monitor sources that aren't visible to ALSA.

use super::cpal_capture::CpalCapture;
use super::AudioCapture;
use crate::config::AudioConfig;
use crate::error::AudioError;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

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

/// Loopback capture via parec subprocess
struct ParecLoopback {
    /// Source name (PulseAudio/PipeWire source)
    source: String,
    /// Child process
    child: Option<std::process::Child>,
    /// Shared buffer for received samples
    buffer: Arc<Mutex<Vec<f32>>>,
    /// Reader thread handle
    reader_thread: Option<std::thread::JoinHandle<()>>,
}

impl ParecLoopback {
    fn new(source: String) -> Self {
        Self {
            source,
            child: None,
            buffer: Arc::new(Mutex::new(Vec::new())),
            reader_thread: None,
        }
    }

    fn start(&mut self) -> Result<(), AudioError> {
        let mut child = std::process::Command::new("parec")
            .args([
                "--device", &self.source,
                "--format=float32le",
                "--channels=1",
                "--rate=16000",
                "--raw",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| AudioError::Connection(format!("Failed to start parec: {}", e)))?;

        let mut stdout = child.stdout.take()
            .ok_or_else(|| AudioError::Connection("Failed to capture parec stdout".to_string()))?;

        self.child = Some(child);
        tracing::info!("Loopback capture started via parec: {}", self.source);

        // Spawn reader thread
        let buffer = Arc::clone(&self.buffer);
        self.reader_thread = Some(std::thread::spawn(move || {
            use std::io::Read;
            let mut raw_buf = [0u8; 4096]; // 1024 f32 samples
            loop {
                match stdout.read(&mut raw_buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        // Convert raw bytes to f32 samples
                        let sample_count = n / 4;
                        let mut samples = Vec::with_capacity(sample_count);
                        for i in 0..sample_count {
                            let offset = i * 4;
                            if offset + 4 <= n {
                                let sample = f32::from_le_bytes([
                                    raw_buf[offset],
                                    raw_buf[offset + 1],
                                    raw_buf[offset + 2],
                                    raw_buf[offset + 3],
                                ]);
                                samples.push(sample);
                            }
                        }
                        if let Ok(mut buf) = buffer.lock() {
                            buf.extend(samples);
                        }
                    }
                    Err(_) => break,
                }
            }
            tracing::debug!("Loopback reader thread stopped");
        }));

        Ok(())
    }

    fn get_samples(&self) -> Vec<f32> {
        if let Ok(mut buf) = self.buffer.lock() {
            std::mem::take(&mut *buf)
        } else {
            Vec::new()
        }
    }

    fn stop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        if let Some(thread) = self.reader_thread.take() {
            let _ = thread.join();
        }
        tracing::debug!("Loopback capture stopped");
    }
}

impl Drop for ParecLoopback {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Dual audio capture for mic + loopback
pub struct DualCapture {
    /// Microphone capture (via cpal/ALSA)
    mic_capture: CpalCapture,
    /// Loopback capture (via parec subprocess)
    loopback: Option<ParecLoopback>,
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

        let loopback = match loopback_device {
            Some("disabled") | Some("") | None => None,
            Some("auto") => {
                match Self::find_monitor_source() {
                    Some(source) => {
                        tracing::info!("Auto-detected loopback source: {}", source);
                        Some(ParecLoopback::new(source))
                    }
                    None => {
                        tracing::warn!("No monitor source found, using mic only");
                        None
                    }
                }
            }
            Some(device) => {
                tracing::info!("Using configured loopback source: {}", device);
                Some(ParecLoopback::new(device.to_string()))
            }
        };

        Ok(Self {
            mic_capture,
            loopback,
            sample_counter: 0,
        })
    }

    /// Find a PipeWire/PulseAudio monitor source via pactl
    fn find_monitor_source() -> Option<String> {
        // pactl list short sources output format:
        // ID\tNAME\tDRIVER\tFORMAT\tSTATUS
        let output = std::process::Command::new("pactl")
            .args(["list", "short", "sources"])
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        // First pass: prefer RUNNING monitor sources (active audio output)
        for line in stdout.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() >= 5 {
                let name = fields[1];
                let status = fields[4];
                if name.contains(".monitor") && status == "RUNNING" {
                    tracing::debug!("Found running monitor source: {}", name);
                    return Some(name.to_string());
                }
            }
        }
        // Fall back to any monitor source
        for line in stdout.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() >= 2 {
                let name = fields[1];
                if name.contains(".monitor") {
                    tracing::debug!("Found monitor source: {}", name);
                    return Some(name.to_string());
                }
            }
        }
        None
    }

    /// Check if loopback capture is active
    pub fn has_loopback(&self) -> bool {
        self.loopback.is_some()
    }

    /// Start both captures
    pub async fn start(&mut self) -> Result<(), AudioError> {
        let _mic_rx = self.mic_capture.start().await?;

        if let Some(ref mut loopback) = self.loopback {
            if let Err(e) = loopback.start() {
                tracing::warn!("Loopback capture failed, continuing with mic only: {}", e);
                self.loopback = None;
            }
        }

        Ok(())
    }

    /// Stop both captures and return all samples
    pub async fn stop(&mut self) -> Result<DualSamples, AudioError> {
        let mic_samples = self.mic_capture.stop().await?;

        let loopback_samples = if let Some(ref mut loopback) = self.loopback {
            let samples = loopback.get_samples();
            loopback.stop();
            samples
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

        let loopback = if let Some(ref loopback) = self.loopback {
            loopback.get_samples()
        } else {
            Vec::new()
        };

        DualSamples { mic, loopback }
    }

    /// Get sourced samples with timestamps for diarization
    pub async fn get_sourced_samples(&mut self) -> Vec<SourcedSample> {
        let dual = self.get_samples().await;
        let mut result = Vec::with_capacity(dual.mic.len() + dual.loopback.len());

        for sample in dual.mic {
            result.push(SourcedSample {
                sample,
                source: AudioSourceType::Microphone,
                timestamp: self.sample_counter,
            });
            self.sample_counter += 1;
        }

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
