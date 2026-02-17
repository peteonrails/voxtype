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
pub struct SimpleDiarizer {
    /// Minimum gap between segments to merge (ms)
    merge_gap_ms: u64,
}

impl SimpleDiarizer {
    /// Create a new simple diarizer
    pub fn new() -> Self {
        Self { merge_gap_ms: 500 }
    }

    /// Create with custom merge gap
    pub fn with_merge_gap(merge_gap_ms: u64) -> Self {
        Self { merge_gap_ms }
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

        // Convert transcript segments to diarized segments
        let mut diarized: Vec<DiarizedSegment> = transcript_segments
            .iter()
            .map(|seg| DiarizedSegment {
                speaker: speaker.clone(),
                start_ms: seg.start_ms,
                end_ms: seg.end_ms,
                text: seg.text.clone(),
                confidence: 1.0, // High confidence for source-based attribution
            })
            .collect();

        // Merge consecutive segments from the same speaker
        self.merge_consecutive(&mut diarized);

        diarized
    }

    fn name(&self) -> &'static str {
        "simple"
    }
}

impl SimpleDiarizer {
    /// Merge consecutive segments from the same speaker if they're close together
    fn merge_consecutive(&self, segments: &mut Vec<DiarizedSegment>) {
        if segments.len() < 2 {
            return;
        }

        let mut i = 0;
        while i < segments.len() - 1 {
            let current_end = segments[i].end_ms;
            let next_start = segments[i + 1].start_ms;
            let same_speaker = segments[i].speaker == segments[i + 1].speaker;
            let close_enough = next_start.saturating_sub(current_end) <= self.merge_gap_ms;

            if same_speaker && close_enough {
                // Clone the text from next segment before modifying
                let next_text = segments[i + 1].text.clone();
                let next_end = segments[i + 1].end_ms;
                let next_confidence = segments[i + 1].confidence;

                // Merge next into current
                segments[i].end_ms = next_end;
                segments[i].text.push(' ');
                segments[i].text.push_str(&next_text);
                segments[i].confidence = (segments[i].confidence + next_confidence) / 2.0;
                segments.remove(i + 1);
            } else {
                i += 1;
            }
        }
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

        // Should merge into one segment since same speaker and close together
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].speaker, SpeakerId::You);
        assert_eq!(result[0].text, "Hello World");
    }

    #[test]
    fn test_diarize_preserves_separate_segments() {
        let diarizer = SimpleDiarizer::new();
        let mut seg1 = TranscriptSegment::new(1, 0, 1000, "First".to_string(), 0);
        seg1.source = AudioSource::Microphone;
        let mut seg2 = TranscriptSegment::new(2, 5000, 6000, "Second".to_string(), 0);
        seg2.source = AudioSource::Microphone;
        let segments = vec![seg1, seg2];

        let result = diarizer.diarize(&[], AudioSource::Microphone, &segments);

        // Should keep separate due to large gap
        assert_eq!(result.len(), 2);
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
