//! Shared CTC (Connectionist Temporal Classification) greedy decoder
//!
//! Used by CTC-based ASR engines: SenseVoice, Paraformer, Dolphin, Omnilingual.
//! Performs argmax per frame, consecutive duplicate removal, blank token removal,
//! and SentencePiece marker cleanup.

use std::collections::HashMap;

/// CTC greedy decode from logits
///
/// - `logits`: flat array of shape [time_steps, vocab_size]
/// - `time_steps`: number of time frames
/// - `vocab_size`: vocabulary size
/// - `blank_id`: blank token ID (typically 0)
/// - `tokens`: token ID to string mapping
///
/// Returns decoded text with SentencePiece markers (U+2581) replaced by spaces.
pub fn ctc_greedy_decode(
    logits: &[f32],
    time_steps: usize,
    vocab_size: usize,
    blank_id: u32,
    tokens: &HashMap<u32, String>,
) -> String {
    let token_ids = ctc_greedy_search(logits, time_steps, vocab_size, blank_id);
    tokens_to_string(&token_ids, tokens)
}

/// CTC greedy search: argmax per frame, collapse consecutive duplicates, remove blanks
///
/// Returns the deduplicated, non-blank token ID sequence.
pub fn ctc_greedy_search(
    logits: &[f32],
    time_steps: usize,
    vocab_size: usize,
    blank_id: u32,
) -> Vec<u32> {
    let mut token_ids: Vec<u32> = Vec::new();
    let mut prev_id: Option<u32> = None;

    for t in 0..time_steps {
        let offset = t * vocab_size;
        let frame_logits = &logits[offset..offset + vocab_size];

        let best_id = frame_logits
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx as u32)
            .unwrap_or(blank_id);

        if best_id != blank_id && Some(best_id) != prev_id {
            token_ids.push(best_id);
        }
        prev_id = Some(best_id);
    }

    token_ids
}

/// Convert token IDs to text string
///
/// Replaces SentencePiece word boundary markers (U+2581) with spaces.
pub fn tokens_to_string(token_ids: &[u32], tokens: &HashMap<u32, String>) -> String {
    let mut result = String::new();
    for &id in token_ids {
        if let Some(token_str) = tokens.get(&id) {
            result.push_str(&token_str.replace('\u{2581}', " "));
        }
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tokens() -> HashMap<u32, String> {
        let mut tokens = HashMap::new();
        tokens.insert(0, "<blank>".to_string());
        tokens.insert(1, "\u{2581}hello".to_string());
        tokens.insert(2, "\u{2581}world".to_string());
        tokens.insert(3, "!".to_string());
        tokens
    }

    #[test]
    fn test_ctc_greedy_search_basic() {
        let vocab_size = 4;
        let time_steps = 7;
        let mut logits = vec![0.0f32; time_steps * vocab_size];

        // Frame 0: blank
        logits[0 * vocab_size + 0] = 10.0;
        // Frame 1: "hello" (id=1)
        logits[1 * vocab_size + 1] = 10.0;
        // Frame 2: "hello" again (duplicate, should be collapsed)
        logits[2 * vocab_size + 1] = 10.0;
        // Frame 3: blank
        logits[3 * vocab_size + 0] = 10.0;
        // Frame 4: "world" (id=2)
        logits[4 * vocab_size + 2] = 10.0;
        // Frame 5: blank
        logits[5 * vocab_size + 0] = 10.0;
        // Frame 6: "!" (id=3)
        logits[6 * vocab_size + 3] = 10.0;

        let ids = ctc_greedy_search(&logits, time_steps, vocab_size, 0);
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn test_ctc_greedy_decode() {
        let tokens = make_tokens();
        let vocab_size = 4;
        let time_steps = 5;
        let mut logits = vec![0.0f32; time_steps * vocab_size];

        logits[0 * vocab_size + 0] = 10.0; // blank
        logits[1 * vocab_size + 1] = 10.0; // hello
        logits[2 * vocab_size + 0] = 10.0; // blank
        logits[3 * vocab_size + 2] = 10.0; // world
        logits[4 * vocab_size + 3] = 10.0; // !

        let text = ctc_greedy_decode(&logits, time_steps, vocab_size, 0, &tokens);
        assert_eq!(text, "hello world!");
    }

    #[test]
    fn test_tokens_to_string_sentencepiece() {
        let tokens = make_tokens();
        let ids = vec![1, 2];
        let text = tokens_to_string(&ids, &tokens);
        assert_eq!(text, "hello world");
    }

    #[test]
    fn test_ctc_greedy_search_all_blank() {
        let vocab_size = 3;
        let time_steps = 5;
        let mut logits = vec![0.0f32; time_steps * vocab_size];
        for t in 0..time_steps {
            logits[t * vocab_size + 0] = 10.0; // all blanks
        }
        let ids = ctc_greedy_search(&logits, time_steps, vocab_size, 0);
        assert!(ids.is_empty());
    }

    #[test]
    fn test_ctc_repeated_non_blank() {
        // Same token repeated with blank in between should produce two copies
        let vocab_size = 3;
        let time_steps = 5;
        let mut logits = vec![0.0f32; time_steps * vocab_size];

        logits[0 * vocab_size + 1] = 10.0; // token 1
        logits[1 * vocab_size + 0] = 10.0; // blank
        logits[2 * vocab_size + 1] = 10.0; // token 1 again (not collapsed because blank separates)
        logits[3 * vocab_size + 0] = 10.0; // blank
        logits[4 * vocab_size + 2] = 10.0; // token 2

        let ids = ctc_greedy_search(&logits, time_steps, vocab_size, 0);
        assert_eq!(ids, vec![1, 1, 2]);
    }
}
