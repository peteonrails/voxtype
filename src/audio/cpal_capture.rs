//! cpal-based audio capture
//!
//! Uses the cpal crate for cross-platform audio input.
//! Works with PipeWire, PulseAudio, and ALSA backends.
//!
//! Note: cpal::Stream is not Send, so we run the audio capture in a
//! dedicated thread and communicate via channels.

use super::AudioCapture;
use crate::config::AudioConfig;
use crate::error::AudioError;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::{mpsc, oneshot};

/// Commands sent to the audio capture thread
enum CaptureCommand {
    Stop(oneshot::Sender<Vec<f32>>),
}

/// Parameters for building an audio input stream
struct StreamBuildParams {
    samples: Arc<Mutex<Vec<f32>>>,
    tx: mpsc::Sender<Vec<f32>>,
    source_rate: u32,
    target_rate: u32,
    source_channels: usize,
}

/// cpal-based audio capture implementation
pub struct CpalCapture {
    /// Audio configuration
    config: AudioConfig,
    /// Command sender to the capture thread
    cmd_tx: Option<std::sync::mpsc::Sender<CaptureCommand>>,
    /// Handle to the capture thread
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl CpalCapture {
    /// Create a new cpal audio capture instance
    pub fn new(config: &AudioConfig) -> Result<Self, AudioError> {
        Ok(Self {
            config: config.clone(),
            cmd_tx: None,
            thread_handle: None,
        })
    }
}

/// Find an audio input device by name with flexible matching.
///
/// Matching strategy (in order):
/// 1. Exact match (case-sensitive)
/// 2. Exact match (case-insensitive)
/// 3. Substring match: device name contains the search term (case-insensitive)
///
/// This allows users to specify either:
/// - Full cpal device names: "alsa_input.pci-0000_00_1f.3.analog-stereo"
/// - PipeWire/PulseAudio short names: "vox_buffer"
/// - Partial device names: "analog-stereo"
fn find_audio_device(host: &cpal::Host, device_name: &str) -> Result<cpal::Device, AudioError> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let devices: Vec<cpal::Device> = host
        .input_devices()
        .map_err(|e| AudioError::Connection(e.to_string()))?
        .collect();

    // Collect device names for error message
    let device_names: Vec<String> = devices.iter().filter_map(|d| d.name().ok()).collect();

    let search_lower = device_name.to_lowercase();

    // 1. Try exact match (case-sensitive)
    for device in &devices {
        if let Ok(name) = device.name() {
            if name == device_name {
                tracing::debug!("Found audio device by exact match: {}", name);
                return host
                    .input_devices()
                    .map_err(|e| AudioError::Connection(e.to_string()))?
                    .find(|d| d.name().map(|n| n == device_name).unwrap_or(false))
                    .ok_or_else(|| AudioError::DeviceNotFound(device_name.to_string()));
            }
        }
    }

    // 2. Try exact match (case-insensitive)
    for device in &devices {
        if let Ok(name) = device.name() {
            if name.to_lowercase() == search_lower {
                tracing::debug!(
                    "Found audio device by case-insensitive match: {} (searched for: {})",
                    name,
                    device_name
                );
                let matched_name = name.clone();
                return host
                    .input_devices()
                    .map_err(|e| AudioError::Connection(e.to_string()))?
                    .find(|d| d.name().map(|n| n == matched_name).unwrap_or(false))
                    .ok_or_else(|| AudioError::DeviceNotFound(device_name.to_string()));
            }
        }
    }

    // 3. Try substring match (case-insensitive)
    for device in &devices {
        if let Ok(name) = device.name() {
            if name.to_lowercase().contains(&search_lower) {
                tracing::debug!(
                    "Found audio device by substring match: {} (searched for: {})",
                    name,
                    device_name
                );
                let matched_name = name.clone();
                return host
                    .input_devices()
                    .map_err(|e| AudioError::Connection(e.to_string()))?
                    .find(|d| d.name().map(|n| n == matched_name).unwrap_or(false))
                    .ok_or_else(|| AudioError::DeviceNotFound(device_name.to_string()));
            }
        }
    }

    // No match found - provide helpful error with available devices
    let available = if device_names.is_empty() {
        "No audio input devices found.".to_string()
    } else {
        format!(
            "Available devices:\n{}",
            device_names
                .iter()
                .map(|n| format!("  - {}", n))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    Err(AudioError::DeviceNotFoundWithList {
        requested: device_name.to_string(),
        available,
    })
}

#[async_trait::async_trait]
impl AudioCapture for CpalCapture {
    async fn start(&mut self) -> Result<mpsc::Receiver<Vec<f32>>, AudioError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        // Get the device info before spawning the thread
        let host = cpal::default_host();

        let device = if self.config.device == "default" {
            host.default_input_device()
                .ok_or_else(|| AudioError::DeviceNotFound("default".to_string()))?
        } else {
            find_audio_device(&host, &self.config.device)?
        };

        let device_name = device.name().unwrap_or_else(|_| "unknown".to_string());
        tracing::info!("Using audio device: {}", device_name);

        // Get supported config
        let supported_config = device
            .default_input_config()
            .map_err(|e| AudioError::Connection(e.to_string()))?;

        let source_sample_rate = supported_config.sample_rate().0;
        let source_channels = supported_config.channels() as usize;
        let target_sample_rate = self.config.sample_rate;
        let sample_format = supported_config.sample_format();

        tracing::debug!(
            "Device config: {} Hz, {} channel(s), format: {:?}",
            source_sample_rate,
            source_channels,
            sample_format
        );

        // Create channels
        let (chunk_tx, chunk_rx) = mpsc::channel(64);
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<CaptureCommand>();

        // Shared state
        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
        let samples_clone = samples.clone();

        // Spawn audio capture thread
        let thread_handle = thread::spawn(move || {
            // Build stream config
            let stream_config = cpal::StreamConfig {
                channels: supported_config.channels(),
                sample_rate: supported_config.sample_rate(),
                buffer_size: cpal::BufferSize::Default,
            };

            let err_fn = |err| tracing::error!("Audio stream error: {}", err);

            // Create the input stream based on sample format
            let make_params = || StreamBuildParams {
                samples: samples_clone.clone(),
                tx: chunk_tx.clone(),
                source_rate: source_sample_rate,
                target_rate: target_sample_rate,
                source_channels,
            };

            let stream_result = match sample_format {
                cpal::SampleFormat::F32 => {
                    build_stream::<f32>(&device, &stream_config, make_params(), err_fn)
                }
                cpal::SampleFormat::I16 => {
                    build_stream::<i16>(&device, &stream_config, make_params(), err_fn)
                }
                cpal::SampleFormat::U16 => {
                    build_stream::<u16>(&device, &stream_config, make_params(), err_fn)
                }
                format => {
                    tracing::error!("Unsupported sample format: {:?}", format);
                    return;
                }
            };

            let stream = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Failed to build audio stream: {}", e);
                    return;
                }
            };

            if let Err(e) = stream.play() {
                tracing::error!("Failed to start audio stream: {}", e);
                return;
            }

            tracing::debug!("Audio capture thread started");

            // Wait for stop command
            if let Ok(CaptureCommand::Stop(response_tx)) = cmd_rx.recv() {
                // Stop the stream (drop it)
                drop(stream);

                // Get collected samples
                let collected = {
                    let guard = samples_clone.lock().unwrap();
                    guard.clone()
                };

                // Send samples back
                let _ = response_tx.send(collected);
            }

            tracing::debug!("Audio capture thread stopped");
        });

        self.cmd_tx = Some(cmd_tx);
        self.thread_handle = Some(thread_handle);

        Ok(chunk_rx)
    }

    async fn stop(&mut self) -> Result<Vec<f32>, AudioError> {
        // Send stop command and get samples back
        let samples = if let Some(cmd_tx) = self.cmd_tx.take() {
            let (response_tx, response_rx) = oneshot::channel();

            if cmd_tx.send(CaptureCommand::Stop(response_tx)).is_ok() {
                // Wait for response (with timeout)
                match tokio::time::timeout(std::time::Duration::from_secs(2), response_rx).await {
                    Ok(Ok(samples)) => samples,
                    Ok(Err(_)) => {
                        return Err(AudioError::StreamError("Channel closed".to_string()))
                    }
                    Err(_) => return Err(AudioError::Timeout(2)),
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // Wait for thread to finish
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }

        let duration_secs = samples.len() as f32 / self.config.sample_rate as f32;
        tracing::debug!(
            "Audio capture stopped: {} samples ({:.2}s)",
            samples.len(),
            duration_secs
        );

        if samples.is_empty() {
            return Err(AudioError::EmptyRecording);
        }

        Ok(samples)
    }
}

/// Build an input stream for a specific sample type
fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    params: StreamBuildParams,
    err_fn: impl Fn(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, AudioError>
where
    T: cpal::Sample + cpal::SizedSample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    use cpal::traits::DeviceTrait;

    let StreamBuildParams {
        samples,
        tx,
        source_rate,
        target_rate,
        source_channels,
    } = params;

    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                // Convert to f32 and mix to mono
                let mono_f32: Vec<f32> = data
                    .chunks(source_channels)
                    .map(|frame| {
                        let sum: f32 = frame
                            .iter()
                            .map(|&s| <f32 as cpal::FromSample<T>>::from_sample_(s))
                            .sum();
                        sum / source_channels as f32
                    })
                    .collect();

                // Resample if needed
                let resampled = if source_rate != target_rate {
                    resample(&mono_f32, source_rate, target_rate)
                } else {
                    mono_f32
                };

                // Store samples
                if let Ok(mut guard) = samples.lock() {
                    guard.extend_from_slice(&resampled);
                }

                // Send chunk for streaming (ignore errors - receiver might be gone)
                let _ = tx.try_send(resampled);
            },
            err_fn,
            None,
        )
        .map_err(|e| AudioError::StreamError(e.to_string()))?;

    Ok(stream)
}

/// Linear interpolation resampling
/// For better quality, consider using the `rubato` crate
fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = (samples.len() as f64 * ratio).ceil() as usize;
    let mut output = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let src_idx = i as f64 / ratio;
        let idx = src_idx.floor() as usize;
        let frac = (src_idx - idx as f64) as f32;

        let sample = if idx + 1 < samples.len() {
            samples[idx] * (1.0 - frac) + samples[idx + 1] * frac
        } else {
            samples.get(idx).copied().unwrap_or(0.0)
        };

        output.push(sample);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resample_same_rate() {
        let samples = vec![1.0, 2.0, 3.0, 4.0];
        let result = resample(&samples, 16000, 16000);
        assert_eq!(result, samples);
    }

    #[test]
    fn test_resample_downsample() {
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let result = resample(&samples, 48000, 16000);
        // 48000 -> 16000 is 3:1 ratio, so 8 samples -> ~3 samples
        assert!(result.len() >= 2 && result.len() <= 4);
    }

    #[test]
    fn test_resample_upsample() {
        let samples = vec![1.0, 2.0];
        let result = resample(&samples, 8000, 16000);
        // 8000 -> 16000 is 1:2 ratio, so 2 samples -> 4 samples
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn test_resample_empty() {
        let samples: Vec<f32> = vec![];
        let result = resample(&samples, 48000, 16000);
        assert!(result.is_empty());
    }
}
