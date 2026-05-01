//! Simple source-based diarization
//!
//! Attributes speakers based on audio source:
//! - Microphone input → "You"
//! - System loopback → "Remote"
//!
//! This provides basic speaker separation without ML models.

use super::{DiarizedSegment, Diarizer, SpeakerId};
use crate::meeting::data::AudioSource;
use crate::meeting::TranscriptSegment;

/// Simple diarizer using audio source for attribution
pub struct SimpleDiarizer;

impl SimpleDiarizer {
    /// Create a new simple diarizer
    pub fn new() -> Self {
        Self
    }

    /// Convert audio source to speaker ID
    fn source_to_speaker(source: AudioSource) -> SpeakerId {
        match source {
            AudioSource::Microphone => SpeakerId::You,
            AudioSource::Loopback => SpeakerId::Remote,
            AudioSource::Unknown => SpeakerId::Unknown,
        }
    }
}

impl Default for SimpleDiarizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Diarizer for SimpleDiarizer {
    fn diarize(
        &self,
        _samples: &[f32],
        source: AudioSource,
        transcript_segments: &[TranscriptSegment],
    ) -> Vec<DiarizedSegment> {
        let speaker = Self::source_to_speaker(source);

        // Preserve transcript segment boundaries. The caller applies diarized
        // output back to transcript segments positionally, so returning fewer
        // segments would leave later transcript segments unlabeled.
        transcript_segments
            .iter()
            .map(|seg| DiarizedSegment {
                speaker: speaker.clone(),
                start_ms: seg.start_ms,
                end_ms: seg.end_ms,
                text: seg.text.clone(),
                confidence: 1.0, // High confidence for source-based attribution
            })
            .collect()
    }

    fn name(&self) -> &'static str {
        "simple"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_to_speaker() {
        assert_eq!(
            SimpleDiarizer::source_to_speaker(AudioSource::Microphone),
            SpeakerId::You
        );
        assert_eq!(
            SimpleDiarizer::source_to_speaker(AudioSource::Loopback),
            SpeakerId::Remote
        );
        assert_eq!(
            SimpleDiarizer::source_to_speaker(AudioSource::Unknown),
            SpeakerId::Unknown
        );
    }

    #[test]
    fn test_diarize_mic_segments() {
        let diarizer = SimpleDiarizer::new();
        let mut seg1 = TranscriptSegment::new(1, 0, 1000, "Hello".to_string(), 0);
        seg1.source = AudioSource::Microphone;
        let mut seg2 = TranscriptSegment::new(2, 1000, 2000, "World".to_string(), 0);
        seg2.source = AudioSource::Microphone;
        let segments = vec![seg1, seg2];

        let result = diarizer.diarize(&[], AudioSource::Microphone, &segments);

        // Should preserve transcript boundaries and label every segment.
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].speaker, SpeakerId::You);
        assert_eq!(result[1].speaker, SpeakerId::You);
        assert_eq!(result[0].text, "Hello");
        assert_eq!(result[1].text, "World");
    }

    #[test]
    fn test_diarize_labels_all_loopback_segments() {
        let diarizer = SimpleDiarizer::new();
        let mut seg1 = TranscriptSegment::new(1, 0, 1000, "First".to_string(), 0);
        seg1.source = AudioSource::Loopback;
        let mut seg2 = TranscriptSegment::new(2, 1000, 2000, "Second".to_string(), 0);
        seg2.source = AudioSource::Loopback;
        let segments = vec![seg1, seg2];

        let result = diarizer.diarize(&[], AudioSource::Loopback, &segments);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].speaker, SpeakerId::Remote);
        assert_eq!(result[1].speaker, SpeakerId::Remote);
        assert_eq!(result[0].text, "First");
        assert_eq!(result[1].text, "Second");
    }

    #[test]
    fn test_diarize_loopback() {
        let diarizer = SimpleDiarizer::new();
        let mut seg = TranscriptSegment::new(1, 0, 1000, "Remote speech".to_string(), 0);
        seg.source = AudioSource::Loopback;
        let segments = vec![seg];

        let result = diarizer.diarize(&[], AudioSource::Loopback, &segments);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].speaker, SpeakerId::Remote);
    }
}
