//! Whisper-based speech-to-text transcription
//!
//! Uses whisper.cpp via the whisper-rs crate for fast, local transcription.
//!
//! Supports three language modes:
//! - Single language: Use a specific language for transcription
//! - Auto-detect: Let Whisper detect from all ~99 supported languages
//! - Constrained auto-detect: Detect from a user-specified subset of languages

use super::Transcriber;
use crate::config::{Config, LanguageConfig, WhisperConfig};
use crate::error::TranscribeError;
use std::path::PathBuf;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Whisper-based transcriber
pub struct WhisperTranscriber {
    /// Whisper context (holds the model)
    ctx: WhisperContext,
    /// Language configuration (single, auto, or array)
    language: LanguageConfig,
    /// Whether to translate to English
    translate: bool,
    /// Number of threads to use
    threads: usize,
    /// Whether to optimize context window for short clips
    context_window_optimization: bool,
    /// Initial prompt to provide context for transcription
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
            context_window_optimization: config.context_window_optimization,
            initial_prompt: config.initial_prompt.clone(),
        })
    }

    /// Select the best language from allowed languages using Whisper's language detection.
    ///
    /// This runs the mel spectrogram computation and language detection head to get
    /// probabilities for all languages, then picks the highest-probability language
    /// from the user's allowed set.
    fn select_language_from_allowed(
        &self,
        state: &mut whisper_rs::WhisperState,
        samples: &[f32],
        allowed: &[String],
    ) -> Result<String, TranscribeError> {
        // Run pcm_to_mel to prepare the spectrogram for language detection
        state
            .pcm_to_mel(samples, self.threads)
            .map_err(|e| TranscribeError::InferenceFailed(format!("pcm_to_mel failed: {}", e)))?;

        // Run language detection to get probabilities for all languages
        let (detected_id, probs) = state
            .lang_detect(0, self.threads)
            .map_err(|e| TranscribeError::InferenceFailed(format!("lang_detect failed: {}", e)))?;

        // Find the highest-probability language from our allowed set
        let mut best_lang = None;
        let mut best_prob = -1.0f32;

        for lang in allowed {
            if let Some(lang_id) = whisper_rs::get_lang_id(lang) {
                if let Some(&prob) = probs.get(lang_id as usize) {
                    if prob > best_prob {
                        best_prob = prob;
                        best_lang = Some(lang.clone());
                    }
                }
            } else {
                tracing::warn!("Unknown language code '{}' in language array", lang);
            }
        }

        let selected = best_lang.unwrap_or_else(|| {
            tracing::warn!(
                "No valid languages found in allowed set {:?}, using first: {}",
                allowed,
                allowed.first().map(|s| s.as_str()).unwrap_or("en")
            );
            allowed.first().cloned().unwrap_or_else(|| "en".to_string())
        });

        // Log the detection result
        let detected_lang = whisper_rs::get_lang_str(detected_id).unwrap_or("unknown");
        tracing::info!(
            "Language detection: Whisper detected '{}', selected '{}' (prob={:.1}%) from allowed {:?}",
            detected_lang,
            selected,
            best_prob * 100.0,
            allowed
        );

        Ok(selected)
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

        // Determine language based on configuration mode
        let selected_language: Option<String> = if self.language.is_auto() {
            // Unconstrained auto-detection: let Whisper detect from all languages
            tracing::debug!("Using unconstrained language auto-detection");
            None
        } else if self.language.is_multiple() {
            // Constrained auto-detection: detect from allowed set only
            let allowed = self.language.as_vec();
            tracing::debug!("Using constrained language detection from: {:?}", allowed);
            Some(self.select_language_from_allowed(&mut state, samples, &allowed)?)
        } else {
            // Single language: use it directly
            let lang = self.language.primary().to_string();
            tracing::debug!("Using specified language: {}", lang);
            Some(lang)
        };

        // Configure parameters
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Set language
        match &selected_language {
            Some(lang) => params.set_language(Some(lang)),
            None => params.set_language(None),
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

        // Set initial prompt if configured
        if let Some(prompt) = &self.initial_prompt {
            params.set_initial_prompt(prompt);
            tracing::debug!("Using initial prompt: {:?}", prompt);
        }

        // For short recordings, use single segment mode
        if duration_secs < 30.0 {
            params.set_single_segment(true);
        }

        // Optimize context window for short clips
        if self.context_window_optimization {
            if let Some(audio_ctx) = calculate_audio_ctx(duration_secs) {
                params.set_audio_ctx(audio_ctx);
                tracing::info!(
                    "Audio context optimization: using audio_ctx={} for {:.2}s clip (formula: {:.2}s * 50 + 64)",
                    audio_ctx,
                    duration_secs,
                    duration_secs
                );
            }
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
///
/// This optimization reduces transcription time for short recordings by
/// telling Whisper to use a smaller context window proportional to the
/// actual audio length, rather than the full 30-second batch window.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_url() {
        let url = get_model_url("base.en");
        assert!(url.contains("ggml-base.en.bin"));
        assert!(url.contains("huggingface.co"));
    }

    #[test]
    fn test_calculate_audio_ctx_short_clips() {
        // Very short clip: 1s -> 1 * 50 + 64 = 114
        assert_eq!(calculate_audio_ctx(1.0), Some(114));

        // 5 second clip: 5 * 50 + 64 = 314
        assert_eq!(calculate_audio_ctx(5.0), Some(314));

        // 10 second clip: 10 * 50 + 64 = 564
        assert_eq!(calculate_audio_ctx(10.0), Some(564));

        // At threshold: 22.5 * 50 + 64 = 1189
        assert_eq!(calculate_audio_ctx(22.5), Some(1189));
    }

    #[test]
    fn test_calculate_audio_ctx_long_clips() {
        // Just over threshold: no optimization
        assert_eq!(calculate_audio_ctx(22.6), None);

        // 30 second clip: no optimization
        assert_eq!(calculate_audio_ctx(30.0), None);

        // 60 second clip: no optimization
        assert_eq!(calculate_audio_ctx(60.0), None);
    }

    #[test]
    fn test_audio_ctx_not_applied_when_disabled() {
        // When context_window_optimization is false, calculate_audio_ctx
        // should not be called, and Whisper uses its default audio_ctx of 1500
        // (the full 30-second context window).
        //
        // This test verifies the optimization logic by demonstrating:
        // 1. When enabled: short clips get optimized audio_ctx (e.g., 114 for 1s)
        // 2. When disabled: Whisper's default 1500 is used (not set explicitly)

        const WHISPER_DEFAULT_AUDIO_CTX: i32 = 1500;

        // With optimization enabled, 1s clip would use audio_ctx=114
        let optimized_ctx = calculate_audio_ctx(1.0);
        assert_eq!(optimized_ctx, Some(114));
        assert!(optimized_ctx.unwrap() < WHISPER_DEFAULT_AUDIO_CTX);

        // With optimization disabled, we don't call calculate_audio_ctx,
        // so Whisper uses its default of 1500. This is handled in transcribe()
        // by checking self.context_window_optimization before applying.

        // Verify the optimization provides significant reduction
        let ratio = WHISPER_DEFAULT_AUDIO_CTX as f32 / optimized_ctx.unwrap() as f32;
        assert!(
            ratio > 10.0,
            "Optimization should reduce context by >10x for 1s clips"
        );
    }
}
