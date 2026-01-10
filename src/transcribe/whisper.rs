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
    pub t0_cs: i64, // start time in centiseconds
    pub t1_cs: i64, // end time in centiseconds
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
    /// Optional initial prompt for context-aware transcription
    initial_prompt: Option<String>,
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

        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));

        Ok(Self {
            ctx,
            language: config.language.clone(),
            translate: config.translate,
            threads,
            initial_prompt: config.initial_prompt.clone(),
        })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
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

        // Set initial prompt if configured
        if let Some(ref prompt) = self.initial_prompt {
            params.set_initial_prompt(prompt);
            tracing::debug!("Using initial prompt for transcription");
        }

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
    pub fn transcribe_with_confidence(
        &self,
        samples: &[f32],
        initial_prompt: Option<&str>,
    ) -> Result<TranscriptionDetails, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
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

        // Set initial prompt (prefer passed prompt, fall back to config)
        let prompt_to_use = initial_prompt.or(self.initial_prompt.as_deref());
        if let Some(prompt) = prompt_to_use {
            params.set_initial_prompt(prompt);
            tracing::debug!("Using initial prompt for transcription with confidence");
        }

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
        params.set_max_len(1); // One word per segment
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

    /// Transcribe a specific segment of audio (for retry passes in hybrid mode).
    ///
    /// Extracts the audio samples for the given time range, transcribes them,
    /// and adjusts timestamps to be relative to the original audio (not the segment start).
    ///
    /// Args:
    /// - samples: Full audio samples (16kHz mono)
    /// - start_ms: Start time in milliseconds (relative to full audio)
    /// - end_ms: End time in milliseconds (relative to full audio)
    /// - initial_prompt: Optional prompt for context-aware transcription
    pub fn transcribe_segment(
        &self,
        samples: &[f32],
        start_ms: i64,
        end_ms: i64,
        initial_prompt: Option<&str>,
    ) -> Result<TranscriptionDetails, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        const SAMPLE_RATE: u32 = 16000;
        let start_idx = ((start_ms as f64 / 1000.0) * SAMPLE_RATE as f64) as usize;
        let end_idx = ((end_ms as f64 / 1000.0) * SAMPLE_RATE as f64) as usize;

        // Clamp indices to valid range
        let start_idx = start_idx.min(samples.len());
        let end_idx = end_idx.min(samples.len()).max(start_idx);

        if start_idx >= samples.len() || end_idx <= start_idx {
            return Err(TranscribeError::AudioFormat(format!(
                "Invalid segment range: {}ms - {}ms",
                start_ms, end_ms
            )));
        }

        // Extract segment samples
        let segment_samples = &samples[start_idx..end_idx];
        let segment_duration_secs = segment_samples.len() as f32 / SAMPLE_RATE as f32;

        tracing::debug!(
            "Transcribing segment: {}ms - {}ms ({:.2}s, {} samples)",
            start_ms,
            end_ms,
            segment_duration_secs,
            segment_samples.len()
        );

        let start = std::time::Instant::now();

        // Create state for this transcription
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;

        // Configure parameters (same as transcribe_with_confidence)
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Set language (handle "auto" for auto-detection)
        if self.language == "auto" {
            params.set_language(None);
        } else {
            params.set_language(Some(&self.language));
        }

        params.set_translate(self.translate);
        params.set_n_threads(self.threads as i32);

        // Set initial prompt (use parameter if provided, otherwise use config)
        let prompt_to_use = initial_prompt.or(self.initial_prompt.as_deref());
        if let Some(prompt) = prompt_to_use {
            params.set_initial_prompt(prompt);
            tracing::debug!("Using initial prompt for segment transcription");
        }

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
        params.set_max_len(1); // One word per segment
        params.set_split_on_word(true);

        // For short recordings, use single segment mode
        if segment_duration_secs < 30.0 {
            params.set_single_segment(true);
        }

        // Optimize context window for short clips
        if let Some(audio_ctx) = calculate_audio_ctx(segment_duration_secs) {
            params.set_audio_ctx(audio_ctx);
        }

        // Run inference
        state
            .full(params, segment_samples)
            .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;

        // Collect segments with confidence information
        let mut segments = Vec::new();
        let mut text = String::new();

        // Offset to adjust timestamps back to original audio timeline
        let offset_cs = (start_ms / 10) as i64;

        for segment in state.as_iter() {
            let segment_text = segment
                .to_str()
                .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;

            // Skip empty segments
            if segment_text.trim().is_empty() {
                continue;
            }

            // Get timestamps (in centiseconds) and adjust to original timeline
            let t0_cs = segment.start_timestamp() + offset_cs;
            let t1_cs = segment.end_timestamp() + offset_cs;

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

        tracing::debug!(
            "Segment transcription completed in {:.2}s: {} words",
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
                         0.3 * (1.0 - red_ratio); // Inverse red ratio (1 - red_ratio gives higher score for fewer reds)

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

/// Identifies time ranges needing retry based on sentence confidence.
///
/// Clusters word segments into sentences, identifies low-confidence sentences,
/// and returns timestamp ranges with padding. Overlapping/adjacent ranges are merged.
///
/// Args:
/// - segments: Word segments to analyze
/// - min_conf_threshold: Minimum confidence threshold (default: SENTENCE_RETRY_MIN_CONF_THRESHOLD)
/// - padding_ms: Milliseconds to add as buffer around retry sections for context (default: 100)
///
/// Returns:
/// Vec of (start_ms, end_ms) tuples for sections needing retry, with padding applied.
pub fn get_retry_sections(
    segments: &[WordSegment],
    min_conf_threshold: f32,
    padding_ms: i64,
) -> Vec<(i64, i64)> {
    if segments.is_empty() {
        return Vec::new();
    }

    let sentences = cluster_into_sentences(segments);
    let mut retry_ranges: Vec<(i64, i64)> = Vec::new();

    for sentence in sentences {
        let (needs_retry, _, _) =
            sentence_needs_retry_with_threshold(&sentence, min_conf_threshold);

        if needs_retry {
            // Get first and last timestamps for the sentence (in centiseconds)
            let t0_cs = sentence[0].t0_cs;
            let t1_cs = sentence[sentence.len() - 1].t1_cs;

            // Find indices by matching timestamps and text (since sentences contain cloned segments)
            let first_word_idx = segments
                .iter()
                .position(|s| s.t0_cs == sentence[0].t0_cs && s.text == sentence[0].text)
                .unwrap_or(0);
            let last_word_idx = segments
                .iter()
                .rposition(|s| {
                    s.t1_cs == sentence[sentence.len() - 1].t1_cs
                        && s.text == sentence[sentence.len() - 1].text
                })
                .unwrap_or(segments.len() - 1);

            // Convert to milliseconds and add padding
            let mut start_ms = ((t0_cs * 10) - padding_ms).max(0);
            let mut end_ms = (t1_cs * 10) + padding_ms;

            // Cap end_ms to not exceed next word's start (if exists)
            if last_word_idx + 1 < segments.len() {
                let next_word = &segments[last_word_idx + 1];
                let max_end_ms = next_word.t0_cs * 10;
                end_ms = end_ms.min(max_end_ms);
            }

            // Cap start_ms to not go before previous word's end (if exists)
            if first_word_idx > 0 {
                let prev_word = &segments[first_word_idx - 1];
                let min_start_ms = prev_word.t1_cs * 10;
                start_ms = start_ms.max(min_start_ms);
            }

            retry_ranges.push((start_ms, end_ms));
        }
    }

    if retry_ranges.is_empty() {
        return Vec::new();
    }

    // Merge overlapping/adjacent ranges
    retry_ranges.sort_by_key(|x| x.0);
    let mut merged: Vec<(i64, i64)> = vec![retry_ranges[0]];

    for (start_ms, end_ms) in retry_ranges.into_iter().skip(1) {
        let last_idx = merged.len() - 1;
        let (last_start, last_end) = merged[last_idx];
        if start_ms <= last_end {
            // Overlapping or adjacent - merge
            merged[last_idx] = (last_start, last_end.max(end_ms));
        } else {
            // Separate range
            merged.push((start_ms, end_ms));
        }
    }

    merged
}

/// Helper function that allows specifying a custom threshold (used by get_retry_sections)
fn sentence_needs_retry_with_threshold(
    sentence: &[WordSegment],
    min_conf_threshold: f32,
) -> (bool, f32, ConfidenceBreakdown) {
    let (score, breakdown) = calculate_sentence_confidence(sentence);
    let needs_retry = breakdown.min_conf < min_conf_threshold;
    (needs_retry, score, breakdown)
}

/// Extract full text from high-confidence sentences for use as context prompt.
///
/// Returns concatenated text from sentences that pass the confidence threshold
/// AND would NOT be retried. This keeps low-confidence sentences out of the
/// prompt, avoiding anchoring the decoder to errors.
///
/// Args:
/// - segments: Word segments to analyze
/// - threshold: Sentence confidence threshold (default: 0.5)
///
/// Returns:
/// Concatenated text from high-confidence sentences, preserving word order
pub fn get_high_confidence_context(
    segments: &[WordSegment],
    threshold: f32,
) -> String {
    let sentences = cluster_into_sentences(segments);
    let mut high_conf_parts: Vec<String> = Vec::new();
    
    for sentence in sentences {
        let (score, _) = calculate_sentence_confidence(&sentence);
        let (needs_retry, _, _) = sentence_needs_retry(&sentence);
        
        if score >= threshold && !needs_retry {
            let sentence_text: String = sentence
                .iter()
                .map(|seg| seg.text.trim())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            
            if !sentence_text.is_empty() {
                high_conf_parts.push(sentence_text);
            }
        }
    }
    
    high_conf_parts.join(" ")
}

/// Replace segments in primary result with retry results for given time ranges.
///
/// This function splices retry segments into the primary transcription by:
/// 1. Iterating through primary segments
/// 2. Replacing segments that fall within retry ranges with retry segments
/// 3. Preserving segments outside retry ranges
///
/// Args:
/// - primary_segments: Original word segments from primary model
/// - retry_results: Vec of (start_ms, end_ms, segments) tuples for each retry range
///
/// Returns:
/// New Vec of WordSegments with retry segments spliced in
pub fn splice_segments(
    primary_segments: Vec<WordSegment>,
    retry_results: Vec<(i64, i64, Vec<WordSegment>)>,
) -> Vec<WordSegment> {
    if retry_results.is_empty() {
        return primary_segments;
    }

    let mut result: Vec<WordSegment> = Vec::new();
    let mut processed_ranges: std::collections::HashSet<(i64, i64)> =
        std::collections::HashSet::new();

    let mut i = 0;
    while i < primary_segments.len() {
        let seg = &primary_segments[i];
        let seg_start_ms = seg.t0_cs * 10;
        let seg_end_ms = seg.t1_cs * 10;

        // Check if this segment falls within any retry range
        let mut matching_range: Option<(i64, i64)> = None;
        for (retry_start_ms, retry_end_ms, _) in &retry_results {
            // Check if segment overlaps with retry range
            if seg_start_ms < *retry_end_ms && seg_end_ms > *retry_start_ms {
                matching_range = Some((*retry_start_ms, *retry_end_ms));
                break;
            }
        }

        if let Some(range) = matching_range {
            // This segment is in a retry range - replace with retry segments
            // Add retry segments for this range (only once per range)
            if !processed_ranges.contains(&range) {
                // Find the retry segments for this range
                if let Some((_, _, retry_segments)) = retry_results
                    .iter()
                    .find(|(s, e, _)| *s == range.0 && *e == range.1)
                {
                    result.extend(retry_segments.clone());
                }
                processed_ranges.insert(range);
            }

            // Skip all primary segments until we're past the retry range
            while i < primary_segments.len() {
                let next_seg = &primary_segments[i];
                let next_start_ms = next_seg.t0_cs * 10;
                let next_end_ms = next_seg.t1_cs * 10;

                // Check if next segment still overlaps with this retry range
                if next_start_ms < range.1 && next_end_ms > range.0 {
                    i += 1;
                } else {
                    break;
                }
            }
        } else {
            // Keep the primary segment
            result.push(seg.clone());
            i += 1;
        }
    }

    // Sort by timestamp to ensure correct order
    result.sort_by_key(|s| s.t0_cs);
    result
}

/// Hybrid transcriber that uses a fast primary model for initial transcription,
/// then selectively re-transcribes low-confidence sections with a more accurate retry model.
pub struct HybridTranscriber {
    primary: WhisperTranscriber,
    retry: WhisperTranscriber,
    config: WhisperConfig,
}

impl HybridTranscriber {
    /// Create a new hybrid transcriber with both primary and retry models
    pub fn new(config: &WhisperConfig) -> Result<Self, TranscribeError> {
        tracing::info!(
            "HybridTranscriber::new called with config: model={}, retry_model={:?}, backend={:?}",
            config.model,
            config.retry_model,
            config.backend
        );
        
        let retry_model = config.retry_model.as_ref().ok_or_else(|| {
            tracing::error!("retry_model is None but HybridTranscriber::new was called");
            TranscribeError::InitFailed(
                "retry_model must be set for hybrid transcription".to_string(),
            )
        })?;

        tracing::info!(
            "Loading hybrid transcriber: primary={}, retry={}",
            config.model,
            retry_model
        );

        // Create primary transcriber
        tracing::info!("Creating primary transcriber with model: {}", config.model);
        let primary = WhisperTranscriber::new(config)?;
        tracing::info!("Primary transcriber created successfully");

        // Create retry transcriber with retry model
        tracing::info!("Creating retry transcriber with model: {}", retry_model);
        let mut retry_config = config.clone();
        retry_config.model = retry_model.clone();
        let retry = WhisperTranscriber::new(&retry_config)?;
        tracing::info!("Retry transcriber created successfully");

        tracing::info!("Hybrid transcriber initialization complete");
        Ok(Self {
            primary,
            retry,
            config: config.clone(),
        })
    }
}

impl Transcriber for HybridTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        tracing::info!(
            "HybridTranscriber::transcribe called with {} samples ({:.2}s)",
            samples.len(),
            samples.len() as f32 / 16000.0
        );
        tracing::info!("Starting hybrid transcription pipeline");

        // Step 1: Run primary model on full audio
        let primary_start = std::time::Instant::now();
        let primary_details = self.primary.transcribe_with_confidence(samples, None)?;
        let primary_time = primary_start.elapsed();

        tracing::info!(
            "Primary transcription completed in {:.2}s: {} words",
            primary_time.as_secs_f32(),
            primary_details.segments.len()
        );

        // Step 2: Identify retry sections
        let retry_sections = get_retry_sections(
            &primary_details.segments,
            SENTENCE_RETRY_MIN_CONF_THRESHOLD,
            100, // padding_ms
        );

        if retry_sections.is_empty() {
            tracing::info!("No low-confidence sections detected. Using primary results as-is.");
            return Ok(primary_details.text);
        }

        tracing::info!("Found {} section(s) requiring retry", retry_sections.len());
        for (i, (start_ms, end_ms)) in retry_sections.iter().enumerate() {
            tracing::debug!("  Retry section {}: {}ms - {}ms", i + 1, start_ms, end_ms);
        }

        // Step 2.5: Extract high-confidence context for retry prompts
        let context_prompt = {
            let context = get_high_confidence_context(&primary_details.segments, 0.5);
            if !context.is_empty() {
                tracing::info!("Extracted high-confidence context: {} chars", context.len());
                tracing::debug!("Context: {}", context);
                Some(context)
            } else {
                None
            }
        };

        // Step 3: Re-transcribe retry sections with retry model
        let retry_start = std::time::Instant::now();
        let mut retry_results: Vec<(i64, i64, Vec<WordSegment>)> = Vec::new();

        for (start_ms, end_ms) in retry_sections {
            // Extract segment samples
            const SAMPLE_RATE: u32 = 16000;
            let start_idx = ((start_ms as f64 / 1000.0) * SAMPLE_RATE as f64) as usize;
            let end_idx = ((end_ms as f64 / 1000.0) * SAMPLE_RATE as f64) as usize;
            let start_idx = start_idx.min(samples.len());
            let end_idx = end_idx.min(samples.len()).max(start_idx);

            if start_idx >= samples.len() || end_idx <= start_idx {
                tracing::warn!(
                    "Invalid retry segment range: {}ms - {}ms, skipping",
                    start_ms,
                    end_ms
                );
                continue;
            }

            let segment_samples = &samples[start_idx..end_idx];

            // Preprocess retry segment (silence removal + speedup)
            let processed_samples = crate::audio::preprocess::preprocess_audio(
                segment_samples,
                SAMPLE_RATE,
                self.config.silence_removal_enabled,
                self.config.speedup_enabled,
            )
            .map_err(|e| TranscribeError::AudioFormat(format!("Preprocessing failed: {}", e)))?;

            // Transcribe preprocessed segment with retry model, using context prompt if available
            let retry_details = self
                .retry
                .transcribe_with_confidence(&processed_samples, context_prompt.as_deref())?;

            // Adjust timestamps to be relative to original audio timeline
            let offset_cs = (start_ms / 10) as i64;
            let adjusted_segments: Vec<WordSegment> = retry_details
                .segments
                .into_iter()
                .map(|mut seg| {
                    seg.t0_cs += offset_cs;
                    seg.t1_cs += offset_cs;
                    seg
                })
                .collect();

            tracing::debug!(
                "Retry section [{}ms - {}ms]: {} words",
                start_ms,
                end_ms,
                adjusted_segments.len()
            );

            retry_results.push((start_ms, end_ms, adjusted_segments));
        }

        let retry_time = retry_start.elapsed();
        tracing::info!(
            "Retry transcription completed in {:.2}s",
            retry_time.as_secs_f32()
        );

        // Step 4: Splice retry results into primary results
        let spliced_segments = splice_segments(primary_details.segments, retry_results);

        // Rebuild text from spliced segments
        let final_text = spliced_segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<String>()
            .trim()
            .to_string();

        tracing::info!(
            "Hybrid transcription completed: primary={:.2}s, retry={:.2}s, total={:.2}s",
            primary_time.as_secs_f32(),
            retry_time.as_secs_f32(),
            (primary_time + retry_time).as_secs_f32()
        );

        Ok(final_text)
    }
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
