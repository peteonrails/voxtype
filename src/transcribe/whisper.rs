//! Whisper-based speech-to-text transcription
//!
//! Uses whisper.cpp via the whisper-rs crate for fast, local transcription.

use super::Transcriber;
use crate::config::{Config, WhisperConfig};
use crate::error::TranscribeError;
use std::path::PathBuf;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Represents a single word segment with its metadata
#[derive(Debug, Clone)]
pub struct WordSegment {
    pub text: String,
    pub t0_cs: i64,  // start time in centiseconds
    pub t1_cs: i64,  // end time in centiseconds
    pub probability: f32,
    pub label: ConfidenceLabel,
}

/// Confidence label for a word segment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfidenceLabel {
    Red,
    Yellow,
    Green,
}

/// Detailed transcription result with word-level confidence
#[derive(Debug)]
pub struct TranscriptionDetails {
    pub text: String,
    pub segments: Vec<WordSegment>,
}

/// Whisper-based transcriber
pub struct WhisperTranscriber {
    /// Whisper context (holds the model)
    ctx: WhisperContext,
    /// Language for transcription
    language: String,
    /// Whether to translate to English
    translate: bool,
    /// Number of threads to use
    threads: usize,
}

impl WhisperTranscriber {
    /// Create a new whisper transcriber
    pub fn new(config: &WhisperConfig) -> Result<Self, TranscribeError> {
        let model_path = resolve_model_path(&config.model)?;

        tracing::info!("Loading whisper model from {:?}", model_path);
        let start = std::time::Instant::now();

        let ctx = WhisperContext::new_with_params(
            model_path
                .to_str()
                .ok_or_else(|| TranscribeError::ModelNotFound("Invalid path".to_string()))?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| TranscribeError::InitFailed(e.to_string()))?;

        tracing::info!("Model loaded in {:.2}s", start.elapsed().as_secs_f32());

        let threads = config
            .threads
            .unwrap_or_else(|| num_cpus::get().min(4));

        Ok(Self {
            ctx,
            language: config.language.clone(),
            translate: config.translate,
            threads,
        })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".to_string()));
        }

        let duration_secs = samples.len() as f32 / 16000.0;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples)",
            duration_secs,
            samples.len()
        );

        let start = std::time::Instant::now();

        // Create state for this transcription
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;

        // Configure parameters
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Set language (handle "auto" for auto-detection)
        if self.language == "auto" {
            // Pass None to enable auto-detection
            params.set_language(None);
        } else {
            params.set_language(Some(&self.language));
        }

        params.set_translate(self.translate);
        params.set_n_threads(self.threads as i32);

        // Disable output we don't need
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // Improve transcription quality
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);

        // For short recordings, use single segment mode
        if duration_secs < 30.0 {
            params.set_single_segment(true);
        }

        // Optimize context window for short clips
        if let Some(audio_ctx) = calculate_audio_ctx(duration_secs) {
            params.set_audio_ctx(audio_ctx);
            tracing::info!(
                "Audio context optimization: using audio_ctx={} for {:.2}s clip (formula: {:.2}s * 50 + 64)",
                audio_ctx,
                duration_secs,
                duration_secs
            );
        }

        // Run inference
        state
            .full(params, samples)
            .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;

        // Collect all segments using iterator API
        let mut text = String::new();
        for segment in state.as_iter() {
            text.push_str(
                segment
                    .to_str()
                    .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?,
            );
        }

        let result = text.trim().to_string();

        tracing::info!(
            "Transcription completed in {:.2}s: {:?}",
            start.elapsed().as_secs_f32(),
            if result.chars().count() > 50 {
                format!("{}...", result.chars().take(50).collect::<String>())
            } else {
                result.clone()
            }
        );

        Ok(result)
    }
}

impl WhisperTranscriber {
    /// Transcribe audio samples with word-level confidence information
    pub fn transcribe_with_confidence(&self, samples: &[f32]) -> Result<TranscriptionDetails, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".to_string()));
        }

        let duration_secs = samples.len() as f32 / 16000.0;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with confidence",
            duration_secs,
            samples.len()
        );

        let start = std::time::Instant::now();

        // Create state for this transcription
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;

        // Configure parameters
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Set language (handle "auto" for auto-detection)
        if self.language == "auto" {
            params.set_language(None);
        } else {
            params.set_language(Some(&self.language));
        }

        params.set_translate(self.translate);
        params.set_n_threads(self.threads as i32);

        // Disable output we don't need
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // Improve transcription quality
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);

        // Enable word-level segmentation
        params.set_token_timestamps(true);
        params.set_max_len(1);  // One word per segment
        params.set_split_on_word(true);

        // For short recordings, use single segment mode
        if duration_secs < 30.0 {
            params.set_single_segment(true);
        }

        // Optimize context window for short clips
        if let Some(audio_ctx) = calculate_audio_ctx(duration_secs) {
            params.set_audio_ctx(audio_ctx);
            tracing::info!(
                "Audio context optimization: using audio_ctx={} for {:.2}s clip",
                audio_ctx,
                duration_secs
            );
        }

        // Run inference
        state
            .full(params, samples)
            .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;

        // Collect segments with confidence information
        let mut segments = Vec::new();
        let mut text = String::new();

        for segment in state.as_iter() {
            let segment_text = segment
                .to_str()
                .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;

            // Skip empty segments
            if segment_text.trim().is_empty() {
                continue;
            }

            // Get timestamps (in centiseconds)
            let t0_cs = segment.start_timestamp();
            let t1_cs = segment.end_timestamp();

            // Calculate geometric mean of token probabilities
            let n_tokens = segment.n_tokens();
            let mut token_probs = Vec::with_capacity(n_tokens as usize);
            for i in 0..n_tokens {
                if let Some(token) = segment.get_token(i) {
                    token_probs.push(token.token_probability());
                }
            }

            let probability = if token_probs.is_empty() {
                f32::NAN
            } else {
                geometric_mean(&token_probs)
            };

            let label = probability_to_label(probability);

            segments.push(WordSegment {
                text: segment_text.to_string(),
                t0_cs,
                t1_cs,
                probability,
                label,
            });

            text.push_str(segment_text);
        }

        let result_text = text.trim().to_string();

        tracing::info!(
            "Transcription completed in {:.2}s: {} words",
            start.elapsed().as_secs_f32(),
            segments.len()
        );

        Ok(TranscriptionDetails {
            text: result_text,
            segments,
        })
    }
}

/// Resolve model name to file path
fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    // If it's already an absolute path, use it directly
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    // Map model names to file names
    let model_filename = match model {
        "tiny" => "ggml-tiny.bin",
        "tiny.en" => "ggml-tiny.en.bin",
        "base" => "ggml-base.bin",
        "base.en" => "ggml-base.en.bin",
        "small" => "ggml-small.bin",
        "small.en" => "ggml-small.en.bin",
        "medium" => "ggml-medium.bin",
        "medium.en" => "ggml-medium.en.bin",
        "large" | "large-v1" => "ggml-large-v1.bin",
        "large-v2" => "ggml-large-v2.bin",
        "large-v3" => "ggml-large-v3.bin",
        "large-v3-turbo" => "ggml-large-v3-turbo.bin",
        // If it looks like a filename, use it as-is
        other if other.ends_with(".bin") => other,
        // Otherwise, assume it's a model name and add prefix/suffix
        other => {
            return Err(TranscribeError::ModelNotFound(format!(
                "Unknown model: '{}'. Valid models: tiny, base, small, medium, large-v3, large-v3-turbo",
                other
            )));
        }
    };

    // Look in the data directory
    let models_dir = Config::models_dir();
    let model_path = models_dir.join(model_filename);

    if model_path.exists() {
        return Ok(model_path);
    }

    // Also check current directory
    let cwd_path = PathBuf::from(model_filename);
    if cwd_path.exists() {
        return Ok(cwd_path);
    }

    // Also check ./models/
    let local_models_path = PathBuf::from("models").join(model_filename);
    if local_models_path.exists() {
        return Ok(local_models_path);
    }

    Err(TranscribeError::ModelNotFound(format!(
        "Model '{}' not found. Looked in:\n  - {}\n  - {}\n  - {}\n\nDownload from: https://huggingface.co/ggerganov/whisper.cpp/tree/main",
        model,
        model_path.display(),
        cwd_path.display(),
        local_models_path.display()
    )))
}

/// Calculate audio_ctx parameter for short clips (≤22.5s).
/// Formula: duration_seconds * 50 + 64
fn calculate_audio_ctx(duration_secs: f32) -> Option<i32> {
    if duration_secs <= 22.5 {
        Some((duration_secs * 50.0) as i32 + 64)
    } else {
        None
    }
}

/// Get the filename for a model
pub fn get_model_filename(model: &str) -> String {
    match model {
        "tiny" => "ggml-tiny.bin",
        "tiny.en" => "ggml-tiny.en.bin",
        "base" => "ggml-base.bin",
        "base.en" => "ggml-base.en.bin",
        "small" => "ggml-small.bin",
        "small.en" => "ggml-small.en.bin",
        "medium" => "ggml-medium.bin",
        "medium.en" => "ggml-medium.en.bin",
        "large-v3" => "ggml-large-v3.bin",
        "large-v3-turbo" => "ggml-large-v3-turbo.bin",
        other => other,
    }
    .to_string()
}

/// Get the download URL for a model
pub fn get_model_url(model: &str) -> String {
    let filename = get_model_filename(model);

    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        filename
    )
}

/// Map a probability value to a confidence label
fn probability_to_label(probability: f32) -> ConfidenceLabel {
    if probability.is_nan() {
        return ConfidenceLabel::Yellow;
    }
    if probability < 0.33 {
        ConfidenceLabel::Red
    } else if probability < 0.66 {
        ConfidenceLabel::Yellow
    } else {
        ConfidenceLabel::Green
    }
}

/// Calculate geometric mean of token probabilities
fn geometric_mean(probabilities: &[f32]) -> f32 {
    if probabilities.is_empty() {
        return f32::NAN;
    }
    let product: f32 = probabilities.iter().product();
    product.powf(1.0 / probabilities.len() as f32)
}

/// Confidence breakdown for sentence-level scoring
#[derive(Debug, Clone)]
pub struct ConfidenceBreakdown {
    pub avg_conf: f32,
    pub min_conf: f32,
    pub red_ratio: f32,
}

/// Minimum confidence threshold below which a sentence requires retry
const SENTENCE_RETRY_MIN_CONF_THRESHOLD: f32 = 0.33;

/// Cluster consecutive word segments into sentences based on punctuation delimiters.
///
/// Sentences are delimited by punctuation marks: `.`, `!`, `?`
pub fn cluster_into_sentences(segments: &[WordSegment]) -> Vec<Vec<WordSegment>> {
    if segments.is_empty() {
        return Vec::new();
    }

    let mut sentences: Vec<Vec<WordSegment>> = Vec::new();
    let mut current_sentence: Vec<WordSegment> = Vec::new();

    for segment in segments {
        current_sentence.push(segment.clone());

        // Check if the segment text ends with sentence-ending punctuation
        let text = segment.text.trim();
        if !text.is_empty() && text.ends_with(|c: char| c == '.' || c == '!' || c == '?') {
            // End of sentence - save current sentence and start new one
            if !current_sentence.is_empty() {
                sentences.push(current_sentence);
                current_sentence = Vec::new();
            }
        }
    }

    // Add any remaining segments as the last sentence
    if !current_sentence.is_empty() {
        sentences.push(current_sentence);
    }

    sentences
}

/// Calculate weighted confidence score for a sentence.
///
/// Uses weighted combination:
/// - Average confidence (40% weight)
/// - Minimum confidence (30% weight)
/// - Red word ratio (30% weight)
///
/// Returns tuple of (weighted_score, breakdown)
pub fn calculate_sentence_confidence(sentence: &[WordSegment]) -> (f32, ConfidenceBreakdown) {
    if sentence.is_empty() {
        return (
            0.0,
            ConfidenceBreakdown {
                avg_conf: 0.0,
                min_conf: 0.0,
                red_ratio: 1.0,
            },
        );
    }

    // Filter out empty/whitespace-only segments and NaN probabilities
    let probabilities: Vec<f32> = sentence
        .iter()
        .filter_map(|seg| {
            if seg.probability.is_nan() || seg.text.trim().is_empty() {
                None
            } else {
                Some(seg.probability)
            }
        })
        .collect();

    if probabilities.is_empty() {
        // All NaN probabilities or all empty segments - treat as low confidence
        return (
            0.0,
            ConfidenceBreakdown {
                avg_conf: 0.0,
                min_conf: 0.0,
                red_ratio: 1.0,
            },
        );
    }

    let avg_conf = probabilities.iter().sum::<f32>() / probabilities.len() as f32;
    let min_conf = probabilities
        .iter()
        .copied()
        .reduce(f32::min)
        .unwrap_or(0.0);

    // Only count non-empty segments for red ratio
    let non_empty_segments: Vec<&WordSegment> = sentence
        .iter()
        .filter(|seg| !seg.text.trim().is_empty())
        .collect();

    let red_count = non_empty_segments
        .iter()
        .filter(|seg| seg.label == ConfidenceLabel::Red)
        .count();

    let red_ratio = if non_empty_segments.is_empty() {
        1.0
    } else {
        red_count as f32 / non_empty_segments.len() as f32
    };

    // Weighted score: higher is better
    // Normalize components to [0, 1] range where 1 is best
    let weighted_score = 0.4 * avg_conf +           // Average confidence (already 0-1)
                         0.3 * min_conf +            // Minimum confidence (already 0-1)
                         0.3 * (1.0 - red_ratio);   // Inverse red ratio (1 - red_ratio gives higher score for fewer reds)

    let breakdown = ConfidenceBreakdown {
        avg_conf,
        min_conf,
        red_ratio,
    };

    (weighted_score, breakdown)
}

/// Decide whether a sentence needs retry based on minimum confidence threshold.
///
/// Current rule (simple, threshold-based): retry if min_conf < SENTENCE_RETRY_MIN_CONF_THRESHOLD.
/// Weighted score is still computed (returned) for visibility/diagnostics, but is not used for the decision.
///
/// Returns tuple of (needs_retry, weighted_score, breakdown)
pub fn sentence_needs_retry(sentence: &[WordSegment]) -> (bool, f32, ConfidenceBreakdown) {
    let (score, breakdown) = calculate_sentence_confidence(sentence);
    let needs_retry = breakdown.min_conf < SENTENCE_RETRY_MIN_CONF_THRESHOLD;
    (needs_retry, score, breakdown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_url() {
        let url = get_model_url("base.en");
        assert!(url.contains("ggml-base.en.bin"));
        assert!(url.contains("huggingface.co"));
    }
}
