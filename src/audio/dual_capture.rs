//! Dual audio capture for meeting mode
//!
//! Captures both microphone input (user's voice) and system audio loopback
//! (remote participants) simultaneously for speaker attribution.
//!
//! Mic capture uses cpal (ALSA). Loopback capture uses `parec` (PulseAudio
//! recording client) which works with PipeWire's PulseAudio compatibility
//! layer and can access monitor sources that aren't visible to ALSA. While
//! the meeting echo canceller runs (`audio::echo_cancel`), the mic is
//! recorded with `parec` too, from the canceller's source, which ALSA
//! cannot name either.

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

/// Capture from a PulseAudio/PipeWire source via a parec subprocess
struct ParecCapture {
    /// Source name (PulseAudio/PipeWire source)
    source: String,
    /// Samples to drop from the start, to line this track up with another
    skip: usize,
    /// Child process
    child: Option<std::process::Child>,
    /// Shared buffer for received samples
    buffer: Arc<Mutex<Vec<f32>>>,
    /// Reader thread handle
    reader_thread: Option<std::thread::JoinHandle<()>>,
    /// Fed by the reader thread, for meeting.sock's level events
    meter: Arc<crate::meeting::levels::TrackMeter>,
}

impl ParecCapture {
    fn new(source: String, meter: Arc<crate::meeting::levels::TrackMeter>) -> Self {
        Self::skipping(source, 0, meter)
    }

    /// Capture `source`, dropping its first `skip` samples.
    fn skipping(
        source: String,
        skip: usize,
        meter: Arc<crate::meeting::levels::TrackMeter>,
    ) -> Self {
        Self {
            source,
            skip,
            child: None,
            buffer: Arc::new(Mutex::new(Vec::new())),
            reader_thread: None,
            meter,
        }
    }

    fn start(&mut self) -> Result<(), AudioError> {
        let mut child = std::process::Command::new("parec")
            .args([
                "--device",
                &self.source,
                "--format=float32le",
                "--channels=1",
                "--rate=16000",
                "--latency-msec=100",
                "--raw",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| AudioError::Connection(format!("Failed to start parec: {}", e)))?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| AudioError::Connection("Failed to capture parec stdout".to_string()))?;

        self.child = Some(child);
        tracing::info!("Capture started via parec: {}", self.source);

        // Spawn reader thread
        let buffer = Arc::clone(&self.buffer);
        let meter = Arc::clone(&self.meter);
        let mut skip = self.skip;
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
                        let dropped = skip.min(samples.len());
                        skip -= dropped;
                        meter.feed(&samples[dropped..]);
                        if let Ok(mut buf) = buffer.lock() {
                            buf.extend(&samples[dropped..]);
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

impl Drop for ParecCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Where the microphone is recorded from.
enum MicCapture {
    /// The configured device, via cpal/ALSA
    Cpal(CpalCapture),
    /// A PipeWire/PulseAudio source, via parec (the echo canceller's)
    Parec(ParecCapture),
}

/// Dual audio capture for mic + loopback
pub struct DualCapture {
    /// Microphone capture
    mic_capture: MicCapture,
    /// Loopback capture (via parec subprocess)
    loopback: Option<ParecCapture>,
    /// Sample counter for timestamps
    sample_counter: u64,
    /// Per-track levels for meeting.sock, fed as the audio arrives
    meters: crate::meeting::levels::MeetingMeters,
    /// Feeds the microphone meter from the capture's chunk stream
    mic_meter_task: Option<tokio::task::JoinHandle<()>>,
}

impl DualCapture {
    /// Create a new dual capture instance
    pub fn new(
        mic_config: &AudioConfig,
        loopback_device: Option<&str>,
    ) -> Result<Self, AudioError> {
        Self::with_mic(
            MicCapture::Cpal(CpalCapture::new(mic_config)?),
            loopback_device,
            crate::meeting::levels::MeetingMeters::new(true),
        )
    }

    /// Like [`DualCapture::new`], feeding `meters`: another capture's, so
    /// level events that follow it carry on when this one replaces it.
    pub fn new_metered(
        mic_config: &AudioConfig,
        loopback_device: Option<&str>,
        mut meters: crate::meeting::levels::MeetingMeters,
    ) -> Result<Self, AudioError> {
        if meters.loopback.is_none() {
            meters.loopback = Some(crate::meeting::levels::TrackMeter::new());
        }
        Self::with_mic(
            MicCapture::Cpal(CpalCapture::new(mic_config)?),
            loopback_device,
            meters,
        )
    }

    /// Like [`DualCapture::new`], recording the mic from the PipeWire or
    /// PulseAudio source `mic_source` (the echo canceller's) with parec.
    /// The first `delay_ms` of it are dropped: the canceller delivers the
    /// mic that much later than the loopback, which comes straight from the
    /// sink's monitor, and dropping it puts both tracks on one timeline.
    pub fn with_mic_source(
        mic_source: &str,
        loopback_device: Option<&str>,
        delay_ms: u32,
    ) -> Result<Self, AudioError> {
        let meters = crate::meeting::levels::MeetingMeters::new(true);
        Self::with_mic(
            MicCapture::Parec(ParecCapture::skipping(
                mic_source.to_string(),
                16 * delay_ms as usize,
                meters.mic.clone(),
            )),
            loopback_device,
            meters,
        )
    }

    fn with_mic(
        mic_capture: MicCapture,
        loopback_device: Option<&str>,
        mut meters: crate::meeting::levels::MeetingMeters,
    ) -> Result<Self, AudioError> {
        let loopback_meter = meters
            .loopback
            .clone()
            .expect("created with a loopback meter");
        let loopback = match loopback_device {
            Some("disabled") | Some("") | None => None,
            Some("auto") => match Self::find_monitor_source() {
                Some(source) => {
                    tracing::info!("Auto-detected loopback source: {}", source);
                    Some(ParecCapture::new(source, loopback_meter))
                }
                None => {
                    tracing::warn!("No monitor source found, using mic only");
                    None
                }
            },
            Some(device) => {
                tracing::info!("Using configured loopback source: {}", device);
                Some(ParecCapture::new(device.to_string(), loopback_meter))
            }
        };
        if loopback.is_none() {
            meters.loopback = None;
        }

        Ok(Self {
            mic_capture,
            loopback,
            sample_counter: 0,
            meters,
            mic_meter_task: None,
        })
    }

    /// Find a PipeWire/PulseAudio monitor source via pactl.
    fn find_monitor_source() -> Option<String> {
        let sources = std::process::Command::new("pactl")
            .args(["list", "short", "sources"])
            .output()
            .ok()?;
        let default_sink = std::process::Command::new("pactl")
            .arg("get-default-sink")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty());
        pick_monitor_source(
            default_sink.as_deref(),
            &String::from_utf8_lossy(&sources.stdout),
        )
    }

    /// Check if loopback capture is active
    pub fn has_loopback(&self) -> bool {
        self.loopback.is_some()
    }

    /// Start both captures
    pub async fn start(&mut self) -> Result<(), AudioError> {
        match self.mic_capture {
            MicCapture::Cpal(ref mut mic) => {
                let mut mic_rx = mic.start().await?;
                // cpal already sends each block it records on this channel
                // (dropping blocks rather than waiting when it is full);
                // reading it here gives the microphone meter the same audio,
                // at no cost to the capture. The parec mic feeds its meter
                // from its reader thread.
                let mic_meter = self.meters.mic.clone();
                self.mic_meter_task = Some(tokio::spawn(async move {
                    while let Some(block) = mic_rx.recv().await {
                        mic_meter.feed(&block);
                    }
                }));
            }
            MicCapture::Parec(ref mut mic) => mic.start()?,
        }

        if let Some(ref mut loopback) = self.loopback {
            if let Err(e) = loopback.start() {
                tracing::warn!("Loopback capture failed, continuing with mic only: {}", e);
                self.loopback = None;
                self.meters.loopback = None;
            }
        }

        Ok(())
    }

    /// The meters of this capture, for the level publisher.
    pub fn meters(&self) -> crate::meeting::levels::MeetingMeters {
        self.meters.clone()
    }

    /// Stop both captures and return all samples
    pub async fn stop(&mut self) -> Result<DualSamples, AudioError> {
        if let Some(task) = self.mic_meter_task.take() {
            task.abort();
        }
        let mic_samples = match self.mic_capture {
            MicCapture::Cpal(ref mut mic) => mic.stop().await?,
            MicCapture::Parec(ref mut mic) => {
                let samples = mic.get_samples();
                mic.stop();
                samples
            }
        };

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
        let mic = match self.mic_capture {
            MicCapture::Cpal(ref mut mic) => mic.get_samples().await,
            MicCapture::Parec(ref mic) => mic.get_samples(),
        };

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

/// Choose the monitor to record the far side of a call from.
///
/// The default sink's monitor comes first, because that is where call audio
/// plays. Taking the first RUNNING monitor instead picked whichever output
/// happened to be active (an idle S/PDIF sink on a USB card) and recorded
/// silence while the call played on the speakers. A running monitor, then
/// any monitor, are fallbacks for servers without a default sink.
///
/// `sources` is `pactl list short sources` output: ID, NAME, DRIVER, FORMAT,
/// STATUS separated by tabs.
fn pick_monitor_source(default_sink: Option<&str>, sources: &str) -> Option<String> {
    let monitors: Vec<(&str, &str)> = sources
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let name = *fields.get(1)?;
            name.ends_with(".monitor")
                .then(|| (name, fields.get(4).copied().unwrap_or("")))
        })
        .collect();

    if let Some(sink) = default_sink {
        let wanted = format!("{sink}.monitor");
        if let Some((name, _)) = monitors.iter().find(|(name, _)| *name == wanted) {
            tracing::debug!("Using the default sink's monitor: {}", name);
            return Some(name.to_string());
        }
    }
    if let Some((name, _)) = monitors.iter().find(|(_, status)| *status == "RUNNING") {
        tracing::debug!("Found running monitor source: {}", name);
        return Some(name.to_string());
    }
    monitors.first().map(|(name, _)| name.to_string())
}

#[cfg(test)]
mod tests {
    const SOURCES: &str = "64\talsa_output.usb-Generic_USB_Audio-00.HiFi__SPDIF__sink.monitor\tPipeWire\ts16le 2ch 48000Hz\tIDLE
65\talsa_output.usb-Generic_USB_Audio-00.HiFi__Speaker__sink.monitor\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED
66\talsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink.monitor\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED
70\talsa_input.usb-Generic_USB_Audio-00.HiFi__Mic1__source\tPipeWire\ts32le 2ch 48000Hz\tRUNNING
";

    #[test]
    fn default_sink_monitor_wins_over_the_first_listed_monitor() {
        // Pete's machine on 2026-11-11: calls played on Speaker, auto picked
        // the S/PDIF monitor listed first, and the remote track was silent.
        assert_eq!(
            pick_monitor_source(
                Some("alsa_output.usb-Generic_USB_Audio-00.HiFi__Speaker__sink"),
                SOURCES
            )
            .as_deref(),
            Some("alsa_output.usb-Generic_USB_Audio-00.HiFi__Speaker__sink.monitor")
        );
    }

    #[test]
    fn default_sink_monitor_wins_over_a_running_one() {
        let sources = SOURCES.replace(
            "Headphones__sink.monitor\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED",
            "Headphones__sink.monitor\tPipeWire\ts32le 2ch 48000Hz\tRUNNING",
        );
        assert_eq!(
            pick_monitor_source(
                Some("alsa_output.usb-Generic_USB_Audio-00.HiFi__Speaker__sink"),
                &sources
            )
            .as_deref(),
            Some("alsa_output.usb-Generic_USB_Audio-00.HiFi__Speaker__sink.monitor")
        );
    }

    #[test]
    fn without_a_default_sink_a_running_monitor_then_any_monitor() {
        let running = SOURCES.replace(
            "Headphones__sink.monitor\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED",
            "Headphones__sink.monitor\tPipeWire\ts32le 2ch 48000Hz\tRUNNING",
        );
        assert_eq!(
            pick_monitor_source(None, &running).as_deref(),
            Some("alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink.monitor")
        );
        assert_eq!(
            pick_monitor_source(Some("gone_sink"), SOURCES).as_deref(),
            Some("alsa_output.usb-Generic_USB_Audio-00.HiFi__SPDIF__sink.monitor")
        );
    }

    #[test]
    fn input_sources_are_never_picked() {
        let only_mic = "70\talsa_input.usb-Mic__source\tPipeWire\ts32le 2ch 48000Hz\tRUNNING\n";
        assert_eq!(pick_monitor_source(None, only_mic), None);
    }

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
