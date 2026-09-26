//! Eager input processing module
//!
//! Handles chunking audio during recording and processing chunks in parallel
//! with continued recording. This reduces perceived latency on slower machines.
//!
//! The basic approach:
//! 1. During recording, cut the audio into consecutive, non-overlapping chunks.
//!    Each cut lands on the quietest point near the nominal chunk length, so
//!    it falls between words rather than through one.
//! 2. As each chunk is ready, spawn a transcription task for it
//! 3. Continue recording while transcription runs in parallel
//! 4. At the end, transcribe the tail and join the chunk texts in order
//!
//! Chunks used to overlap and the joined text was deduplicated at each
//! boundary by matching words. That both missed duplicates (punctuation made
//! "dictation." and "dictation" differ) and could not tell a duplicate from a
//! word the speaker really said twice. Disjoint chunks need no deduplication.

use crate::state::ChunkResult;

/// Length of the energy frames used to find a quiet cut point (20ms at 16kHz).
const FRAME_SAMPLES: usize = 320;

/// Longest stretch before the nominal boundary searched for a quiet cut.
const MAX_SEARCH_SECS: f32 = 1.5;

/// Configuration for eager processing
#[derive(Debug, Clone)]
pub struct EagerConfig {
    /// Nominal duration of each chunk in seconds. Actual chunks end at the
    /// quietest point within the search window before this length.
    pub chunk_secs: f32,
    /// Sample rate (assumed 16kHz for whisper)
    pub sample_rate: u32,
}

impl EagerConfig {
    /// Create config from whisper config settings
    pub fn from_whisper_config(config: &crate::config::WhisperConfig) -> Self {
        Self {
            chunk_secs: config.eager_chunk_secs,
            sample_rate: 16000, // Whisper expects 16kHz
        }
    }

    /// Get nominal chunk size in samples
    pub fn chunk_samples(&self) -> usize {
        ((self.chunk_secs * self.sample_rate as f32) as usize).max(FRAME_SAMPLES)
    }

    /// Samples before the nominal boundary searched for the quietest cut:
    /// up to 1.5s, never more than half a chunk.
    pub fn search_samples(&self) -> usize {
        let max = (MAX_SEARCH_SECS * self.sample_rate as f32) as usize;
        max.min(self.chunk_samples() / 2)
    }
}

/// Mean energy of each `FRAME_SAMPLES` frame in `audio[lo..hi]`, as
/// `(frame_start, mean_square)`. A trailing partial frame is ignored.
fn frame_energies(audio: &[f32], lo: usize, hi: usize) -> impl Iterator<Item = (usize, f32)> + '_ {
    (lo..hi.saturating_sub(FRAME_SAMPLES - 1))
        .step_by(FRAME_SAMPLES)
        .map(move |start| {
            let frame = &audio[start..start + FRAME_SAMPLES];
            let energy = frame.iter().map(|s| s * s).sum::<f32>() / FRAME_SAMPLES as f32;
            (start, energy)
        })
}

/// The cut point inside `audio[lo..hi]`: the middle of its quietest frame.
/// Ties go to the latest frame so chunks stay as close to nominal length as
/// the audio allows. Falls back to `hi` when the window holds no full frame.
fn quietest_cut(audio: &[f32], lo: usize, hi: usize) -> usize {
    frame_energies(audio, lo, hi)
        .fold(None::<(usize, f32)>, |best, (start, energy)| match best {
            Some((_, e)) if energy > e => best,
            _ => Some((start, energy)),
        })
        .map(|(start, _)| start + FRAME_SAMPLES / 2)
        .unwrap_or(hi)
}

/// End positions of every complete chunk in `audio`. Chunk `k` covers
/// `boundaries[k-1]..boundaries[k]` (chunk 0 starts at 0), so chunks are
/// contiguous and never overlap.
///
/// A boundary depends only on the audio before it, so the result for a
/// shorter prefix of the same recording is always a prefix of the result for
/// the longer one. That lets the daemon recompute boundaries on every poll
/// and trust that chunks it already sent keep the same bounds.
pub fn chunk_boundaries(audio: &[f32], config: &EagerConfig) -> Vec<usize> {
    let chunk = config.chunk_samples();
    let search = config.search_samples();
    let mut boundaries = Vec::new();
    let mut start = 0;
    while start + chunk <= audio.len() {
        let nominal = start + chunk;
        let cut = quietest_cut(audio, nominal - search, nominal);
        boundaries.push(cut);
        start = cut;
    }
    boundaries
}

/// Sample range of chunk `index`, or None if that chunk isn't complete yet.
pub fn chunk_range(boundaries: &[usize], index: usize) -> Option<(usize, usize)> {
    let end = *boundaries.get(index)?;
    let start = if index == 0 { 0 } else { boundaries[index - 1] };
    Some((start, end))
}

/// Where the tail begins once `chunks_sent` chunks have been transcribed.
pub fn tail_start(boundaries: &[usize], chunks_sent: usize) -> usize {
    match chunks_sent {
        0 => 0,
        n => boundaries.get(n - 1).copied().unwrap_or(0),
    }
}

/// Join transcription results from multiple chunks in chunk order.
///
/// Chunks are disjoint, so every word belongs to exactly one chunk and the
/// texts are simply concatenated. Empty chunks (silence, failures) are
/// skipped without leaving a double space.
///
/// # Arguments
/// * `results` - Vector of chunk results (may be in any order)
///
/// # Returns
/// Combined transcription text
pub fn combine_chunk_results(mut results: Vec<ChunkResult>) -> String {
    results.sort_by_key(|r| r.chunk_index);
    results
        .iter()
        .map(|r| r.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: usize = 16000;

    fn test_config() -> EagerConfig {
        EagerConfig {
            chunk_secs: 5.0,
            sample_rate: 16000,
        }
    }

    /// Continuous "speech" (a loud tone) with silent gaps at the given
    /// second offsets, each gap `gap_ms` long.
    fn speech_with_gaps(total_secs: f32, gaps_at: &[f32], gap_ms: usize) -> Vec<f32> {
        let len = (total_secs * SR as f32) as usize;
        let mut audio: Vec<f32> = (0..len).map(|i| 0.5 * (i as f32 * 0.07).sin()).collect();
        for &at in gaps_at {
            let start = (at * SR as f32) as usize;
            let end = (start + gap_ms * SR / 1000).min(len);
            audio[start..end].iter_mut().for_each(|s| *s = 0.0);
        }
        audio
    }

    fn result(text: &str, chunk_index: usize) -> ChunkResult {
        ChunkResult {
            text: text.to_string(),
            chunk_index,
        }
    }

    #[test]
    fn search_window_is_capped_at_half_a_chunk() {
        assert_eq!(test_config().search_samples(), 24000); // 1.5s
        let short = EagerConfig {
            chunk_secs: 2.0,
            sample_rate: 16000,
        };
        assert_eq!(short.search_samples(), 16000); // half of 2s
    }

    #[test]
    fn no_boundaries_before_a_full_chunk() {
        let config = test_config();
        assert!(chunk_boundaries(&[], &config).is_empty());
        assert!(chunk_boundaries(&vec![0.1; 4 * SR], &config).is_empty());
    }

    #[test]
    fn cut_lands_in_the_silent_gap_before_the_nominal_boundary() {
        // Gap at 4.2s-4.4s: inside the 3.5s-5.0s search window.
        let audio = speech_with_gaps(7.0, &[4.2], 200);
        let boundaries = chunk_boundaries(&audio, &test_config());
        assert_eq!(boundaries.len(), 1);
        let cut = boundaries[0] as f32 / SR as f32;
        assert!(
            (4.2..4.4).contains(&cut),
            "cut at {cut}s, expected in the gap"
        );
    }

    #[test]
    fn a_gap_outside_the_window_is_not_used() {
        // Gap at 2.0s is before the search window (3.5s-5.0s).
        let audio = speech_with_gaps(7.0, &[2.0], 200);
        let cut = chunk_boundaries(&audio, &test_config())[0] as f32 / SR as f32;
        assert!((3.5..=5.0).contains(&cut), "cut at {cut}s");
    }

    #[test]
    fn chunks_are_contiguous_and_cover_the_audio_once() {
        let audio = speech_with_gaps(23.0, &[4.6, 9.1, 13.7, 18.0], 150);
        let boundaries = chunk_boundaries(&audio, &test_config());
        assert!(boundaries.len() >= 3);
        let mut expected_start = 0;
        for i in 0..boundaries.len() {
            let (start, end) = chunk_range(&boundaries, i).unwrap();
            assert_eq!(
                start,
                expected_start,
                "chunk {i} starts where {} ended",
                i.saturating_sub(1)
            );
            assert!(end > start);
            expected_start = end;
        }
        assert_eq!(tail_start(&boundaries, boundaries.len()), expected_start);
        assert!(chunk_range(&boundaries, boundaries.len()).is_none());
    }

    #[test]
    fn boundaries_are_stable_as_audio_grows() {
        // The daemon recomputes boundaries every poll; chunks already sent
        // must keep their bounds when more audio arrives.
        let audio = speech_with_gaps(30.0, &[4.3, 8.8, 12.1, 17.5, 21.9], 120);
        let config = test_config();
        let full = chunk_boundaries(&audio, &config);
        for len in (5 * SR..audio.len()).step_by(SR / 3) {
            let prefix = chunk_boundaries(&audio[..len], &config);
            assert_eq!(
                &full[..prefix.len()],
                &prefix[..],
                "prefix of {len} samples"
            );
        }
    }

    #[test]
    fn continuous_speech_still_cuts_inside_the_window() {
        let audio = speech_with_gaps(11.0, &[], 0);
        let boundaries = chunk_boundaries(&audio, &test_config());
        assert_eq!(boundaries.len(), 2);
        assert!((3 * SR + SR / 2..=5 * SR).contains(&boundaries[0]));
    }

    #[test]
    fn tail_starts_at_zero_without_chunks() {
        assert_eq!(tail_start(&[], 0), 0);
        assert_eq!(tail_start(&[70000, 150000], 1), 70000);
        assert_eq!(tail_start(&[70000, 150000], 2), 150000);
    }

    #[test]
    fn test_combine_chunk_results_empty() {
        assert_eq!(combine_chunk_results(vec![]), "");
    }

    #[test]
    fn test_combine_chunk_results_single() {
        assert_eq!(
            combine_chunk_results(vec![result("hello world", 0)]),
            "hello world"
        );
    }

    #[test]
    fn combine_joins_disjoint_chunks_in_order() {
        let results = vec![
            result("foo bar", 1),
            result("hello world", 0),
            result("baz", 2),
        ];
        assert_eq!(combine_chunk_results(results), "hello world foo bar baz");
    }

    #[test]
    fn combine_keeps_words_the_speaker_really_repeated() {
        // A word said twice across a boundary is two words, not a duplicate.
        let results = vec![result("I think that", 0), result("that is fine", 1)];
        assert_eq!(combine_chunk_results(results), "I think that that is fine");
        let results = vec![result("dictation.", 0), result("Dictation works.", 1)];
        assert_eq!(
            combine_chunk_results(results),
            "dictation. Dictation works."
        );
    }

    #[test]
    fn combine_skips_empty_chunks_without_double_spaces() {
        let results = vec![
            result(" one ", 0),
            result("", 1),
            result("  ", 2),
            result("two", 3),
        ];
        assert_eq!(combine_chunk_results(results), "one two");
    }
}
