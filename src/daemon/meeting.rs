//! Meeting mode's own state: the capture pair, the two chunk buffers, the
//! meeting daemon it drives, and the state file it reports through.
//!
//! These were seven fields on `Daemon` whose lifecycles happened to agree, with
//! the agreement spread across the start, stop, pause and event arms. Here they
//! agree by construction: the buffers are only reachable through this type's
//! methods, so a cycle cannot clear one and keep the other.
//!
//! What stays on `Daemon` is the orchestration that needs the rest of the
//! daemon: starting a meeting reads the daemon's config, stopping one plays
//! feedback and sends the notification, and the loop's arm drives the events.

use crate::audio::MeetingCapture;
use crate::config::Config;
use crate::meeting::{self, MeetingDaemon, MeetingEvent, StorageConfig};
use crate::runtime_files::RuntimePaths;
use std::path::PathBuf;
use tokio::sync::mpsc::Receiver;

#[cfg(feature = "onnx-common")]
use crate::audio::enhance::GtcrnEnhancer;
#[cfg(feature = "onnx-common")]
use std::sync::Arc;

/// Everything a meeting owns between its start and its stop.
pub struct MeetingSession {
    /// The meeting daemon, present between start and stop.
    daemon: Option<MeetingDaemon>,
    /// The mic + loopback capture, present between start and stop.
    capture: Option<Box<dyn MeetingCapture>>,
    /// Mic samples that do not yet fill a chunk.
    mic_buffer: Vec<f32>,
    /// Loopback samples that do not yet fill a chunk.
    loopback_buffer: Vec<f32>,
    /// Events from the meeting daemon, driven by the main loop.
    events: Option<Receiver<MeetingEvent>>,
    /// GTCRN echo/noise enhancer, loaded with the first meeting that asks for
    /// echo cancellation.
    #[cfg(feature = "onnx-common")]
    enhancer: Option<Arc<GtcrnEnhancer>>,
    /// The state file to report through, when the daemon has one at all.
    state_file: Option<PathBuf>,
}

impl MeetingSession {
    /// A session with nothing in flight. `state_file` is `None` when the daemon
    /// runs without a state file, in which case meetings report nowhere.
    pub fn new(state_file: Option<PathBuf>) -> Self {
        Self {
            daemon: None,
            capture: None,
            mic_buffer: Vec::new(),
            loopback_buffer: Vec::new(),
            events: None,
            #[cfg(feature = "onnx-common")]
            enhancer: None,
            state_file,
        }
    }

    /// The state file, for the shutdown cleanup.
    pub fn state_file(&self) -> Option<&PathBuf> {
        self.state_file.as_ref()
    }

    /// Report a meeting state to external integrations, if a file is configured.
    pub fn update_state(&self, state_name: &str, meeting_id: Option<&str>) {
        if let Some(path) = &self.state_file {
            write_state_file(path, state_name, meeting_id);
        }
    }

    /// Whether a meeting exists at all, paused or not.
    pub fn in_progress(&self) -> bool {
        self.daemon.is_some()
    }

    /// Whether the meeting in progress is paused.
    pub fn is_paused(&self) -> bool {
        self.daemon.as_ref().is_some_and(|d| d.state().is_paused())
    }

    /// How long the meeting has been running, for the duration limit.
    pub fn elapsed(&self) -> Option<std::time::Duration> {
        self.daemon.as_ref().and_then(|d| d.state().elapsed())
    }

    pub fn capture_mut(&mut self) -> Option<&mut Box<dyn MeetingCapture>> {
        self.capture.as_mut()
    }

    /// Take one queued event without waiting. `Some(None)` means the channel
    /// closed, which is how the daemon learns the meeting daemon is gone.
    pub fn try_event(&mut self) -> Option<Option<MeetingEvent>> {
        let rx = self.events.as_mut()?;
        match rx.try_recv() {
            Ok(event) => Some(Some(event)),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => None,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => Some(None),
        }
    }

    /// Whether a meeting is running (started and not paused).
    pub fn is_active(&self) -> bool {
        self.daemon.as_ref().is_some_and(|d| d.state().is_active())
    }

    pub fn daemon_mut(&mut self) -> Option<&mut MeetingDaemon> {
        self.daemon.as_mut()
    }

    pub fn take_daemon(&mut self) -> Option<MeetingDaemon> {
        self.daemon.take()
    }

    /// Adopt a started meeting. The buffers start empty because this meeting's
    /// audio begins now; the previous meeting's tail was flushed at its stop.
    pub fn set_daemon(&mut self, daemon: MeetingDaemon, events: Receiver<MeetingEvent>) {
        self.daemon = Some(daemon);
        self.events = Some(events);
        self.mic_buffer.clear();
        self.loopback_buffer.clear();
    }

    pub fn take_capture(&mut self) -> Option<Box<dyn MeetingCapture>> {
        self.capture.take()
    }

    pub fn set_capture(&mut self, capture: Box<dyn MeetingCapture>) {
        self.capture = Some(capture);
    }

    pub fn push_mic(&mut self, samples: Vec<f32>) {
        self.mic_buffer.extend(samples);
    }

    pub fn push_loopback(&mut self, samples: Vec<f32>) {
        self.loopback_buffer.extend(samples);
    }

    #[cfg(feature = "onnx-common")]
    pub fn enhancer(&self) -> Option<&Arc<GtcrnEnhancer>> {
        self.enhancer.as_ref()
    }

    #[cfg(feature = "onnx-common")]
    pub fn set_enhancer(&mut self, enhancer: Arc<GtcrnEnhancer>) {
        self.enhancer = Some(enhancer);
    }

    /// Samples in one chunk at the meeting's configured chunk duration. 16 kHz
    /// is the meeting pipeline's rate, not a preference.
    pub fn chunk_samples(&self, chunk_duration_secs: u32) -> usize {
        16_000 * chunk_duration_secs as usize
    }

    /// Feed one chunk from each source to the meeting, enhancing the mic side
    /// first when the GTCRN model is loaded.
    async fn process_pair(&mut self, mic_chunk: Vec<f32>, loopback_chunk: Vec<f32>) {
        #[cfg_attr(not(feature = "onnx-common"), allow(unused_mut))]
        let mut mic_chunk = mic_chunk;

        // Enhance mic audio with GTCRN if available (removes echo/noise)
        #[cfg(feature = "onnx-common")]
        {
            if !mic_chunk.is_empty() {
                if let Some(enhancer) = &self.enhancer {
                    match enhancer.enhance(&mic_chunk) {
                        Ok(enhanced) => {
                            tracing::debug!(
                                "GTCRN enhanced mic chunk ({} samples)",
                                enhanced.len()
                            );
                            mic_chunk = enhanced;
                        }
                        Err(e) => {
                            tracing::warn!("GTCRN enhancement failed, using raw mic: {}", e);
                        }
                    }
                }
            }
        }

        if let Some(daemon) = &mut self.daemon {
            let mut had_loopback = false;

            if !mic_chunk.is_empty() {
                match daemon
                    .process_chunk_with_source(mic_chunk, meeting::data::AudioSource::Microphone)
                    .await
                {
                    Ok(Some(segments)) => {
                        tracing::debug!("Processed mic chunk with {} segments", segments.len());
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!("Error processing mic chunk: {}", e);
                    }
                }
            }

            if !loopback_chunk.is_empty() {
                match daemon
                    .process_chunk_with_source(loopback_chunk, meeting::data::AudioSource::Loopback)
                    .await
                {
                    Ok(Some(segments)) => {
                        tracing::debug!(
                            "Processed loopback chunk with {} segments",
                            segments.len()
                        );
                        if !segments.is_empty() {
                            had_loopback = true;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!("Error processing loopback chunk: {}", e);
                    }
                }
            }

            // Reconcile per-source offsets so any source that received a short or
            // skipped chunk this iteration catches up to wall-clock before the
            // next one. Added in PR #330 to fix dual-source timestamp inflation
            // in meeting mode.
            daemon.sync_source_offsets();

            // Dedup bleed-through: strip echoed phrases from mic segments
            if had_loopback {
                if let Some(meeting) = daemon.current_meeting_mut() {
                    let removed = meeting.transcript.dedup_bleed_through();
                    if removed > 0 {
                        tracing::info!("Removed {} bleed-through word(s) via dedup", removed);
                    }
                }
            }
        }
    }

    /// Turn buffered audio into full chunks and feed them, optionally flushing
    /// the partial tail as well (at stop, so speech near the end is not lost).
    pub async fn process_buffered(&mut self, include_tail: bool, chunk_samples: usize) {
        while self.mic_buffer.len() >= chunk_samples {
            let mic_chunk: Vec<f32> = self.mic_buffer.drain(..chunk_samples).collect();
            let loopback_len = self.loopback_buffer.len().min(chunk_samples);
            let loopback_chunk: Vec<f32> = self.loopback_buffer.drain(..loopback_len).collect();
            self.process_pair(mic_chunk, loopback_chunk).await;
        }

        if include_tail {
            let mic_tail = std::mem::take(&mut self.mic_buffer);
            let loopback_tail = std::mem::take(&mut self.loopback_buffer);
            if !mic_tail.is_empty() || !loopback_tail.is_empty() {
                tracing::debug!(
                    mic_samples = mic_tail.len(),
                    loopback_samples = loopback_tail.len(),
                    "Processing final meeting audio tail"
                );
                self.process_pair(mic_tail, loopback_tail).await;
            }
        }
    }

    /// Forget the meeting: its events, its capture and its buffers. The daemon
    /// is taken separately by the stop path, which has to report the id.
    pub fn clear(&mut self) {
        self.mic_buffer.clear();
        self.loopback_buffer.clear();
        self.events = None;
    }
}

/// Mark any active/paused meetings as completed on daemon startup.
/// This handles meetings orphaned by a crash or daemon restart.
pub(super) fn cleanup_stale(paths: &RuntimePaths, config: &Config) {
    let storage_path = if config.meeting.storage_path == "auto" {
        Config::data_dir().join("meetings")
    } else {
        PathBuf::from(&config.meeting.storage_path)
    };

    let storage_config = StorageConfig {
        storage_path,
        retain_audio: config.meeting.retain_audio,
        max_meetings: 0,
    };

    match meeting::MeetingStorage::open(storage_config) {
        Ok(storage) => match storage.complete_stale_meetings() {
            Ok(count) if count > 0 => {
                tracing::info!("Marked {} orphaned meeting(s) as completed", count);
                // Reset meeting state file to idle
                let state_file = paths.meeting_state();
                let _ = std::fs::write(&state_file, "idle");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("Failed to clean up stale meetings: {}", e),
        },
        Err(e) => tracing::warn!("Failed to open meeting storage for cleanup: {}", e),
    }
}

/// Write meeting state file for external integrations
fn write_state_file(path: &PathBuf, state: &str, meeting_id: Option<&str>) {
    let content = if let Some(id) = meeting_id {
        format!("{}\n{}", state, id)
    } else {
        state.to_string()
    };

    if let Err(e) = std::fs::write(path, content) {
        tracing::warn!("Failed to write meeting state file: {}", e);
    }
}
