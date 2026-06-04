//! Recording queue configuration.

use serde::{Deserialize, Serialize};

pub const MIN_ENABLED_RECORDING_QUEUE_SIZE: usize = 2;

fn default_queue_size() -> usize {
    5
}

/// Queueing configuration for normal batch dictation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RecordingConfig {
    /// Enable queueing while a previous normal batch is transcribing or outputting.
    #[serde(default)]
    pub queue_enabled: bool,

    /// Maximum stopped recordings waiting, transcribing, or outputting.
    ///
    /// A live recording is not counted while active, but starting one requires
    /// one available stopped slot so stopping can enqueue it.
    #[serde(default = "default_queue_size")]
    pub queue_size: usize,
}

impl RecordingConfig {
    pub fn effective_enabled(&self) -> bool {
        self.queue_enabled && self.queue_size >= MIN_ENABLED_RECORDING_QUEUE_SIZE
    }
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            queue_enabled: false,
            queue_size: default_queue_size(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_disable_queueing() {
        let config = RecordingConfig::default();
        assert!(!config.queue_enabled);
        assert_eq!(config.queue_size, 5);
        assert!(!config.effective_enabled());
    }

    #[test]
    fn queueing_is_effective_only_when_enabled_and_at_least_two() {
        let disabled = RecordingConfig::default();
        assert!(!disabled.effective_enabled());

        let enabled = RecordingConfig {
            queue_enabled: true,
            ..RecordingConfig::default()
        };
        assert!(enabled.effective_enabled());

        let size_one = RecordingConfig {
            queue_enabled: true,
            queue_size: 1,
        };
        assert!(!size_one.effective_enabled());

        let size_zero = RecordingConfig {
            queue_enabled: true,
            queue_size: 0,
        };
        assert!(!size_zero.effective_enabled());
    }
}
