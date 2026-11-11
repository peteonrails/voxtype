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
                "--device",
                &self.source,
                "--format=float32le",
                "--channels=1",
                "--rate=16000",
                "--raw",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    AudioError::Connection(PAREC_NOT_FOUND.to_string())
                } else {
                    AudioError::Connection(format!("Failed to start parec: {}", e))
                }
            })?;

        // parec exits at once when it cannot reach a PulseAudio server or the
        // source does not exist. Give it a moment so that failure surfaces
        // here, at meeting start, rather than as a transcript with no remote
        // side.
        std::thread::sleep(std::time::Duration::from_millis(200));
        if let Ok(Some(status)) = child.try_wait() {
            return Err(AudioError::Connection(format!(
                "parec could not record from '{}' ({})",
                self.source, status
            )));
        }

        let mut stdout = child
            .stdout
            .take()
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

/// Error text for a missing `parec`, matched by `LoopbackIssue::from_start_error`.
const PAREC_NOT_FOUND: &str = "parec not found";

/// Why a meeting records only the microphone although loopback was requested.
///
/// Without loopback the remote participants vanish from the transcript, so
/// the daemon tells the user at meeting start, with the fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopbackIssue {
    /// `pactl` is not installed, so no monitor source could be detected.
    PactlMissing,
    /// `pactl` ran but could not list sources (usually no PulseAudio server).
    PactlFailed(String),
    /// The audio server lists no `.monitor` source.
    NoMonitorSource,
    /// `parec` is not installed.
    ParecMissing,
    /// `parec` started but could not record from the source.
    ParecFailed(String),
}

impl LoopbackIssue {
    fn from_start_error(error: &AudioError) -> Self {
        match error {
            AudioError::Connection(text) if text == PAREC_NOT_FOUND => LoopbackIssue::ParecMissing,
            AudioError::Connection(text) => LoopbackIssue::ParecFailed(text.clone()),
            other => LoopbackIssue::ParecFailed(other.to_string()),
        }
    }

    /// User-facing text: what is wrong and how to fix it.
    pub fn message(&self) -> String {
        const INSTALL: &str = "Install pactl and parec (libpulse on Arch, \
                               pulseaudio-utils on Debian, Ubuntu and Fedora).";
        const SERVER: &str = "On PipeWire, install pipewire-pulse and run \
                              `systemctl --user enable --now pipewire-pulse.socket`.";
        const PICK: &str = "Set [meeting.audio] loopback_device to a monitor source \
                            listed by `pactl list short sources`.";
        match self {
            LoopbackIssue::PactlMissing => {
                format!("Remote audio is not recorded: pactl is not installed. {INSTALL}")
            }
            LoopbackIssue::PactlFailed(detail) => format!(
                "Remote audio is not recorded: pactl could not list sources ({detail}). {SERVER}"
            ),
            LoopbackIssue::NoMonitorSource => {
                format!("Remote audio is not recorded: no monitor source was found. {PICK}")
            }
            LoopbackIssue::ParecMissing => {
                format!("Remote audio is not recorded: parec is not installed. {INSTALL}")
            }
            LoopbackIssue::ParecFailed(detail) => {
                format!("Remote audio is not recorded: {detail}. {SERVER} {PICK}")
            }
        }
    }
}

/// Pick the monitor source to record, preferring one that is RUNNING
/// (audio is playing through it). `listing` is `pactl list short sources`
/// output: ID, NAME, DRIVER, FORMAT, STATUS separated by tabs.
fn pick_monitor_source(listing: &str) -> Option<String> {
    let monitors: Vec<Vec<&str>> = listing
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .filter(|fields| fields.len() >= 2 && fields[1].contains(".monitor"))
        .collect();
    monitors
        .iter()
        .find(|fields| fields.get(4) == Some(&"RUNNING"))
        .or_else(|| monitors.first())
        .map(|fields| fields[1].to_string())
}

/// Dual audio capture for mic + loopback
pub struct DualCapture {
    /// Microphone capture (via cpal/ALSA)
    mic_capture: CpalCapture,
    /// Loopback capture (via parec subprocess)
    loopback: Option<ParecLoopback>,
    /// Sample counter for timestamps
    sample_counter: u64,
    /// Why loopback was requested but is not recording, if it isn't
    loopback_issue: Option<LoopbackIssue>,
}

impl DualCapture {
    /// Create a new dual capture instance
    pub fn new(
        mic_config: &AudioConfig,
        loopback_device: Option<&str>,
    ) -> Result<Self, AudioError> {
        let mic_capture = CpalCapture::new(mic_config)?;

        let mut loopback_issue = None;
        let loopback = match loopback_device {
            Some("disabled") | Some("") | None => None,
            Some("auto") => match Self::find_monitor_source() {
                Ok(source) => {
                    tracing::info!("Auto-detected loopback source: {}", source);
                    Some(ParecLoopback::new(source))
                }
                Err(issue) => {
                    tracing::warn!("Using mic only: {}", issue.message());
                    loopback_issue = Some(issue);
                    None
                }
            },
            Some(device) => {
                tracing::info!("Using configured loopback source: {}", device);
                Some(ParecLoopback::new(device.to_string()))
            }
        };

        Ok(Self {
            mic_capture,
            loopback,
            sample_counter: 0,
            loopback_issue,
        })
    }

    /// Find a PipeWire/PulseAudio monitor source via pactl
    fn find_monitor_source() -> Result<String, LoopbackIssue> {
        let output = std::process::Command::new("pactl")
            .args(["list", "short", "sources"])
            .output()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    LoopbackIssue::PactlMissing
                } else {
                    LoopbackIssue::PactlFailed(e.to_string())
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| output.status.to_string());
            return Err(LoopbackIssue::PactlFailed(detail));
        }

        let source = pick_monitor_source(&String::from_utf8_lossy(&output.stdout))
            .ok_or(LoopbackIssue::NoMonitorSource)?;
        tracing::debug!("Found monitor source: {}", source);
        Ok(source)
    }

    /// Why loopback was requested but is not recording. `None` when loopback
    /// is recording or was never requested. Meaningful after `start()`.
    pub fn loopback_issue(&self) -> Option<&LoopbackIssue> {
        self.loopback_issue.as_ref()
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
                self.loopback_issue = Some(LoopbackIssue::from_start_error(&e));
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
    fn pick_monitor_source_prefers_running() {
        let listing = "1\talsa_input.mic\tPipeWire\ts16le\tRUNNING\n\
                       2\talsa_output.hdmi.monitor\tPipeWire\ts16le\tSUSPENDED\n\
                       3\talsa_output.speakers.monitor\tPipeWire\ts16le\tRUNNING\n";
        assert_eq!(
            pick_monitor_source(listing).as_deref(),
            Some("alsa_output.speakers.monitor")
        );
    }

    #[test]
    fn pick_monitor_source_falls_back_to_any_monitor() {
        let listing = "1\talsa_input.mic\tPipeWire\ts16le\tRUNNING\n\
                       2\talsa_output.hdmi.monitor\tPipeWire\ts16le\tIDLE\n";
        assert_eq!(
            pick_monitor_source(listing).as_deref(),
            Some("alsa_output.hdmi.monitor")
        );
    }

    #[test]
    fn pick_monitor_source_none_without_monitors() {
        assert_eq!(
            pick_monitor_source("1\talsa_input.mic\tPipeWire\ts16le\tRUNNING\n"),
            None
        );
        assert_eq!(pick_monitor_source(""), None);
    }

    #[test]
    fn loopback_issue_messages_name_the_fix() {
        assert!(LoopbackIssue::PactlMissing.message().contains("libpulse"));
        assert!(LoopbackIssue::ParecMissing
            .message()
            .contains("pulseaudio-utils"));
        assert!(LoopbackIssue::PactlFailed("Connection refused".into())
            .message()
            .contains("pipewire-pulse"));
        assert!(LoopbackIssue::NoMonitorSource
            .message()
            .contains("loopback_device"));
    }

    #[test]
    fn parec_start_errors_classify() {
        let missing = AudioError::Connection(PAREC_NOT_FOUND.to_string());
        assert_eq!(
            LoopbackIssue::from_start_error(&missing),
            LoopbackIssue::ParecMissing
        );
        let failed =
            AudioError::Connection("parec could not record from 'x' (exit status: 1)".into());
        assert!(matches!(
            LoopbackIssue::from_start_error(&failed),
            LoopbackIssue::ParecFailed(_)
        ));
    }

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
