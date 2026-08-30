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
use rubato::{FftFixedIn, Resampler};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::{mpsc, oneshot};

/// Input block size presented to the FFT resampler. This bounds buffering to
/// about 21-64 ms for the input rates Voxtype commonly encounters while
/// Rubato internally chooses exact FFT sizes for the rate ratio.
const RESAMPLER_INPUT_FRAMES: usize = 1024;
const RESAMPLER_SUB_CHUNKS: usize = 2;

/// Commands sent to the audio capture thread
enum CaptureCommand {
    Stop(oneshot::Sender<Result<Vec<f32>, AudioError>>),
    /// Get current samples and clear the buffer (for continuous recording)
    GetSamples(oneshot::Sender<Vec<f32>>),
}

/// Parameters for building an audio input stream
struct StreamBuildParams {
    pipeline: Arc<Mutex<CapturePipeline>>,
    tx: mpsc::Sender<Vec<f32>>,
    source_channels: usize,
}

/// Converts a continuous mono input stream without resetting filter state at
/// CPAL callback boundaries. A new instance is created for every recording,
/// so overlap samples can never leak into the next session.
struct BandlimitedResampler {
    processor: Option<FftFixedIn<f32>>,
    pending: Vec<f32>,
    source_rate: usize,
    target_rate: usize,
    delay_remaining: usize,
    input_frames: usize,
    output_frames: usize,
    finished: bool,
}

impl BandlimitedResampler {
    fn new(source_rate: u32, target_rate: u32) -> Result<Self, AudioError> {
        if source_rate == 0 || target_rate == 0 {
            return Err(AudioError::StreamError(
                "Sample rates must be greater than zero".to_string(),
            ));
        }

        let mut processor = if source_rate == target_rate {
            None
        } else {
            Some(
                FftFixedIn::<f32>::new(
                    source_rate as usize,
                    target_rate as usize,
                    RESAMPLER_INPUT_FRAMES,
                    RESAMPLER_SUB_CHUNKS,
                    1,
                )
                .map_err(|error| AudioError::StreamError(error.to_string()))?,
            )
        };
        let delay_remaining = processor
            .as_mut()
            .map(|resampler| resampler.output_delay())
            .unwrap_or(0);

        Ok(Self {
            processor,
            pending: Vec::with_capacity(RESAMPLER_INPUT_FRAMES * 2),
            source_rate: source_rate as usize,
            target_rate: target_rate as usize,
            delay_remaining,
            input_frames: 0,
            output_frames: 0,
            finished: false,
        })
    }

    /// Add an arbitrary input chunk and return all target-rate frames that
    /// became available. The initial FFT filter delay is removed once across
    /// the entire recording, not once per callback.
    fn push(&mut self, samples: &[f32]) -> Result<Vec<f32>, AudioError> {
        if self.finished {
            return Err(AudioError::StreamError(
                "Cannot add samples after resampling has finished".to_string(),
            ));
        }

        self.input_frames += samples.len();
        let Some(processor) = self.processor.as_mut() else {
            self.output_frames += samples.len();
            return Ok(samples.to_vec());
        };

        self.pending.extend_from_slice(samples);
        let mut consumed = 0;
        let mut output = Vec::new();
        let input_frames_next = processor.input_frames_next();

        while self.pending.len() - consumed >= input_frames_next {
            let input = [&self.pending[consumed..consumed + input_frames_next]];
            let processed = processor
                .process(&input, None)
                .map_err(|error| AudioError::StreamError(error.to_string()))?;
            Self::append_after_delay(&mut output, &processed[0], &mut self.delay_remaining);
            consumed += input_frames_next;
        }

        if consumed > 0 {
            self.pending.drain(..consumed);
        }
        self.output_frames += output.len();
        Ok(output)
    }

    /// Flush the partial input block and filter overlap, returning exactly the
    /// number of frames implied by the recording duration.
    fn finish(&mut self) -> Result<Vec<f32>, AudioError> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;

        let Some(processor) = self.processor.as_mut() else {
            return Ok(Vec::new());
        };

        let expected_frames =
            expected_output_frames(self.input_frames, self.source_rate, self.target_rate);
        let frames_needed = expected_frames.saturating_sub(self.output_frames);
        let mut output = Vec::with_capacity(frames_needed);

        if !self.pending.is_empty() {
            let input = [self.pending.as_slice()];
            let processed = processor
                .process_partial(Some(&input), None)
                .map_err(|error| AudioError::StreamError(error.to_string()))?;
            Self::append_after_delay(&mut output, &processed[0], &mut self.delay_remaining);
        }

        // When the source length lands exactly on an input block boundary,
        // or the final partial block does not contain the complete overlap,
        // feed zero padding until the delayed tail is available.
        while output.len() < frames_needed {
            let no_input: Option<&[&[f32]]> = None;
            let processed = processor
                .process_partial(no_input, None)
                .map_err(|error| AudioError::StreamError(error.to_string()))?;
            let before = output.len();
            Self::append_after_delay(&mut output, &processed[0], &mut self.delay_remaining);
            if output.len() == before {
                break;
            }
        }

        if output.len() < frames_needed {
            return Err(AudioError::StreamError(format!(
                "Resampler produced {} of {} required tail frames",
                output.len(),
                frames_needed
            )));
        }

        output.truncate(frames_needed);
        self.output_frames += output.len();
        self.pending.clear();
        processor.reset();
        Ok(output)
    }

    fn append_after_delay(output: &mut Vec<f32>, processed: &[f32], delay: &mut usize) {
        let skip = (*delay).min(processed.len());
        *delay -= skip;
        output.extend_from_slice(&processed[skip..]);
    }
}

/// Number of target frames needed to preserve the full input duration. Use
/// integer arithmetic so long recordings do not accumulate floating-point
/// rounding error.
// Keep compatibility with the project's Rust 1.70 MSRV. Integer `div_ceil`
// stabilized later.
#[allow(clippy::manual_div_ceil)]
fn expected_output_frames(input_frames: usize, source_rate: usize, target_rate: usize) -> usize {
    let numerator = input_frames as u128 * target_rate as u128;
    ((numerator + source_rate as u128 - 1) / source_rate as u128) as usize
}

struct CapturePipeline {
    resampler: BandlimitedResampler,
    samples: Vec<f32>,
    error: Option<String>,
}

impl CapturePipeline {
    fn new(source_rate: u32, target_rate: u32) -> Result<Self, AudioError> {
        Ok(Self {
            resampler: BandlimitedResampler::new(source_rate, target_rate)?,
            samples: Vec::new(),
            error: None,
        })
    }

    fn push(&mut self, input: &[f32]) -> Option<Vec<f32>> {
        if self.error.is_some() {
            return None;
        }

        match self.resampler.push(input) {
            Ok(output) => {
                self.samples.extend_from_slice(&output);
                Some(output)
            }
            Err(error) => {
                tracing::error!("Audio resampling failed: {}", error);
                self.error = Some(error.to_string());
                None
            }
        }
    }

    fn take_samples(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.samples)
    }

    fn finish(&mut self) -> Result<(Vec<f32>, Vec<f32>), AudioError> {
        if let Some(error) = self.error.take() {
            return Err(AudioError::StreamError(error));
        }

        let tail = self.resampler.finish()?;
        self.samples.extend_from_slice(&tail);
        Ok((self.take_samples(), tail))
    }
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

        // Shared resampling and collection state. The processor is created
        // once per recording so FFT overlap remains continuous across CPAL
        // callbacks but cannot cross a recording boundary.
        let pipeline = Arc::new(Mutex::new(CapturePipeline::new(
            source_sample_rate,
            target_sample_rate,
        )?));
        let pipeline_for_thread = Arc::clone(&pipeline);
        let tail_tx = chunk_tx.clone();

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
                pipeline: Arc::clone(&pipeline_for_thread),
                tx: chunk_tx.clone(),
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

            // Handle commands in a loop
            loop {
                match cmd_rx.recv() {
                    Ok(CaptureCommand::Stop(response_tx)) => {
                        // Stop the stream (drop it)
                        drop(stream);

                        // Flush the resampler's delayed tail before returning
                        // the final target-rate recording.
                        let collected = match pipeline_for_thread.lock() {
                            Ok(mut pipeline) => pipeline.finish(),
                            Err(_) => Err(AudioError::StreamError(
                                "Audio pipeline lock poisoned".to_string(),
                            )),
                        };

                        if let Ok((_, tail)) = &collected {
                            if !tail.is_empty() {
                                let _ = tail_tx.try_send(tail.clone());
                            }
                        }

                        // Send samples back
                        let _ = response_tx.send(collected.map(|(samples, _)| samples));
                        break;
                    }
                    Ok(CaptureCommand::GetSamples(response_tx)) => {
                        // Get and clear current samples (for continuous recording)
                        let samples = match pipeline_for_thread.lock() {
                            Ok(mut pipeline) => pipeline.take_samples(),
                            Err(_) => {
                                tracing::error!("Audio pipeline lock poisoned");
                                Vec::new()
                            }
                        };
                        let _ = response_tx.send(samples);
                    }
                    Err(_) => {
                        // Channel closed, exit thread
                        tracing::debug!("Command channel closed");
                        break;
                    }
                }
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
                    Ok(Ok(Ok(samples))) => samples,
                    Ok(Ok(Err(error))) => return Err(error),
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

    async fn get_samples(&mut self) -> Vec<f32> {
        // Get current samples without stopping
        if let Some(ref cmd_tx) = self.cmd_tx {
            let (response_tx, response_rx) = oneshot::channel();

            if cmd_tx.send(CaptureCommand::GetSamples(response_tx)).is_ok() {
                // Wait for response (with short timeout)
                match tokio::time::timeout(std::time::Duration::from_millis(500), response_rx).await
                {
                    Ok(Ok(samples)) => return samples,
                    Ok(Err(_)) => {
                        tracing::warn!("get_samples: channel closed");
                    }
                    Err(_) => {
                        tracing::warn!("get_samples: timeout");
                    }
                }
            }
        }
        Vec::new()
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
        pipeline,
        tx,
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

                // Preserve filter state across callback boundaries. This is
                // important because CPAL is free to vary callback sizes.
                let resampled = match pipeline.lock() {
                    Ok(mut pipeline) => pipeline.push(&mono_f32),
                    Err(_) => {
                        tracing::error!("Audio pipeline lock poisoned");
                        None
                    }
                };

                // Send chunk for streaming (ignore errors - receiver might be gone)
                if let Some(resampled) = resampled {
                    if !resampled.is_empty() {
                        let _ = tx.try_send(resampled);
                    }
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| AudioError::StreamError(e.to_string()))?;

    Ok(stream)
}

#[cfg(test)]
fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>, AudioError> {
    let mut resampler = BandlimitedResampler::new(from_rate, to_rate)?;
    let mut output = resampler.push(samples)?;
    output.extend(resampler.finish()?);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resample_same_rate() {
        let samples = vec![1.0, 2.0, 3.0, 4.0];
        let result = resample(&samples, 16000, 16000).unwrap();
        assert_eq!(result, samples);
    }

    #[test]
    fn test_resample_downsample() {
        let samples = vec![0.0; 48_000];
        let result = resample(&samples, 48_000, 16_000).unwrap();
        assert_eq!(result.len(), 16_000);
    }

    #[test]
    fn test_resample_downsample_44100_has_exact_duration() {
        let samples = vec![0.0; 44_100];
        let result = resample(&samples, 44_100, 16_000).unwrap();
        assert_eq!(result.len(), 16_000);
    }

    #[test]
    fn test_resample_upsample() {
        let samples = vec![0.0; 8_000];
        let result = resample(&samples, 8_000, 16_000).unwrap();
        assert_eq!(result.len(), 16_000);
    }

    #[test]
    fn test_resample_empty() {
        let samples: Vec<f32> = vec![];
        let result = resample(&samples, 48000, 16000).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_resample_very_short_clip_preserves_duration() {
        let samples = vec![1.0; 7];
        let result = resample(&samples, 48_000, 16_000).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_resample_suppresses_frequencies_above_target_nyquist() {
        let source_rate = 48_000;
        let samples: Vec<f32> = (0..source_rate)
            .map(|sample| {
                (2.0 * std::f32::consts::PI * 12_000.0 * sample as f32 / source_rate as f32).sin()
            })
            .collect();

        let result = resample(&samples, source_rate, 16_000).unwrap();
        let steady_state = &result[1_000..result.len() - 1_000];
        let rms = (steady_state
            .iter()
            .map(|sample| sample * sample)
            .sum::<f32>()
            / steady_state.len() as f32)
            .sqrt();

        assert!(rms < 0.01, "out-of-band RMS was {rms}");
    }

    #[test]
    fn test_resample_preserves_speech_band_amplitude() {
        let source_rate = 48_000;
        let samples: Vec<f32> = (0..source_rate)
            .map(|sample| {
                (2.0 * std::f32::consts::PI * 1_000.0 * sample as f32 / source_rate as f32).sin()
            })
            .collect();

        let result = resample(&samples, source_rate, 16_000).unwrap();
        let steady_state = &result[1_000..result.len() - 1_000];
        let rms = (steady_state
            .iter()
            .map(|sample| sample * sample)
            .sum::<f32>()
            / steady_state.len() as f32)
            .sqrt();

        assert!((rms - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
    }

    #[test]
    fn test_resample_is_independent_of_callback_boundaries() {
        let source_rate = 44_100;
        let samples: Vec<f32> = (0..source_rate)
            .map(|sample| {
                (2.0 * std::f32::consts::PI * 1_300.0 * sample as f32 / source_rate as f32).sin()
            })
            .collect();
        let expected = resample(&samples, source_rate, 16_000).unwrap();

        let mut streaming = BandlimitedResampler::new(source_rate, 16_000).unwrap();
        let mut actual = Vec::new();
        let chunk_sizes = [1, 127, 480, 997, 64, 2_048];
        let mut offset = 0;
        let mut chunk_index = 0;
        while offset < samples.len() {
            let end = (offset + chunk_sizes[chunk_index % chunk_sizes.len()]).min(samples.len());
            actual.extend(streaming.push(&samples[offset..end]).unwrap());
            offset = end;
            chunk_index += 1;
        }
        actual.extend(streaming.finish().unwrap());

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_resampler_state_does_not_cross_recordings() {
        let mut first = vec![0.0; 48_000];
        first[24_000] = 1.0;
        let first_output = resample(&first, 48_000, 16_000).unwrap();
        assert!(first_output.iter().any(|sample| sample.abs() > 0.01));

        let second = vec![0.0; 48_000];
        let second_output = resample(&second, 48_000, 16_000).unwrap();
        assert!(second_output.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn test_resample_keeps_audio_near_both_boundaries() {
        let mut samples = vec![0.0; 48_000];
        samples[300] = 1.0;
        samples[47_700] = 1.0;

        let result = resample(&samples, 48_000, 16_000).unwrap();
        assert!(result[..200].iter().any(|sample| sample.abs() > 0.01));
        assert!(result[result.len() - 200..]
            .iter()
            .any(|sample| sample.abs() > 0.01));
    }
}
