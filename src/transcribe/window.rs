//! Splitting long audio into model-sized windows, and stitching the results
//! back together.
//!
//! Several ONNX engines have a hard ceiling on how much audio one forward
//! pass can take, and we were handing them whole recordings regardless. The
//! failures differ by engine — Cohere degenerates past its declared 35-second
//! clip window and its memory use climbs with the frame count (#551), while
//! Parakeet TDT aborts outright with a broadcast error from a fixed-size
//! attention bias (#288) — but the shape of the fix is the same: cut the
//! audio into overlapping windows, transcribe each, and rejoin.
//!
//! This module deliberately holds no engine types, so it compiles and its
//! tests run without the `cohere` or `parakeet` features. Both of those need
//! an ONNX Runtime prebuilt to link, which CI and most development machines
//! do not have, and logic that only gets exercised on a release builder is
//! logic nobody is checking.

/// Byte offsets of each window over `sample_count` samples.
///
/// Windows are `window` samples long and advance by `window - overlap`, so
/// consecutive windows share `overlap` samples of audio. The last window is
/// truncated at the end of the buffer rather than padded.
///
/// Returns a single full-length window when the audio already fits, so the
/// caller's short-input path stays byte-identical to a non-windowed run.
pub fn windows(sample_count: usize, window: usize, overlap: usize) -> Vec<(usize, usize)> {
    if sample_count == 0 || window == 0 {
        return Vec::new();
    }
    if sample_count <= window {
        return vec![(0, sample_count)];
    }

    // Clamped so a misconfigured overlap cannot stall the loop.
    let stride = window.saturating_sub(overlap).max(1);
    let mut out = Vec::new();
    let mut start = 0usize;
    loop {
        let end = (start + window).min(sample_count);
        out.push((start, end));
        if end == sample_count {
            break;
        }
        start += stride;
    }
    out
}

/// Join two consecutive window transcripts, dropping the text the shared
/// audio caused both of them to contain.
///
/// Consecutive windows overlap, so the tail of one and the head of the next
/// describe the same speech and usually decode to the same words. Take the
/// longest run of words that agrees and keep it once.
///
/// Comparison ignores case and surrounding punctuation, because the two
/// passes see different context and punctuate the shared region differently.
/// When nothing agrees — the overlap fell in silence, or the passes genuinely
/// disagree — fall back to plain concatenation. That repeats a few words,
/// where guessing at a join would delete speech, and repetition is the
/// failure a user can see and recover from.
pub fn merge_overlapping(acc: &str, next: &str) -> String {
    let next = next.trim();
    let acc = acc.trim();
    if acc.is_empty() {
        return next.to_string();
    }
    if next.is_empty() {
        return acc.to_string();
    }

    let norm = |w: &str| {
        w.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    let acc_words: Vec<&str> = acc.split_whitespace().collect();
    let next_words: Vec<&str> = next.split_whitespace().collect();

    // An overlap longer than either side cannot be a real repeat, and the
    // shared region is bounded by the overlap duration in any case.
    let max_k = acc_words.len().min(next_words.len()).min(64);
    for k in (1..=max_k).rev() {
        let tail = &acc_words[acc_words.len() - k..];
        let head = &next_words[..k];
        if tail
            .iter()
            .zip(head.iter())
            .all(|(a, b)| norm(a) == norm(b) && !norm(a).is_empty())
        {
            let rest = next_words[k..].join(" ");
            if rest.is_empty() {
                return acc.to_string();
            }
            return format!("{acc} {rest}");
        }
    }
    format!("{acc} {next}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Consecutive windows share audio, so both transcribe the same speech.
    /// Keeping it once is the point; keeping it twice reads as a stutter, and
    /// keeping it zero times deletes what the user said.
    #[test]
    fn overlapping_windows_keep_the_shared_words_once() {
        assert_eq!(
            merge_overlapping("the quick brown fox jumps over", "jumps over the lazy dog"),
            "the quick brown fox jumps over the lazy dog"
        );
    }

    /// The two passes see different context, so they punctuate and capitalize
    /// the shared region differently. If that defeated the match, every merge
    /// would fall back to duplicating the overlap.
    #[test]
    fn overlap_matching_ignores_case_and_punctuation() {
        assert_eq!(
            merge_overlapping(
                "we should ship it on friday,",
                "Friday we can cut the release"
            ),
            "we should ship it on friday, we can cut the release"
        );
    }

    /// When the overlap lands in silence or the passes genuinely disagree,
    /// there is nothing to match. Concatenating repeats a few words; guessing
    /// at a join would delete speech.
    #[test]
    fn disagreeing_windows_concatenate_rather_than_drop_speech() {
        assert_eq!(
            merge_overlapping("first window ends here", "second window starts elsewhere"),
            "first window ends here second window starts elsewhere"
        );
    }

    /// A window that decodes to nothing must not erase what came before it or
    /// leave stray whitespace behind.
    #[test]
    fn empty_windows_are_absorbed() {
        assert_eq!(merge_overlapping("", "opening words"), "opening words");
        assert_eq!(merge_overlapping("kept text", ""), "kept text");
        assert_eq!(merge_overlapping("", ""), "");
        assert_eq!(merge_overlapping("kept text", "   "), "kept text");
    }

    /// A window whose entire content already appeared in the previous one
    /// adds nothing, and must not leave a trailing space.
    #[test]
    fn a_fully_contained_window_adds_nothing() {
        assert_eq!(
            merge_overlapping("alpha beta gamma", "beta gamma"),
            "alpha beta gamma"
        );
    }

    /// A run of pure punctuation normalizes to the empty string. Treating
    /// that as agreement would splice two unrelated windows together at an
    /// arbitrary point.
    #[test]
    fn punctuation_only_tokens_do_not_count_as_a_match() {
        let merged = merge_overlapping("the meeting ended ...", "... and then we left");
        assert!(
            merged.contains("the meeting ended") && merged.contains("and then we left"),
            "both windows must survive, got {merged:?}"
        );
    }

    /// Audio that already fits produces exactly one full-length window, so
    /// the caller's short path is the same code it always ran.
    #[test]
    fn short_audio_is_a_single_window() {
        assert_eq!(windows(1000, 4000, 500), vec![(0, 1000)]);
        assert_eq!(windows(4000, 4000, 500), vec![(0, 4000)]);
    }

    /// Windows must cover every sample and overlap by the requested amount,
    /// or the merge has nothing to match on and speech falls in the gaps.
    #[test]
    fn windows_cover_the_whole_buffer_and_overlap() {
        let w = windows(10_000, 4_000, 1_000);
        assert_eq!(w.first().unwrap().0, 0, "coverage must start at zero");
        assert_eq!(w.last().unwrap().1, 10_000, "coverage must reach the end");
        for pair in w.windows(2) {
            let (_, prev_end) = pair[0];
            let (next_start, _) = pair[1];
            assert!(
                next_start < prev_end,
                "consecutive windows must overlap, got {next_start} after {prev_end}"
            );
            assert_eq!(prev_end - next_start, 1_000, "overlap must be as requested");
        }
    }

    /// Degenerate inputs must terminate rather than loop forever building
    /// windows. An overlap at or beyond the window size is a configuration
    /// error, not a reason to hang the daemon.
    #[test]
    fn degenerate_parameters_still_terminate() {
        assert!(windows(0, 4_000, 500).is_empty());
        assert!(windows(10_000, 0, 500).is_empty());

        let w = windows(10_000, 4_000, 9_999);
        assert_eq!(w.last().unwrap().1, 10_000);
        assert!(!w.is_empty());
    }
}
