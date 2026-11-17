//! Multilingual punctuation restoration and true-casing for plain CTC text.
//!
//! The model is a token classifier, not a text generator. It can only add
//! punctuation, capitalization and sentence boundaries to the input tokens.

use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;
use std::sync::Mutex;
use tokenizers::Tokenizer;

pub const MODEL_DIR: &str = "xlm-roberta-punctuation-fullstop-truecase";
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "af", "am", "ar", "bg", "bn", "de", "el", "en", "es", "et", "fa", "fi", "fr", "gu", "hi", "hr",
    "hu", "id", "is", "it", "ja", "kk", "kn", "ko", "ky", "lt", "lv", "mk", "ml", "mr", "nl", "or",
    "pa", "pl", "ps", "pt", "ro", "ru", "rw", "so", "sr", "sw", "ta", "te", "tr", "uk", "zh",
];

const MAX_LENGTH: usize = 256;
const PRE_LABELS: [&str; 2] = ["", "¿"];
const POST_LABELS: [&str; 17] = [
    "",
    "<ACRONYM>",
    ".",
    ",",
    "?",
    "？",
    "，",
    "。",
    "、",
    "・",
    "।",
    "؟",
    "،",
    ";",
    "።",
    "፣",
    "፧",
];

pub fn supports_language(language: Option<&str>) -> bool {
    language.is_none_or(|language| {
        let base = language.split(['-', '_']).next().unwrap_or(language);
        SUPPORTED_LANGUAGES.contains(&base)
    })
}

/// Resident punctuation model. A mutex is required because ONNX Runtime's
/// session run method is mutable; text processing itself remains callable
/// through `&TextProcessor` on both batch and streaming output paths.
pub struct PunctuationCaser {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

impl PunctuationCaser {
    pub fn from_dir(model_dir: &Path) -> Result<Self, TranscribeError> {
        let model_path = model_dir.join("model.onnx");
        let tokenizer_path = model_dir.join("tokenizer.json");
        for (label, path) in [
            ("model.onnx", &model_path),
            ("tokenizer.json", &tokenizer_path),
        ] {
            if !path.exists() {
                return Err(TranscribeError::ModelNotFound(format!(
                    "Punctuation/casing {label} not found at {}.\n  Run: voxtype setup punctuation",
                    path.display()
                )));
            }
        }

        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            TranscribeError::InitFailed(format!(
                "Failed to load punctuation tokenizer from {}: {e}",
                tokenizer_path.display()
            ))
        })?;
        let session = Session::builder()
            .map_err(|e| TranscribeError::InitFailed(format!("ONNX session builder failed: {e}")))?
            .with_intra_threads(num_cpus::get().min(4))
            .map_err(|e| {
                TranscribeError::InitFailed(format!("Failed to set punctuation threads: {e}"))
            })?
            .commit_from_file(&model_path)
            .map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load punctuation model from {}: {e}",
                    model_path.display()
                ))
            })?;
        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
        })
    }

    pub fn restore(&self, text: &str) -> Result<String, TranscribeError> {
        if text.trim().is_empty() {
            return Ok(text.to_string());
        }

        // The checkpoint was trained on lowercase, unpunctuated input. This
        // stage is capability-gated, so lowercasing cannot discard native ASR
        // casing from Whisper, Parakeet TDT, Cohere, or remote engines.
        let input = text.trim().to_lowercase();
        let encoding = self.tokenizer.encode(input, true).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Punctuation tokenization failed: {e}"))
        })?;
        let ids = encoding.get_ids();
        let tokens = encoding.get_tokens();
        if ids.len() < 2 {
            return Ok(text.to_string());
        }

        let bos = ids[0];
        let eos = *ids.last().expect("length checked above");
        let mut restored = Vec::new();
        for start in (1..ids.len() - 1).step_by(MAX_LENGTH - 2) {
            let end = (start + MAX_LENGTH - 2).min(ids.len() - 1);
            let mut chunk_ids = Vec::with_capacity(end - start + 2);
            chunk_ids.push(bos as i64);
            chunk_ids.extend(ids[start..end].iter().map(|&id| id as i64));
            chunk_ids.push(eos as i64);
            restored.push(self.restore_chunk(&chunk_ids, &tokens[start..end])?);
        }
        Ok(restored.join(" "))
    }

    fn restore_chunk(&self, ids: &[i64], tokens: &[String]) -> Result<String, TranscribeError> {
        let input = Tensor::from_array(([1, ids.len()], ids.to_vec())).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to build punctuation input: {e}"))
        })?;
        let mut session = self.session.lock().map_err(|_| {
            TranscribeError::InferenceFailed("Punctuation model lock was poisoned".to_string())
        })?;
        let outputs = session
            .run(ort::inputs!["input_ids" => input])
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Punctuation inference failed: {e}"))
            })?;

        let (_, pre) = outputs[0].try_extract_tensor::<i64>().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Invalid pre-punctuation output: {e}"))
        })?;
        let (_, post) = outputs[1].try_extract_tensor::<i64>().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Invalid post-punctuation output: {e}"))
        })?;
        let (cap_shape, caps) = outputs[2].try_extract_tensor::<bool>().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Invalid true-casing output: {e}"))
        })?;
        let (_, boundaries) = outputs[3].try_extract_tensor::<bool>().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Invalid sentence-boundary output: {e}"))
        })?;
        let cap_width = cap_shape.last().copied().unwrap_or(0) as usize;
        if cap_width == 0 {
            return Err(TranscribeError::InferenceFailed(
                "Punctuation model returned an empty true-casing dimension".to_string(),
            ));
        }
        let prediction_range = 1..tokens.len() + 1;
        let pre = pre.get(prediction_range.clone()).ok_or_else(|| {
            TranscribeError::InferenceFailed(
                "Punctuation model returned short pre-punctuation output".to_string(),
            )
        })?;
        let post = post.get(prediction_range.clone()).ok_or_else(|| {
            TranscribeError::InferenceFailed(
                "Punctuation model returned short post-punctuation output".to_string(),
            )
        })?;
        let boundaries = boundaries.get(prediction_range).ok_or_else(|| {
            TranscribeError::InferenceFailed(
                "Punctuation model returned short sentence-boundary output".to_string(),
            )
        })?;
        let caps = caps
            .get(cap_width..cap_width.saturating_mul(tokens.len() + 1))
            .ok_or_else(|| {
                TranscribeError::InferenceFailed(
                    "Punctuation model returned short true-casing output".to_string(),
                )
            })?;
        apply_predictions(tokens, pre, post, caps, cap_width, boundaries)
    }
}

fn apply_predictions(
    tokens: &[String],
    pre: &[i64],
    post: &[i64],
    caps: &[bool],
    cap_width: usize,
    boundaries: &[bool],
) -> Result<String, TranscribeError> {
    if pre.len() < tokens.len()
        || post.len() < tokens.len()
        || boundaries.len() < tokens.len()
        || caps.len() < tokens.len().saturating_mul(cap_width)
    {
        return Err(TranscribeError::InferenceFailed(
            "Punctuation model returned shorter predictions than its input".to_string(),
        ));
    }

    let mut out = String::new();
    for (idx, token) in tokens.iter().enumerate() {
        let leading_space = token.starts_with('▁') && !out.is_empty() && !out.ends_with(' ');
        if leading_space {
            out.push(' ');
        }
        let pre_label = PRE_LABELS.get(pre[idx] as usize).ok_or_else(|| {
            TranscribeError::InferenceFailed(format!("Unknown pre-punctuation label {}", pre[idx]))
        })?;
        out.push_str(pre_label);

        let post_label = POST_LABELS.get(post[idx] as usize).ok_or_else(|| {
            TranscribeError::InferenceFailed(format!(
                "Unknown post-punctuation label {}",
                post[idx]
            ))
        })?;
        let piece = token.strip_prefix('▁').unwrap_or(token);
        for (char_idx, ch) in piece.chars().enumerate() {
            if char_idx < cap_width && caps[idx * cap_width + char_idx] {
                out.extend(ch.to_uppercase());
            } else {
                out.push(ch);
            }
            if *post_label == "<ACRONYM>" {
                out.push('.');
            }
        }
        if *post_label != "<ACRONYM>" {
            out.push_str(post_label);
        }
        if boundaries[idx] && idx + 1 < tokens.len() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    Ok(out.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_punctuation_casing_and_sentence_boundaries() {
        let tokens = vec!["▁hello".to_string(), "▁friend".to_string()];
        let mut caps = vec![false; 2 * 8];
        caps[0] = true;
        assert_eq!(
            apply_predictions(&tokens, &[0, 0], &[0, 4], &caps, 8, &[false, true]).unwrap(),
            "Hello friend?"
        );
    }

    #[test]
    fn applies_pre_punctuation_and_acronyms() {
        let tokens = vec!["▁como".to_string(), "▁pm".to_string()];
        assert_eq!(
            apply_predictions(&tokens, &[1, 0], &[0, 1], &[false; 16], 8, &[false, true]).unwrap(),
            "¿como p.m."
        );
    }

    #[test]
    fn sentence_boundaries_do_not_duplicate_spaces() {
        let tokens = vec!["▁one".to_string(), "▁two".to_string()];
        assert_eq!(
            apply_predictions(&tokens, &[0, 0], &[2, 2], &[false; 16], 8, &[true, true]).unwrap(),
            "one. two."
        );
    }

    #[test]
    fn language_support_accepts_regions_and_unknown_detection() {
        assert!(supports_language(Some("pt-BR")));
        assert!(supports_language(None));
        assert!(!supports_language(Some("vi")));
    }
}
