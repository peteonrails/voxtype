//! cpal input streams with a separate, per-recording sample buffer.
//!
//! The daemon may hold an idle stream open to avoid device wake-up latency.
//! Idle callbacks return before conversion or buffering. cpal::Stream stays
//! on its own thread because it is not Send on every platform.

use super::AudioCapture;
use crate::config::AudioConfig;
use crate::error::AudioError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::{mpsc, oneshot};

/// Owns the daemon's optional idle stream. Recording handles borrow its stream,
/// but each gets a fresh buffer, channel, and resampler.
pub struct CaptureFactory {
    config: AudioConfig,
    ready: Option<Arc<InputStream>>,
    suspended: bool,
    #[cfg(test)]
    test_opener: Option<TestOpener>,
}

impl CaptureFactory {
    /// Construct a factory without opening the device. Call `prepare` to
    /// eagerly open it when readiness is enabled.
    pub fn new(config: &AudioConfig) -> Self {
        Self {
            config: config.clone(),
            ready: None,
            suspended: false,
            #[cfg(test)]
            test_opener: None,
        }
    }

    /// Keep one healthy idle stream only when enabled and not suspended.
    /// Opening failures are returned; a later call retries. Idle audio is discarded.
    pub async fn prepare(&mut self) -> Result<(), AudioError> {
        if self.config.keep_ready
            && !self.suspended
            && self.ready.as_ref().is_none_or(|s| s.is_dead())
        {
            // Release a failed stream before reopening an exclusive device.
            self.ready = None;
            self.ready = Some(self.open_input().await?);
            tracing::info!("Microphone kept ready; idle audio is discarded");
        }
        Ok(())
    }

    /// Meeting mode owns its microphone separately. Release our idle stream
    /// so exclusive devices remain available until the meeting ends.
    pub fn set_suspended(&mut self, suspended: bool) {
        self.suspended = suspended;
        if suspended {
            self.ready = None;
        }
    }

    /// Create an unstarted recording handle, sharing the ready stream when
    /// enabled. Each started handle owns fresh samples and a resampler;
    /// otherwise the device opens on `AudioCapture::start`.
    pub async fn create_capture(&mut self) -> Result<Box<dyn AudioCapture>, AudioError> {
        self.prepare().await?;
        Ok(Box::new(CpalCapture {
            config: self.config.clone(),
            input: self.ready.clone(),
            recording: false,
        }))
    }

    async fn open_input(&self) -> Result<Arc<InputStream>, AudioError> {
        #[cfg(test)]
        if let Some(opener) = &self.test_opener {
            opener().await?;
            return Ok(fake_input());
        }
        InputStream::open(&self.config).await
    }

    #[cfg(test)]
    pub(crate) fn set_test_opener<F, Fut>(&mut self, opener: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), AudioError>> + Send + 'static,
    {
        self.test_opener = Some(Arc::new(move || Box::pin(opener())));
    }
}

/// Only exists during an explicit recording. Taking it out of the shared
/// slot closes the chunk channel and makes the next callback discard input.
struct RecordingSession {
    samples: Vec<f32>,
    tx: mpsc::Sender<Vec<f32>>,
    resampler: super::resampler::StreamResampler,
}

impl RecordingSession {
    fn new(
        source_rate: u32,
        target_rate: u32,
    ) -> Result<(Self, mpsc::Receiver<Vec<f32>>), AudioError> {
        let (tx, rx) = mpsc::channel(64);
        let resampler = super::resampler::StreamResampler::new(source_rate, target_rate)
            .map_err(AudioError::StreamError)?;
        Ok((
            Self {
                samples: Vec::new(),
                tx,
                resampler,
            },
            rx,
        ))
    }

    fn push<T>(&mut self, data: &[T], channels: usize)
    where
        T: cpal::Sample,
        f32: cpal::FromSample<T>,
    {
        let mono: Vec<f32> = data
            .chunks(channels)
            .map(|frame| {
                frame
                    .iter()
                    .map(|&s| <f32 as cpal::FromSample<T>>::from_sample_(s))
                    .sum::<f32>()
                    / channels as f32
            })
            .collect();
        let converted = self.resampler.push(&mono);
        self.samples.extend_from_slice(&converted);
        let _ = self.tx.try_send(converted);
    }

    fn finish(mut self) -> Vec<f32> {
        self.samples.extend(self.resampler.flush());
        self.samples
    }
}

type RecordingSlot = Arc<Mutex<Option<RecordingSession>>>;

struct InputStream {
    session: RecordingSlot,
    source_rate: u32,
    dead: Arc<AtomicBool>,
    shutdown: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl InputStream {
    async fn open(config: &AudioConfig) -> Result<Arc<Self>, AudioError> {
        let config = config.clone();
        let session: RecordingSlot = Arc::new(Mutex::new(None));
        let session_for_thread = session.clone();
        let dead = Arc::new(AtomicBool::new(false));
        let dead_for_thread = dead.clone();
        let (shutdown, shutdown_rx) = std::sync::mpsc::channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        let thread = thread::spawn(move || {
            use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
            let opened = (|| {
                let host = cpal::default_host();
                let device = if config.device == "default" {
                    host.default_input_device()
                        .ok_or_else(|| AudioError::DeviceNotFound("default".into()))?
                } else {
                    find_audio_device(&host, &config.device)?
                };
                tracing::info!(
                    "Using audio device: {}",
                    device.name().unwrap_or_else(|_| "unknown".into())
                );
                let supported = device
                    .default_input_config()
                    .map_err(|e| AudioError::Connection(e.to_string()))?;
                let stream_config = cpal::StreamConfig {
                    channels: supported.channels(),
                    sample_rate: supported.sample_rate(),
                    buffer_size: cpal::BufferSize::Default,
                };
                let err_fn = move |err| {
                    tracing::error!("Audio stream error, capture is now dead: {}", err);
                    dead_for_thread.store(true, Ordering::SeqCst);
                };
                let stream = match supported.sample_format() {
                    cpal::SampleFormat::F32 => {
                        build_stream::<f32>(&device, &stream_config, session_for_thread, err_fn)
                    }
                    cpal::SampleFormat::I16 => {
                        build_stream::<i16>(&device, &stream_config, session_for_thread, err_fn)
                    }
                    cpal::SampleFormat::U16 => {
                        build_stream::<u16>(&device, &stream_config, session_for_thread, err_fn)
                    }
                    format => Err(AudioError::StreamError(format!(
                        "Unsupported sample format: {format:?}"
                    ))),
                }?;
                stream
                    .play()
                    .map_err(|e| AudioError::StreamError(e.to_string()))?;
                Ok::<_, AudioError>((stream, supported.sample_rate().0))
            })();
            match opened {
                Ok((stream, source_rate)) => {
                    if ready_tx.send(Ok(source_rate)).is_ok() {
                        let _ = shutdown_rx.recv();
                    }
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        // Dropping this future closes shutdown even if device setup is still
        // running, so a cancelled/timed-out startup cannot leave an idle mic.
        let source_rate = tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| AudioError::Timeout(5))?
            .map_err(|_| {
                AudioError::StreamError("Audio input thread exited during startup".into())
            })??;
        Ok(Arc::new(Self {
            session,
            source_rate,
            dead,
            shutdown: Some(shutdown),
            thread: Some(thread),
        }))
    }

    fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst) || self.thread.as_ref().is_some_and(|t| t.is_finished())
    }
}

impl Drop for InputStream {
    fn drop(&mut self) {
        self.shutdown.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct CpalCapture {
    config: AudioConfig,
    input: Option<Arc<InputStream>>,
    recording: bool,
}

impl CpalCapture {
    pub fn new(config: &AudioConfig) -> Result<Self, AudioError> {
        Ok(Self {
            config: config.clone(),
            input: None,
            recording: false,
        })
    }
}

#[async_trait::async_trait]
impl AudioCapture for CpalCapture {
    async fn start(&mut self) -> Result<mpsc::Receiver<Vec<f32>>, AudioError> {
        if self.recording {
            return Err(AudioError::StreamError("Recording already started".into()));
        }
        if self.input.as_ref().is_none_or(|s| s.is_dead()) {
            self.input = None;
            self.input = Some(InputStream::open(&self.config).await?);
        }
        let input = self.input.as_ref().unwrap();
        let (session, rx) = RecordingSession::new(input.source_rate, self.config.sample_rate)?;
        let mut slot = input.session.lock().unwrap();
        if slot.is_some() {
            return Err(AudioError::StreamError(
                "Microphone already recording".into(),
            ));
        }
        *slot = Some(session);
        self.recording = true;
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<Vec<f32>, AudioError> {
        let session = if self.recording {
            self.recording = false;
            self.input.as_ref().and_then(|input| {
                if input.is_dead() {
                    tracing::error!("Audio device failed during this recording; the transcript may be incomplete. Reconnect the device and record again.");
                }
                input.session.lock().unwrap().take()
            })
        } else {
            None
        };
        // The factory holds its own reference only when keep_ready is enabled.
        // Otherwise this releases the device before returning the recording.
        self.input = None;
        let samples = session.map(RecordingSession::finish).unwrap_or_default();
        tracing::debug!(
            "Audio capture stopped: {} samples ({:.2}s)",
            samples.len(),
            samples.len() as f32 / self.config.sample_rate as f32
        );
        if samples.is_empty() {
            Err(AudioError::EmptyRecording)
        } else {
            Ok(samples)
        }
    }

    async fn get_samples(&mut self) -> Vec<f32> {
        if !self.recording {
            return Vec::new();
        }
        self.input
            .as_ref()
            .and_then(|input| {
                let mut slot = input.session.lock().unwrap();
                slot.as_mut().map(|s| std::mem::take(&mut s.samples))
            })
            .unwrap_or_default()
    }
}

impl Drop for CpalCapture {
    fn drop(&mut self) {
        if self.recording {
            if let Some(input) = &self.input {
                // Also close the gate on cancellation/error paths which drop
                // a handle without calling stop(). Do not retain that audio.
                input.session.lock().unwrap().take();
            }
        }
    }
}

fn capture_input<T>(slot: &RecordingSlot, data: &[T], channels: usize)
where
    T: cpal::Sample,
    f32: cpal::FromSample<T>,
{
    let mut guard = slot.lock().unwrap();
    if let Some(session) = guard.as_mut() {
        session.push(data, channels);
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    session: RecordingSlot,
    err_fn: impl Fn(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, AudioError>
where
    T: cpal::Sample + cpal::SizedSample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    use cpal::traits::DeviceTrait;
    let channels = config.channels as usize;
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| capture_input(&session, data, channels),
            err_fn,
            None,
        )
        .map_err(|e| AudioError::StreamError(e.to_string()))
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

#[cfg(test)]
type TestOpener = Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AudioError>> + Send>>
        + Send
        + Sync,
>;

#[cfg(test)]
fn fake_input() -> Arc<InputStream> {
    Arc::new(InputStream {
        session: Arc::new(Mutex::new(None)),
        source_rate: 16000,
        dead: Arc::new(AtomicBool::new(false)),
        shutdown: None,
        thread: None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::resampler::resample_buffer;
    use super::*;

    #[tokio::test]
    async fn enabled_readiness_reuses_recovers_and_releases_without_hardware() {
        use std::sync::atomic::AtomicUsize;
        let opened = Arc::new(AtomicUsize::new(0));
        let count = opened.clone();
        let mut factory = CaptureFactory::new(&AudioConfig {
            keep_ready: true,
            ..AudioConfig::default()
        });
        factory.set_test_opener(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        });
        factory.prepare().await.unwrap();
        let input = factory.ready.as_ref().unwrap().clone();
        for _ in 0..2 {
            let mut capture = factory.create_capture().await.unwrap();
            let _rx = capture.start().await.unwrap();
            capture_input(&input.session, &[0.2_f32; 160], 1);
            assert_eq!(capture.stop().await.unwrap(), vec![0.2; 160]);
            assert!(input.session.lock().unwrap().is_none());
        }
        assert_eq!(opened.load(Ordering::SeqCst), 1);
        let old = Arc::downgrade(&input);
        input.dead.store(true, Ordering::SeqCst);
        drop(input);
        let capture = factory.create_capture().await.unwrap();
        assert!(old.upgrade().is_none());
        assert_eq!(opened.load(Ordering::SeqCst), 2);
        drop(capture);
        let ready = Arc::downgrade(factory.ready.as_ref().unwrap());
        factory.set_suspended(true);
        assert!(
            ready.upgrade().is_none(),
            "meeting must release the idle device"
        );
        factory.prepare().await.unwrap();
        assert!(factory.ready.is_none());
        assert_eq!(opened.load(Ordering::SeqCst), 2);
        factory.set_suspended(false);
        factory.prepare().await.unwrap();
        assert!(factory.ready.is_some());
        assert_eq!(opened.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn failed_ready_open_can_retry_and_pending_open_can_be_cancelled() {
        let mut factory = CaptureFactory::new(&AudioConfig {
            keep_ready: true,
            ..AudioConfig::default()
        });
        factory.set_test_opener(|| async { Err(AudioError::Timeout(5)) });
        assert!(factory.prepare().await.is_err());
        assert!(factory.ready.is_none());
        factory.set_test_opener(std::future::pending);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), factory.prepare())
                .await
                .is_err()
        );
        assert!(factory.ready.is_none());
        factory.set_test_opener(|| async { Ok(()) });
        factory.prepare().await.unwrap();
        assert!(factory.ready.is_some());
    }

    #[tokio::test]
    async fn readiness_off_does_not_open_a_device_before_recording() {
        let config = AudioConfig {
            device: "nonexistent-device-for-readiness-test".into(),
            ..AudioConfig::default()
        };
        let mut factory = CaptureFactory::new(&config);
        // An unavailable device must not affect daemon startup or creating
        // a recording handle when readiness is disabled.
        factory.prepare().await.unwrap();
        let _capture = factory.create_capture().await.unwrap();
        assert!(factory.ready.is_none());
    }

    #[tokio::test]
    async fn idle_audio_is_discarded_and_recordings_are_isolated() {
        let input = fake_input();
        // Idle callbacks must not retain audio or create a recording buffer.
        capture_input(&input.session, &[0.9_f32; 4096], 1);
        assert!(input.session.lock().unwrap().is_none());
        let mut capture = CpalCapture {
            config: AudioConfig::default(),
            input: Some(input.clone()),
            recording: false,
        };
        let mut chunks = capture.start().await.unwrap();
        assert!(capture.start().await.is_err());
        capture_input(&input.session, &[0.2_f32, 0.4, 0.6, 0.8], 2);
        let recorded = capture.stop().await.unwrap();
        assert_eq!(recorded.len(), 2);
        assert!((recorded[0] - 0.3).abs() < 0.0001);
        assert_eq!(chunks.recv().await.unwrap(), recorded);
        assert!(chunks.recv().await.is_none());
        assert!(input.session.lock().unwrap().is_none());

        capture_input(&input.session, &[0.9_f32; 4096], 1);
        capture.input = Some(input.clone());
        let mut next_chunks = capture.start().await.unwrap();
        capture_input(&input.session, &[0.1_f32; 4], 1);
        assert_eq!(capture.get_samples().await, vec![0.1; 4]);
        assert!(capture.get_samples().await.is_empty());
        capture_input(&input.session, &[0.2_f32; 3], 1);
        assert_eq!(capture.stop().await.unwrap(), vec![0.2; 3]);
        assert_eq!(next_chunks.recv().await.unwrap(), vec![0.1; 4]);
        assert_eq!(next_chunks.recv().await.unwrap(), vec![0.2; 3]);
        assert!(next_chunks.recv().await.is_none());
    }

    #[test]
    fn resampler_tail_is_flushed_and_does_not_cross_recordings() {
        let slot: RecordingSlot = Arc::new(Mutex::new(None));
        let (session, _rx) = RecordingSession::new(48000, 16000).unwrap();
        *slot.lock().unwrap() = Some(session);
        capture_input(&slot, &[0.75_f32; 300], 1);
        let first = slot.lock().unwrap().take().unwrap().finish();
        assert_eq!(first.len(), 100);
        capture_input(&slot, &[0.9_f32; 4096], 1);
        let (session, _rx) = RecordingSession::new(48000, 16000).unwrap();
        *slot.lock().unwrap() = Some(session);
        capture_input(&slot, &[0.0_f32; 300], 1);
        let second = slot.lock().unwrap().take().unwrap().finish();
        assert_eq!(second, vec![0.0; 100]);
    }

    #[tokio::test]
    async fn dropping_a_recording_closes_the_gate_and_chunk_channel() {
        let input = fake_input();
        let mut capture = CpalCapture {
            config: AudioConfig::default(),
            input: Some(input.clone()),
            recording: false,
        };
        let mut chunks = capture.start().await.unwrap();
        drop(capture);
        assert!(input.session.lock().unwrap().is_none());
        capture_input(&input.session, &[0.9_f32; 4096], 1);
        assert!(chunks.recv().await.is_none());
    }

    #[tokio::test]
    async fn an_unstarted_handle_cannot_stop_another_recording() {
        let input = fake_input();
        let mut first = CpalCapture {
            config: AudioConfig::default(),
            input: Some(input.clone()),
            recording: false,
        };
        let _rx = first.start().await.unwrap();
        let mut second = CpalCapture {
            config: AudioConfig::default(),
            input: Some(input.clone()),
            recording: false,
        };
        assert!(second.start().await.is_err());
        assert!(second.stop().await.is_err());
        drop(second);
        assert!(input.session.lock().unwrap().is_some());
    }

    /// Run explicitly on a machine with an available microphone. No audio is
    /// written to disk or sent for transcription.
    #[tokio::test]
    #[ignore = "requires a microphone"]
    async fn ready_microphone_reuses_the_real_input_stream() {
        let config = AudioConfig {
            keep_ready: true,
            ..AudioConfig::default()
        };
        let mut factory = CaptureFactory::new(&config);
        factory.prepare().await.unwrap();
        let input = factory.ready.as_ref().unwrap().clone();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        assert!(input.session.lock().unwrap().is_none());
        for _ in 0..2 {
            let mut capture = factory.create_capture().await.unwrap();
            let mut chunks = capture.start().await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while chunks.recv().await.unwrap().is_empty() {}
            })
            .await
            .unwrap();
            assert!(!capture.stop().await.unwrap().is_empty());
            assert!(Arc::ptr_eq(factory.ready.as_ref().unwrap(), &input));
            assert!(input.session.lock().unwrap().is_none());
        }
        // Simulate the CPAL terminal-error notification; the factory must
        // open a new stream on the next attempt instead of reusing a dead one.
        input.dead.store(true, Ordering::SeqCst);
        drop(input);
        factory.prepare().await.unwrap();
        assert!(!factory.ready.as_ref().unwrap().is_dead());
        assert!(factory
            .ready
            .as_ref()
            .unwrap()
            .session
            .lock()
            .unwrap()
            .is_none());

        let previous = Arc::downgrade(factory.ready.as_ref().unwrap());
        factory.set_suspended(true);
        assert!(
            previous.upgrade().is_none(),
            "suspending must release the device"
        );
        factory.prepare().await.unwrap();
        assert!(
            factory.ready.is_none(),
            "meeting mode must not reopen the idle mic"
        );
        factory.set_suspended(false);
        factory.prepare().await.unwrap();
        assert!(factory.ready.is_some());
    }

    // These moved off the removed linear-interpolation `resample()` and onto
    // the band-limited path (#641). They cover very short inputs, which is a
    // real case: a recording shorter than one FFT chunk still has to come out
    // the other side.

    #[test]
    fn test_resample_same_rate() {
        let samples = vec![1.0, 2.0, 3.0, 4.0];
        let result = resample_buffer(&samples, 16000, 16000);
        assert_eq!(result, samples);
    }

    #[test]
    fn test_resample_downsample() {
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let result = resample_buffer(&samples, 48000, 16000);
        // 48000 -> 16000 is 3:1, so 8 samples land around 3.
        assert!(
            result.len() >= 2 && result.len() <= 4,
            "expected about 3 samples, got {}",
            result.len()
        );
    }

    #[test]
    fn test_resample_upsample() {
        let samples = vec![1.0, 2.0];
        let result = resample_buffer(&samples, 8000, 16000);
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn test_resample_empty() {
        let samples: Vec<f32> = vec![];
        assert!(resample_buffer(&samples, 48000, 16000).is_empty());
    }
}
