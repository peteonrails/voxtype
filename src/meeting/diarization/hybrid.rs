//! Hybrid diarization: the track decides "you or them", clustering splits "them"
//!
//! The microphone track is always the local user. Nothing on it is
//! clustered, so echo of the remote side that the mic picks up can never
//! turn into a speaker of its own. Speaker clustering (ECAPA-TDNN in the
//! `ml` backend) runs over loopback segments only and separates the remote
//! participants.
//!
//! Remote speakers are named by how many voices the loopback holds: a call
//! with one remote voice is "Remote", as with the `simple` backend. When a
//! second voice appears, the first becomes "Remote 1" and the new one
//! "Remote 2". That rename reaches segments from earlier chunks through
//! [`Diarizer::take_speaker_renames`].

use super::{DiarizedSegment, Diarizer, SpeakerId};
use crate::meeting::data::AudioSource;
use crate::meeting::TranscriptSegment;
use std::sync::Mutex;

/// The name every remote segment carries while only one remote voice is known.
const SINGLE_REMOTE: &str = "Remote";

#[derive(Default)]
struct HybridState {
    /// Remote clusters seen so far. Cluster IDs come from the inner
    /// clusterer as `Auto(n)` and are dense from 0.
    clusters: u32,
    /// The cluster that spoke last, given to loopback segments too short or
    /// too quiet to embed: a short interjection most often continues the
    /// voice before it.
    last_cluster: Option<u32>,
    /// Renames the meeting has not collected yet.
    pending_renames: Vec<(String, String)>,
}

/// Source-split diarizer with clustering over the loopback track only.
pub struct HybridDiarizer {
    /// Clusters loopback segments, labelling each `Auto(n)` or `Unknown`.
    remote: Box<dyn Diarizer>,
    state: Mutex<HybridState>,
}

impl HybridDiarizer {
    /// Wrap `remote`, the clusterer run over loopback segments.
    pub fn new(remote: Box<dyn Diarizer>) -> Self {
        Self {
            remote,
            state: Mutex::new(HybridState::default()),
        }
    }

    /// The name of remote cluster `n` when `clusters` voices are known.
    fn remote_name(n: u32, clusters: u32) -> String {
        if clusters <= 1 {
            SINGLE_REMOTE.to_string()
        } else {
            format!("Remote {}", n + 1)
        }
    }

    fn diarize_loopback(
        &self,
        samples: &[f32],
        chunk_start_ms: u64,
        transcript_segments: &[TranscriptSegment],
    ) -> Vec<DiarizedSegment> {
        let clustered = self.remote.diarize(
            samples,
            AudioSource::Loopback,
            chunk_start_ms,
            transcript_segments,
        );

        let Ok(mut state) = self.state.lock() else {
            tracing::warn!("Hybrid diarizer state lock poisoned, labelling loopback as Remote");
            return transcript_segments
                .iter()
                .map(|seg| labelled(seg, SINGLE_REMOTE.to_string(), 1.0))
                .collect();
        };

        let before = state.clusters;
        let mut clusters_by_segment = Vec::with_capacity(transcript_segments.len());
        for (i, _) in transcript_segments.iter().enumerate() {
            let cluster = match clustered.get(i).map(|d| &d.speaker) {
                Some(SpeakerId::Auto(n)) => {
                    state.clusters = state.clusters.max(n + 1);
                    state.last_cluster = Some(*n);
                    Some(*n)
                }
                _ => state.last_cluster,
            };
            clusters_by_segment.push(cluster);
        }

        // Earlier chunks named the only remote voice "Remote". Now that a
        // second one has spoken, that voice is "Remote 1".
        let clusters = state.clusters;
        if before <= 1 && clusters > 1 {
            state
                .pending_renames
                .push((SINGLE_REMOTE.to_string(), Self::remote_name(0, clusters)));
        }

        transcript_segments
            .iter()
            .zip(clusters_by_segment)
            .enumerate()
            .map(|(i, (seg, cluster))| {
                let confidence = clustered.get(i).map(|d| d.confidence).unwrap_or(0.0);
                match cluster {
                    Some(n) => labelled(seg, Self::remote_name(n, clusters), confidence),
                    // Nothing embedded yet: the remote side as a whole.
                    None => labelled(seg, SINGLE_REMOTE.to_string(), confidence),
                }
            })
            .collect()
    }
}

fn labelled(seg: &TranscriptSegment, name: String, confidence: f32) -> DiarizedSegment {
    DiarizedSegment {
        speaker: SpeakerId::Named(name),
        start_ms: seg.start_ms,
        end_ms: seg.end_ms,
        text: seg.text.clone(),
        confidence,
    }
}

impl Diarizer for HybridDiarizer {
    fn diarize(
        &self,
        samples: &[f32],
        source: AudioSource,
        chunk_start_ms: u64,
        transcript_segments: &[TranscriptSegment],
    ) -> Vec<DiarizedSegment> {
        match source {
            AudioSource::Microphone => transcript_segments
                .iter()
                .map(|seg| DiarizedSegment {
                    speaker: SpeakerId::You,
                    start_ms: seg.start_ms,
                    end_ms: seg.end_ms,
                    text: seg.text.clone(),
                    confidence: 1.0,
                })
                .collect(),
            AudioSource::Loopback => {
                self.diarize_loopback(samples, chunk_start_ms, transcript_segments)
            }
            AudioSource::Unknown => transcript_segments
                .iter()
                .map(|seg| DiarizedSegment {
                    speaker: SpeakerId::Unknown,
                    start_ms: seg.start_ms,
                    end_ms: seg.end_ms,
                    text: seg.text.clone(),
                    confidence: 0.0,
                })
                .collect(),
        }
    }

    fn take_speaker_renames(&self) -> Vec<(String, String)> {
        self.state
            .lock()
            .map(|mut s| std::mem::take(&mut s.pending_renames))
            .unwrap_or_default()
    }

    fn name(&self) -> &'static str {
        "hybrid"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A clusterer that hands out a scripted speaker per segment, in order
    /// across calls, and records which tracks it was asked about.
    struct Scripted {
        speakers: Mutex<std::collections::VecDeque<SpeakerId>>,
        sources_seen: Arc<Mutex<Vec<AudioSource>>>,
    }

    impl Scripted {
        fn new(speakers: Vec<SpeakerId>) -> (Self, Arc<Mutex<Vec<AudioSource>>>) {
            let seen = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    speakers: Mutex::new(speakers.into()),
                    sources_seen: seen.clone(),
                },
                seen,
            )
        }
    }

    impl Diarizer for Scripted {
        fn diarize(
            &self,
            _samples: &[f32],
            source: AudioSource,
            _chunk_start_ms: u64,
            segs: &[TranscriptSegment],
        ) -> Vec<DiarizedSegment> {
            self.sources_seen.lock().unwrap().push(source);
            let mut speakers = self.speakers.lock().unwrap();
            segs.iter()
                .map(|seg| DiarizedSegment {
                    speaker: speakers.pop_front().unwrap_or(SpeakerId::Unknown),
                    start_ms: seg.start_ms,
                    end_ms: seg.end_ms,
                    text: seg.text.clone(),
                    confidence: 0.8,
                })
                .collect()
        }

        fn name(&self) -> &'static str {
            "scripted"
        }
    }

    fn segs(n: usize, source: AudioSource) -> Vec<TranscriptSegment> {
        (0..n)
            .map(|i| {
                let mut s = TranscriptSegment::new(
                    i as u32,
                    i as u64 * 1000,
                    i as u64 * 1000 + 900,
                    format!("s{i}"),
                    0,
                );
                s.source = source;
                s
            })
            .collect()
    }

    fn names(out: &[DiarizedSegment]) -> Vec<String> {
        out.iter().map(|d| d.speaker.display_name()).collect()
    }

    #[test]
    fn mic_is_always_you_and_never_clustered() {
        let (inner, seen) = Scripted::new(vec![SpeakerId::Auto(0), SpeakerId::Auto(1)]);
        let d = HybridDiarizer::new(Box::new(inner));

        let out = d.diarize(
            &[],
            AudioSource::Microphone,
            0,
            &segs(2, AudioSource::Microphone),
        );

        assert_eq!(names(&out), vec!["You", "You"]);
        assert!(
            seen.lock().unwrap().is_empty(),
            "mic audio reached the clusterer"
        );
        assert!(d.take_speaker_renames().is_empty());
    }

    #[test]
    fn one_remote_voice_stays_remote() {
        let (inner, _) = Scripted::new(vec![SpeakerId::Auto(0); 5]);
        let d = HybridDiarizer::new(Box::new(inner));

        let first = d.diarize(
            &[],
            AudioSource::Loopback,
            0,
            &segs(3, AudioSource::Loopback),
        );
        let second = d.diarize(
            &[],
            AudioSource::Loopback,
            30_000,
            &segs(2, AudioSource::Loopback),
        );

        assert_eq!(names(&first), vec!["Remote"; 3]);
        assert_eq!(names(&second), vec!["Remote"; 2]);
        assert!(d.take_speaker_renames().is_empty());
    }

    #[test]
    fn a_second_remote_voice_numbers_both_and_renames_the_first() {
        let (inner, _) = Scripted::new(vec![
            SpeakerId::Auto(0),
            SpeakerId::Auto(0),
            SpeakerId::Auto(1),
            SpeakerId::Auto(0),
        ]);
        let d = HybridDiarizer::new(Box::new(inner));

        let first = d.diarize(
            &[],
            AudioSource::Loopback,
            0,
            &segs(2, AudioSource::Loopback),
        );
        assert_eq!(names(&first), vec!["Remote", "Remote"]);

        let second = d.diarize(
            &[],
            AudioSource::Loopback,
            30_000,
            &segs(2, AudioSource::Loopback),
        );
        assert_eq!(names(&second), vec!["Remote 2", "Remote 1"]);
        assert_eq!(
            d.take_speaker_renames(),
            vec![("Remote".to_string(), "Remote 1".to_string())]
        );
        // Collected once.
        assert!(d.take_speaker_renames().is_empty());
    }

    #[test]
    fn two_voices_in_the_first_chunk_are_numbered_without_a_rename_of_earlier_chunks() {
        let (inner, _) = Scripted::new(vec![SpeakerId::Auto(0), SpeakerId::Auto(1)]);
        let d = HybridDiarizer::new(Box::new(inner));

        let out = d.diarize(
            &[],
            AudioSource::Loopback,
            0,
            &segs(2, AudioSource::Loopback),
        );

        // Within the chunk the names use the final count.
        assert_eq!(names(&out), vec!["Remote 1", "Remote 2"]);
        // A rename is still queued; with nothing named "Remote" yet it is a no-op.
        assert_eq!(
            d.take_speaker_renames(),
            vec![("Remote".to_string(), "Remote 1".to_string())]
        );
    }

    #[test]
    fn unembeddable_segment_follows_the_last_remote_voice() {
        let (inner, _) = Scripted::new(vec![
            SpeakerId::Auto(0),
            SpeakerId::Auto(1),
            SpeakerId::Unknown,
        ]);
        let d = HybridDiarizer::new(Box::new(inner));

        let out = d.diarize(
            &[],
            AudioSource::Loopback,
            0,
            &segs(3, AudioSource::Loopback),
        );

        assert_eq!(names(&out), vec!["Remote 1", "Remote 2", "Remote 2"]);
    }

    #[test]
    fn unembeddable_segment_before_any_voice_is_remote() {
        let (inner, _) = Scripted::new(vec![SpeakerId::Unknown]);
        let d = HybridDiarizer::new(Box::new(inner));

        let out = d.diarize(
            &[],
            AudioSource::Loopback,
            0,
            &segs(1, AudioSource::Loopback),
        );

        assert_eq!(names(&out), vec!["Remote"]);
    }
}
