use crate::config::{OutputMode, MIN_ENABLED_RECORDING_QUEUE_SIZE};
use crate::state::AudioBuffer;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::SystemTime;

/// Per-recording metadata captured at recording start.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecordingMetadata {
    /// Optional model override for this recording.
    pub model_override: Option<String>,
    /// Optional profile override name for this recording.
    pub profile_override: Option<String>,
    /// Optional output mode override for this recording.
    pub output_mode_override: Option<OutputMode>,
    /// Optional explicit output file path override.
    pub output_file_path: Option<PathBuf>,
    /// Optional auto-submit override for this recording.
    pub auto_submit_override: Option<bool>,
    /// Optional shift-enter override for this recording.
    pub shift_enter_override: Option<bool>,
    /// Optional smart auto-submit override for this recording.
    pub smart_auto_submit_override: Option<bool>,
    /// Recording started timestamp.
    pub started_at: SystemTime,
    /// Recording stopped/enqueued timestamp.
    pub stopped_at: Option<SystemTime>,
}

impl RecordingMetadata {
    /// Construct metadata from explicit runtime values at recording start.
    #[allow(clippy::too_many_arguments)]
    pub fn started(
        model_override: Option<String>,
        profile_override: Option<String>,
        output_mode_override: Option<OutputMode>,
        output_file_path: Option<PathBuf>,
        auto_submit_override: Option<bool>,
        shift_enter_override: Option<bool>,
        smart_auto_submit_override: Option<bool>,
        started_at: SystemTime,
    ) -> Self {
        Self {
            model_override,
            profile_override,
            output_mode_override,
            output_file_path,
            auto_submit_override,
            shift_enter_override,
            smart_auto_submit_override,
            started_at,
            stopped_at: None,
        }
    }

    /// Update the stop/enqueue timestamp after recording ends.
    pub fn with_stopped_at(mut self, stopped_at: SystemTime) -> Self {
        self.stopped_at = Some(stopped_at);
        self
    }
}

/// Processing stages for queued work items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordingStage {
    /// Waiting to be processed.
    Waiting,
    /// Running transcription.
    Transcribing,
}

/// A stopped recording kept in queue.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QueuedStoppedRecording {
    pub metadata: RecordingMetadata,
    pub samples: AudioBuffer,
    pub stage: RecordingStage,
}

impl QueuedStoppedRecording {
    pub fn new(metadata: RecordingMetadata, samples: AudioBuffer) -> Self {
        Self {
            metadata,
            samples,
            stage: RecordingStage::Waiting,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QueueConfig {
    pub enabled: bool,
    pub queue_size: usize,
}

impl QueueConfig {
    pub const fn new(enabled: bool, queue_size: usize) -> Self {
        Self {
            enabled,
            queue_size,
        }
    }

    pub const fn is_effective_enabled(&self) -> bool {
        self.enabled && self.queue_size >= MIN_ENABLED_RECORDING_QUEUE_SIZE
    }
}

/// Internal FIFO helper for queued stopped recordings and one live reservation.
#[derive(Debug, Clone)]
pub(crate) struct RecordingQueue {
    config: QueueConfig,
    queue: VecDeque<QueuedStoppedRecording>,
    /// Work in-flight for queued items.
    processing_count: usize,
    /// Work in-flight for queued items deferred to output.
    deferred_output_count: usize,
    /// There is at most one active live capture reserved for one future queue slot.
    live_recording_reserved: bool,
}

impl RecordingQueue {
    pub fn new(config: QueueConfig) -> Self {
        Self {
            config,
            queue: VecDeque::new(),
            processing_count: 0,
            deferred_output_count: 0,
            live_recording_reserved: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.is_effective_enabled()
    }

    pub fn queued_count(&self) -> usize {
        self.queue.len()
    }

    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.config.queue_size
    }

    pub fn stopped_count(&self) -> usize {
        self.queue.len() + self.processing_count + self.deferred_output_count
    }

    #[allow(dead_code)]
    pub fn processing_count(&self) -> usize {
        self.processing_count
    }

    #[allow(dead_code)]
    pub fn deferred_output_count(&self) -> usize {
        self.deferred_output_count
    }

    #[allow(dead_code)]
    pub fn can_start_next_queued_job(&self) -> bool {
        self.processing_count == 0 && self.deferred_output_count == 0
    }

    pub fn can_start_recording(&self) -> bool {
        if !self.is_enabled() {
            return false;
        }
        if self.live_recording_reserved {
            return false;
        }
        self.stopped_count() < self.config.queue_size
    }

    pub fn start_recording(&mut self) -> bool {
        if !self.can_start_recording() {
            return false;
        }
        self.live_recording_reserved = true;
        true
    }

    pub fn release_live_recording(&mut self) {
        self.live_recording_reserved = false;
    }

    pub fn can_queue_stopped_recording(&self) -> bool {
        if !self.is_enabled() || !self.live_recording_reserved {
            return false;
        }
        // Live capture does not count in stopped_count(), but it may only stop
        // into the queue while stopped capacity remains.
        self.stopped_count() < self.config.queue_size
    }

    pub fn queue_stopped_recording(&mut self, item: QueuedStoppedRecording) -> bool {
        if !self.can_queue_stopped_recording() {
            return false;
        }

        self.queue.push_back(item);
        self.live_recording_reserved = false;
        true
    }

    /// Cancel a reserved live recording slot when a live recording is aborted
    /// before stop is reached (e.g. explicit cancel).
    pub fn cancel_active_recording(&mut self) {
        self.live_recording_reserved = false;
    }

    /// Move a queued job from transcribing to deferred output state.
    /// Returns true when a processing slot was moved.
    #[allow(dead_code)]
    pub fn move_transcribing_to_deferred_output(&mut self) -> bool {
        if self.processing_count == 0 {
            return false;
        }
        self.processing_count -= 1;
        self.deferred_output_count += 1;
        true
    }

    pub fn pop_next_for_transcription(&mut self) -> Option<QueuedStoppedRecording> {
        if !self.can_start_next_queued_job() {
            return None;
        }
        let mut item = self.queue.pop_front()?;
        item.stage = RecordingStage::Transcribing;
        self.processing_count += 1;
        Some(item)
    }

    pub fn finish_processing(&mut self) {
        if self.deferred_output_count > 0 {
            self.deferred_output_count -= 1;
            return;
        }

        self.processing_count = self.processing_count.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn base_metadata(
        name: &str,
        start_offset_secs: u64,
        stop_offset_secs: u64,
    ) -> RecordingMetadata {
        RecordingMetadata::started(
            Some(format!("{name}-model")),
            Some(format!("{name}-profile")),
            Some(OutputMode::Type),
            Some(PathBuf::from(format!("/tmp/{name}.txt"))),
            Some(true),
            Some(false),
            Some(true),
            UNIX_EPOCH + Duration::from_secs(start_offset_secs),
        )
        .with_stopped_at(UNIX_EPOCH + Duration::from_secs(stop_offset_secs))
    }

    #[test]
    fn queueing_is_effective_only_when_enabled_and_at_least_two() {
        assert!(!QueueConfig::new(true, 0).is_effective_enabled());
        assert!(!QueueConfig::new(true, 1).is_effective_enabled());
        assert!(!QueueConfig::new(false, 5).is_effective_enabled());
        assert!(QueueConfig::new(true, 2).is_effective_enabled());
        assert!(QueueConfig::new(true, 4).is_effective_enabled());
    }

    #[test]
    fn capacity_allows_live_reserve_until_stopped_queue_is_full() {
        let mut queue = RecordingQueue::new(QueueConfig::new(true, 2));

        assert!(queue.can_start_recording());
        assert!(queue.start_recording());
        assert!(queue.can_queue_stopped_recording());
        assert!(queue.queue_stopped_recording(QueuedStoppedRecording::new(
            base_metadata("first", 1, 2),
            vec![0.1],
        )));

        assert_eq!(queue.queued_count(), 1);
        assert_eq!(queue.stopped_count(), 1);
        assert!(queue.can_start_recording());
        assert!(queue.start_recording());
        assert!(queue.can_queue_stopped_recording());

        assert!(queue.queue_stopped_recording(QueuedStoppedRecording::new(
            base_metadata("second", 3, 4),
            vec![0.2],
        )));

        assert_eq!(queue.queued_count(), 2);
        assert_eq!(queue.stopped_count(), 2);
        assert!(!queue.can_start_recording());
    }

    #[test]
    fn queue_size_one_disables_queueing() {
        let queue = RecordingQueue::new(QueueConfig::new(true, 1));
        assert_eq!(queue.capacity(), 1);
        assert!(!queue.is_enabled());
        assert!(!queue.can_start_recording());
    }

    #[test]
    fn processing_job_counts_toward_capacity_until_finished() {
        let mut queue = RecordingQueue::new(QueueConfig::new(true, 2));

        assert!(queue.start_recording());
        assert!(queue.queue_stopped_recording(QueuedStoppedRecording::new(
            base_metadata("first", 1, 2),
            vec![0.1],
        )));

        let _processing = queue.pop_next_for_transcription().unwrap();
        assert_eq!(queue.queued_count(), 0);
        assert_eq!(queue.stopped_count(), 1);
        assert!(!queue.can_start_next_queued_job());
        assert!(queue.start_recording());
        assert!(queue.queue_stopped_recording(QueuedStoppedRecording::new(
            base_metadata("second", 3, 4),
            vec![0.2],
        )));
        assert_eq!(queue.stopped_count(), 2);
        assert!(!queue.can_start_recording());

        queue.finish_processing();
        assert_eq!(queue.stopped_count(), 1);
        assert!(queue.can_start_recording());
    }

    #[test]
    fn deferred_output_slot_counts_toward_stop_capacity() {
        let mut queue = RecordingQueue::new(QueueConfig::new(true, 2));

        assert!(queue.start_recording());
        assert!(queue.queue_stopped_recording(QueuedStoppedRecording::new(
            base_metadata("first", 1, 2),
            vec![0.1],
        )));

        let _processing = queue.pop_next_for_transcription().unwrap();
        assert_eq!(queue.stopped_count(), 1);
        assert_eq!(queue.processing_count(), 1);
        assert!(!queue.can_start_next_queued_job());

        assert!(queue.move_transcribing_to_deferred_output());
        assert_eq!(queue.processing_count(), 0);
        assert_eq!(queue.deferred_output_count(), 1);
        assert_eq!(queue.stopped_count(), 1);
        assert!(!queue.can_start_next_queued_job());

        assert!(queue.start_recording());
        assert!(queue.can_queue_stopped_recording());
        assert!(queue.queue_stopped_recording(QueuedStoppedRecording::new(
            base_metadata("second", 3, 4),
            vec![0.2],
        )));
        assert_eq!(queue.stopped_count(), 2);
        assert!(!queue.can_start_recording());

        queue.finish_processing();
        assert_eq!(queue.deferred_output_count(), 0);
        assert_eq!(queue.stopped_count(), 1);
        assert!(queue.can_start_recording());
    }

    #[test]
    fn released_live_recording_does_not_consume_capacity() {
        let mut queue = RecordingQueue::new(QueueConfig::new(true, 2));

        assert!(queue.start_recording());
        assert!(!queue.can_start_recording());
        queue.release_live_recording();

        assert!(queue.can_start_recording());
        assert_eq!(queue.stopped_count(), 0);
    }

    #[test]
    fn fifo_order_follows_stop_enqueue_order() {
        let mut queue = RecordingQueue::new(QueueConfig::new(true, 5));

        assert!(queue.start_recording());
        queue
            .queue_stopped_recording(QueuedStoppedRecording::new(
                base_metadata("first", 10, 11),
                vec![0.0, 0.1],
            ))
            .then_some(())
            .unwrap();

        assert!(queue.start_recording());
        queue
            .queue_stopped_recording(QueuedStoppedRecording::new(
                base_metadata("second", 20, 21),
                vec![0.2],
            ))
            .then_some(())
            .unwrap();

        let first = queue.pop_next_for_transcription().unwrap();
        assert_eq!(
            first.metadata.model_override.as_deref(),
            Some("first-model")
        );
        assert_eq!(first.stage, RecordingStage::Transcribing);
        assert!(!queue.can_start_next_queued_job());

        queue.finish_processing();

        let second = queue.pop_next_for_transcription().unwrap();
        assert_eq!(
            second.metadata.model_override.as_deref(),
            Some("second-model")
        );
        assert_eq!(second.stage, RecordingStage::Transcribing);
        assert_eq!(queue.stopped_count(), 1);

        queue.finish_processing();
        queue.finish_processing();
        assert_eq!(queue.stopped_count(), 0);
    }

    #[test]
    fn metadata_capture_records_all_overrides_and_timestamps() {
        let started = UNIX_EPOCH + Duration::from_millis(100);
        let stopped = UNIX_EPOCH + Duration::from_millis(500);
        let meta = RecordingMetadata::started(
            Some("model-a".to_string()),
            Some("profile-b".to_string()),
            Some(OutputMode::File),
            Some(PathBuf::from("/tmp/out.txt")),
            Some(true),
            Some(false),
            Some(true),
            started,
        )
        .with_stopped_at(stopped);

        assert_eq!(meta.model_override.as_deref(), Some("model-a"));
        assert_eq!(meta.profile_override.as_deref(), Some("profile-b"));
        assert_eq!(meta.output_mode_override, Some(OutputMode::File));
        assert_eq!(meta.output_file_path, Some(PathBuf::from("/tmp/out.txt")));
        assert_eq!(meta.auto_submit_override, Some(true));
        assert_eq!(meta.shift_enter_override, Some(false));
        assert_eq!(meta.smart_auto_submit_override, Some(true));
        assert_eq!(meta.started_at, started);
        assert_eq!(meta.stopped_at, Some(stopped));
    }
}
