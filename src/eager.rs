//! Eager input processing module
//!
//! Handles chunking audio during recording and processing chunks in parallel
//! with continued recording. This reduces perceived latency on slower machines.
//!
//! The basic approach:
//! 1. During recording, split audio into fixed-size chunks with small overlaps
//! 2. As each chunk is ready, spawn a transcription task for it
//! 3. Continue recording while transcription runs in parallel
//! 4. At the end, combine all chunk results, deduplicating at boundaries

use crate::state::ChunkResult;

/// Configuration for eager processing
#[derive(Debug, Clone)]
pub struct EagerConfig {
    /// Duration of each chunk in seconds
    pub chunk_secs: f32,
    /// Overlap between adjacent chunks in seconds
    pub overlap_secs: f32,
    /// Sample rate (assumed 16kHz for whisper)
    pub sample_rate: u32,
}

impl EagerConfig {
    /// Create config from whisper config settings
    pub fn from_whisper_config(config: &crate::config::WhisperConfig) -> Self {
        Self {
            chunk_secs: config.eager_chunk_secs,
            overlap_secs: config.eager_overlap_secs,
            sample_rate: 16000, // Whisper expects 16kHz
        }
    }

    /// Get chunk size in samples
    pub fn chunk_samples(&self) -> usize {
        (self.chunk_secs * self.sample_rate as f32) as usize
    }

    /// Get overlap size in samples
    pub fn overlap_samples(&self) -> usize {
        (self.overlap_secs * self.sample_rate as f32) as usize
    }

    /// Get the stride between chunk starts (chunk - overlap)
    pub fn stride_samples(&self) -> usize {
        self.chunk_samples().saturating_sub(self.overlap_samples())
    }
}

/// Extract a chunk from accumulated audio for transcription.
/// Returns None if there isn't enough audio for the requested chunk yet.
///
/// # Arguments
/// * `accumulated` - All audio samples collected so far
/// * `chunk_index` - Which chunk to extract (0-based)
/// * `config` - Eager processing configuration
///
/// # Returns
/// * `Some(Vec<f32>)` - The audio chunk to transcribe
/// * `None` - Not enough audio yet for this chunk
pub fn extract_chunk(
    accumulated: &[f32],
    chunk_index: usize,
    config: &EagerConfig,
) -> Option<Vec<f32>> {
    let chunk_size = config.chunk_samples();
    let stride = config.stride_samples();

    // Calculate chunk boundaries
    let start = chunk_index * stride;
    let end = start + chunk_size;

    // Check if we have enough samples
    if end > accumulated.len() {
        return None;
    }

    Some(accumulated[start..end].to_vec())
}

/// Check how many complete chunks are available in the accumulated audio.
/// A chunk is "complete" when we have enough samples to extract it plus
/// the overlap for the next chunk (so we don't cut off mid-word).
///
/// # Arguments
/// * `accumulated_len` - Number of samples accumulated so far
/// * `config` - Eager processing configuration
///
/// # Returns
/// Number of complete chunks available
pub fn count_complete_chunks(accumulated_len: usize, config: &EagerConfig) -> usize {
    let stride = config.stride_samples();
    let chunk_size = config.chunk_samples();

    if accumulated_len < chunk_size {
        return 0;
    }

    // Number of chunks where we have the full chunk + overlap for boundary handling
    let available_after_first = accumulated_len.saturating_sub(chunk_size);
    1 + available_after_first / stride
}

/// Combine transcription results from multiple chunks, handling duplicates
/// at chunk boundaries.
///
/// # Arguments
/// * `results` - Vector of chunk results (may be in any order)
///
/// # Returns
/// Combined transcription text with duplicates at boundaries removed
pub fn combine_chunk_results(mut results: Vec<ChunkResult>) -> String {
    if results.is_empty() {
        return String::new();
    }

    // Sort by chunk index to ensure correct order
    results.sort_by_key(|r| r.chunk_index);

    if results.len() == 1 {
        return results[0].text.clone();
    }

    let mut combined = String::new();

    for (i, result) in results.iter().enumerate() {
        if i == 0 {
            // First chunk: use full text
            combined = result.text.clone();
        } else {
            // Subsequent chunks: deduplicate at boundary
            let new_text = deduplicate_boundary(&combined, &result.text);
            if !new_text.is_empty() {
                if !combined.is_empty() && !combined.ends_with(' ') && !new_text.starts_with(' ') {
                    combined.push(' ');
                }
                combined.push_str(&new_text);
            }
        }
    }

    combined.trim().to_string()
}

/// Remove duplicate text at the boundary between previous and new transcription.
///
/// This uses a simple approach: look for the longest suffix of `previous` that
/// matches a prefix of `new_text`, and return `new_text` with that prefix removed.
///
/// # Arguments
/// * `previous` - Text transcribed so far (from earlier chunks)
/// * `new_text` - Text from the new chunk
///
/// # Returns
/// The portion of `new_text` that isn't a duplicate of `previous`
fn deduplicate_boundary(previous: &str, new_text: &str) -> String {
    let previous_words: Vec<&str> = previous.split_whitespace().collect();
    let new_words: Vec<&str> = new_text.split_whitespace().collect();

    if previous_words.is_empty() || new_words.is_empty() {
        return new_text.to_string();
    }

    // Look for overlap: find the longest suffix of previous that matches
    // a prefix of new_text
    let max_overlap = previous_words.len().min(new_words.len());

    let mut best_overlap = 0;
    for overlap_len in 1..=max_overlap {
        let prev_suffix = &previous_words[previous_words.len() - overlap_len..];
        let new_prefix = &new_words[..overlap_len];

        // Case-insensitive comparison for robustness
        if prev_suffix
            .iter()
            .zip(new_prefix.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            best_overlap = overlap_len;
        }
    }

    if best_overlap > 0 {
        // Remove the overlapping prefix from new_text
        new_words[best_overlap..].join(" ")
    } else {
        new_text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> EagerConfig {
        EagerConfig {
            chunk_secs: 5.0,
            overlap_secs: 0.5,
            sample_rate: 16000,
        }
    }

    #[test]
    fn test_chunk_samples() {
        let config = test_config();
        assert_eq!(config.chunk_samples(), 80000); // 5 seconds * 16000 Hz
        assert_eq!(config.overlap_samples(), 8000); // 0.5 seconds * 16000 Hz
        assert_eq!(config.stride_samples(), 72000); // chunk - overlap
    }

    #[test]
    fn test_count_complete_chunks_empty() {
        let config = test_config();
        assert_eq!(count_complete_chunks(0, &config), 0);
    }

    #[test]
    fn test_count_complete_chunks_less_than_one() {
        let config = test_config();
        // Less than one chunk
        assert_eq!(count_complete_chunks(40000, &config), 0);
    }

    #[test]
    fn test_count_complete_chunks_one() {
        let config = test_config();
        // Exactly one chunk
        assert_eq!(count_complete_chunks(80000, &config), 1);
    }

    #[test]
    fn test_count_complete_chunks_multiple() {
        let config = test_config();
        // First chunk (80000) + stride for second (72000) = 152000
        assert_eq!(count_complete_chunks(152000, &config), 2);
        // First chunk + 2 strides = 224000
        assert_eq!(count_complete_chunks(224000, &config), 3);
    }

    #[test]
    fn test_extract_chunk_insufficient_data() {
        let config = test_config();
        let audio = vec![0.0; 40000]; // Less than one chunk
        assert!(extract_chunk(&audio, 0, &config).is_none());
    }

    #[test]
    fn test_extract_chunk_first() {
        let config = test_config();
        let audio: Vec<f32> = (0..100000).map(|i| i as f32).collect();
        let chunk = extract_chunk(&audio, 0, &config).unwrap();
        assert_eq!(chunk.len(), 80000);
        assert_eq!(chunk[0], 0.0);
    }

    #[test]
    fn test_extract_chunk_second() {
        let config = test_config();
        let audio: Vec<f32> = (0..200000).map(|i| i as f32).collect();
        let chunk = extract_chunk(&audio, 1, &config).unwrap();
        assert_eq!(chunk.len(), 80000);
        // Second chunk starts at stride (72000)
        assert_eq!(chunk[0], 72000.0);
    }

    #[test]
    fn test_deduplicate_boundary_no_overlap() {
        let result = deduplicate_boundary("hello world", "foo bar");
        assert_eq!(result, "foo bar");
    }

    #[test]
    fn test_deduplicate_boundary_single_word_overlap() {
        let result = deduplicate_boundary("hello world", "world foo bar");
        assert_eq!(result, "foo bar");
    }

    #[test]
    fn test_deduplicate_boundary_multi_word_overlap() {
        let result = deduplicate_boundary("hello world foo", "world foo bar baz");
        assert_eq!(result, "bar baz");
    }

    #[test]
    fn test_deduplicate_boundary_case_insensitive() {
        let result = deduplicate_boundary("Hello World", "world foo");
        assert_eq!(result, "foo");
    }

    #[test]
    fn test_deduplicate_boundary_empty_previous() {
        let result = deduplicate_boundary("", "hello world");
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_deduplicate_boundary_empty_new() {
        let result = deduplicate_boundary("hello world", "");
        assert_eq!(result, "");
    }

    #[test]
    fn test_combine_chunk_results_empty() {
        let results: Vec<ChunkResult> = vec![];
        assert_eq!(combine_chunk_results(results), "");
    }

    #[test]
    fn test_combine_chunk_results_single() {
        let results = vec![ChunkResult {
            text: "hello world".to_string(),
            chunk_index: 0,
        }];
        assert_eq!(combine_chunk_results(results), "hello world");
    }

    #[test]
    fn test_combine_chunk_results_multiple_no_overlap() {
        let results = vec![
            ChunkResult {
                text: "hello world".to_string(),
                chunk_index: 0,
            },
            ChunkResult {
                text: "foo bar".to_string(),
                chunk_index: 1,
            },
        ];
        assert_eq!(combine_chunk_results(results), "hello world foo bar");
    }

    #[test]
    fn test_combine_chunk_results_with_overlap() {
        let results = vec![
            ChunkResult {
                text: "hello world foo".to_string(),
                chunk_index: 0,
            },
            ChunkResult {
                text: "foo bar baz".to_string(),
                chunk_index: 1,
            },
        ];
        assert_eq!(combine_chunk_results(results), "hello world foo bar baz");
    }

    #[test]
    fn test_combine_chunk_results_out_of_order() {
        // Results can arrive out of order; they should be sorted by chunk_index
        let results = vec![
            ChunkResult {
                text: "bar baz".to_string(),
                chunk_index: 1,
            },
            ChunkResult {
                text: "hello world bar".to_string(),
                chunk_index: 0,
            },
        ];
        assert_eq!(combine_chunk_results(results), "hello world bar baz");
    }

    #[test]
    fn test_combine_chunk_results_three_chunks() {
        let results = vec![
            ChunkResult {
                text: "one two three".to_string(),
                chunk_index: 0,
            },
            ChunkResult {
                text: "three four five".to_string(),
                chunk_index: 1,
            },
            ChunkResult {
                text: "five six seven".to_string(),
                chunk_index: 2,
            },
        ];
        assert_eq!(
            combine_chunk_results(results),
            "one two three four five six seven"
        );
    }
}
