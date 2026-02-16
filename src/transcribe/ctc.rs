//! Shared CTC (Connectionist Temporal Classification) greedy decoding
//!
//! Used by SenseVoice, Dolphin, and Omnilingual backends. These models
//! all use CTC output and share the same decoding logic: argmax per frame,
//! collapse consecutive duplicates, remove blank tokens.
//!
//! SenseVoice additionally skips metadata tokens (language, emotion, event, ITN)
//! at the start of the sequence.

use crate::error::TranscribeError;
use std::collections::HashMap;
use std::path::Path;

/// Configuration for CTC greedy decoding
pub struct CtcConfig {
    /// Token ID used for CTC blank (usually 0)
    pub blank_id: u32,
    /// Number of metadata tokens to skip at start of decoded sequence
    /// (SenseVoice: 4 for language/emotion/event/ITN, others: 0)
    pub num_metadata_tokens: usize,
    /// Replace SentencePiece word boundary markers (U+2581) with spaces
    pub sentencepiece_cleanup: bool,
}

impl Default for CtcConfig {
    fn default() -> Self {
        Self {
            blank_id: 0,
            num_metadata_tokens: 0,
            sentencepiece_cleanup: false,
        }
    }
}

impl CtcConfig {
    /// Config for SenseVoice: skip 4 metadata tokens, clean SentencePiece markers
    pub fn sensevoice() -> Self {
        Self {
            blank_id: 0,
            num_metadata_tokens: 4,
            sentencepiece_cleanup: true,
        }
    }
}

/// CTC greedy decoding: argmax per frame, collapse duplicates, remove blanks
///
/// Input: raw logits of shape (time_steps, vocab_size) flattened to a 1D slice
/// Output: decoded text string
pub fn ctc_greedy_decode(
    logits: &[f32],
    time_steps: usize,
    vocab_size: usize,
    tokens: &HashMap<u32, String>,
    config: &CtcConfig,
) -> String {
    let mut token_ids: Vec<u32> = Vec::new();
    let mut prev_id: Option<u32> = None;

    for t in 0..time_steps {
        let offset = t * vocab_size;
        let frame_logits = &logits[offset..offset + vocab_size];

        // Argmax
        let best_id = frame_logits
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx as u32)
            .unwrap_or(config.blank_id);

        // Collapse consecutive duplicates and skip blanks
        if best_id != config.blank_id && Some(best_id) != prev_id {
            token_ids.push(best_id);
        }
        prev_id = Some(best_id);
    }

    tokens_to_string(&token_ids, tokens, config)
}

/// Decode pre-argmaxed output where values are already token IDs (as f32)
///
/// Some ONNX models output 2D logits where each value is already the best
/// token ID rather than a probability distribution over the vocabulary.
pub fn decode_pre_argmax(
    token_ids_f32: &[f32],
    tokens: &HashMap<u32, String>,
    config: &CtcConfig,
) -> String {
    let mut token_ids: Vec<u32> = Vec::new();
    let mut prev_id: Option<u32> = None;

    for &val in token_ids_f32 {
        let id = val as u32;
        if id != config.blank_id && Some(id) != prev_id {
            token_ids.push(id);
        }
        prev_id = Some(id);
    }

    tokens_to_string(&token_ids, tokens, config)
}

/// Convert token IDs to string, applying metadata skipping and SentencePiece cleanup
fn tokens_to_string(
    token_ids: &[u32],
    tokens: &HashMap<u32, String>,
    config: &CtcConfig,
) -> String {
    let content_tokens = if token_ids.len() > config.num_metadata_tokens {
        &token_ids[config.num_metadata_tokens..]
    } else if config.num_metadata_tokens > 0 {
        &[]
    } else {
        token_ids
    };

    let mut result = String::new();
    for &id in content_tokens {
        if let Some(token_str) = tokens.get(&id) {
            if config.sentencepiece_cleanup {
                result.push_str(&token_str.replace('\u{2581}', " "));
            } else {
                result.push_str(token_str);
            }
        }
    }

    result.trim().to_string()
}

/// Load tokens.txt into a HashMap<u32, String>
///
/// Format: each line is "token_string token_id" (space-separated).
/// The token string may contain spaces, so we split from the right.
pub fn load_tokens(path: &Path) -> Result<HashMap<u32, String>, TranscribeError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        TranscribeError::InitFailed(format!("Failed to read tokens.txt: {}", e))
    })?;

    let mut tokens = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Split from the right to handle tokens containing spaces
        if let Some(last_space) = line.rfind(' ') {
            let token_str = &line[..last_space];
            let id_str = &line[last_space + 1..];
            if let Ok(id) = id_str.parse::<u32>() {
                tokens.insert(id, token_str.to_string());
            }
        }
    }

    if tokens.is_empty() {
        return Err(TranscribeError::InitFailed(
            "tokens.txt appears empty or malformed".to_string(),
        ));
    }

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_load_tokens() {
        let temp_dir = TempDir::new().unwrap();
        let tokens_path = temp_dir.path().join("tokens.txt");
        fs::write(
            &tokens_path,
            "<blank> 0\n<sos/eos> 1\nhello 2\nworld 3\n",
        )
        .unwrap();

        let tokens = load_tokens(&tokens_path).unwrap();
        assert_eq!(tokens.get(&0), Some(&"<blank>".to_string()));
        assert_eq!(tokens.get(&2), Some(&"hello".to_string()));
        assert_eq!(tokens.get(&3), Some(&"world".to_string()));
    }

    #[test]
    fn test_load_tokens_empty() {
        let temp_dir = TempDir::new().unwrap();
        let tokens_path = temp_dir.path().join("tokens.txt");
        fs::write(&tokens_path, "").unwrap();

        let result = load_tokens(&tokens_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_ctc_decode_basic() {
        let mut tokens = HashMap::new();
        tokens.insert(1, "h".to_string());
        tokens.insert(2, "i".to_string());

        // Simulate: blank, h, h, blank, i
        let vocab_size = 3;
        let time_steps = 5;
        let mut logits = vec![0.0f32; time_steps * vocab_size];

        let set_max = |logits: &mut Vec<f32>, t: usize, id: usize| {
            logits[t * vocab_size + id] = 10.0;
        };

        set_max(&mut logits, 0, 0); // blank
        set_max(&mut logits, 1, 1); // h
        set_max(&mut logits, 2, 1); // h (duplicate)
        set_max(&mut logits, 3, 0); // blank
        set_max(&mut logits, 4, 2); // i

        let config = CtcConfig::default();
        let result = ctc_greedy_decode(&logits, time_steps, vocab_size, &tokens, &config);
        assert_eq!(result, "hi");
    }

    #[test]
    fn test_ctc_decode_with_metadata_skip() {
        let mut tokens = HashMap::new();
        tokens.insert(1, "lang".to_string());
        tokens.insert(2, "emo".to_string());
        tokens.insert(3, "event".to_string());
        tokens.insert(4, "itn".to_string());
        tokens.insert(5, "h".to_string());
        tokens.insert(6, "i".to_string());

        let vocab_size = 7;
        let time_steps = 6;
        let mut logits = vec![0.0f32; time_steps * vocab_size];

        let set_max = |logits: &mut Vec<f32>, t: usize, id: usize| {
            logits[t * vocab_size + id] = 10.0;
        };

        set_max(&mut logits, 0, 1); // lang (metadata)
        set_max(&mut logits, 1, 2); // emo (metadata)
        set_max(&mut logits, 2, 3); // event (metadata)
        set_max(&mut logits, 3, 4); // itn (metadata)
        set_max(&mut logits, 4, 5); // h
        set_max(&mut logits, 5, 6); // i

        let config = CtcConfig::sensevoice();
        let result = ctc_greedy_decode(&logits, time_steps, vocab_size, &tokens, &config);
        assert_eq!(result, "hi");
    }

    #[test]
    fn test_ctc_decode_sentencepiece_cleanup() {
        let mut tokens = HashMap::new();
        tokens.insert(1, "\u{2581}hello".to_string());
        tokens.insert(2, "\u{2581}world".to_string());

        let vocab_size = 3;
        let time_steps = 2;
        let mut logits = vec![0.0f32; time_steps * vocab_size];

        logits[0 * vocab_size + 1] = 10.0; // hello
        logits[1 * vocab_size + 2] = 10.0; // world

        let config = CtcConfig {
            sentencepiece_cleanup: true,
            ..CtcConfig::default()
        };
        let result = ctc_greedy_decode(&logits, time_steps, vocab_size, &tokens, &config);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_decode_pre_argmax() {
        let mut tokens = HashMap::new();
        tokens.insert(1, "a".to_string());
        tokens.insert(2, "b".to_string());

        // Pre-argmaxed: blank, a, a, blank, b
        let token_ids: Vec<f32> = vec![0.0, 1.0, 1.0, 0.0, 2.0];
        let config = CtcConfig::default();
        let result = decode_pre_argmax(&token_ids, &tokens, &config);
        assert_eq!(result, "ab");
    }
}
