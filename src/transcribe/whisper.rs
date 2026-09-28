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
use std::sync::Mutex;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Whisper-based transcriber
pub struct WhisperTranscriber {
    /// Whisper context (holds the model)
    ctx: WhisperContext,
    /// CPU fallback context, built lazily on the first GPU failure.
    ///
    /// Why this exists: the GPU context holds the model persistently, but every
    /// transcription re-creates a *state* (KV cache + compute buffers) and that
    /// allocation can fail when another process has taken the VRAM. Measured on a
    /// RTX 3070 Laptop (8 GiB) shared with a batch indexer:
    ///
    /// ```text
    /// ggml_vulkan: Device memory allocation of size 18874368 failed.
    /// ggml_vulkan: vk::Device::allocateMemory: ErrorOutOfDeviceMemory
    /// whisper_kv_cache_init: failed to allocate memory for the kv cache
    /// ERROR Transcription failed: Failed to create a new whisper context.
    /// ```
    ///
    /// The model itself was still resident on the card — only the 18 MB of
    /// per-transcription scratch could not be found. The user loses the sentence
    /// they just spoke, which no amount of retrying afterwards can give back.
    ///
    /// `None` inside the mutex means "not built yet"; a build failure is not cached,
    /// so a transient condition (page cache pressure while reading the model) does
    /// not poison the fallback for the rest of the process lifetime.
    ///
    /// Cost when nothing ever fails: zero. Nothing is loaded until the first failure.
    cpu_fallback: Mutex<Option<WhisperContext>>,
    /// Model path, kept to build the CPU fallback without re-resolving.
    model_path: PathBuf,
    /// Flash attention setting, mirrored onto the fallback context.
    flash_attention: bool,
    /// Whether the primary context was asked to use a GPU. When false there is
    /// nothing to fall back *from*, and the fallback is never attempted.
    gpu_requested: bool,
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
    /// Two-letter code for the language used during the most recent
    /// `transcribe()` call. Populated for single-language and constrained
    /// auto-detection modes; left empty for unconstrained auto-detection
    /// (since whisper-rs does not currently expose the chosen language
    /// from the full() pipeline). Read via [`Transcriber::last_detected_language`].
    last_language: Mutex<Option<String>>,
}

impl WhisperTranscriber {
    /// Create a new whisper transcriber
    pub fn new(config: &WhisperConfig) -> Result<Self, TranscribeError> {
        let model_path = resolve_model_path(&config.model)?;

        tracing::info!("Loading whisper model from {:?}", model_path);
        let start = std::time::Instant::now();

        let mut ctx_params = WhisperContextParameters::default();
        if let Some(device) = config.gpu_device {
            tracing::info!("Using GPU device index {}", device);
            ctx_params.gpu_device(device);
        }
        ctx_params.flash_attn(config.flash_attention);
        if config.flash_attention {
            tracing::info!("Flash attention enabled");
        }

        let ctx = WhisperContext::new_with_params(
            model_path
                .to_str()
                .ok_or_else(|| TranscribeError::ModelNotFound("Invalid path".to_string()))?,
            ctx_params,
        )
        .map_err(|e| TranscribeError::InitFailed(e.to_string()))?;

        tracing::info!("Model loaded in {:.2}s", start.elapsed().as_secs_f32());

        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));

        Ok(Self {
            ctx,
            cpu_fallback: Mutex::new(None),
            model_path,
            flash_attention: config.flash_attention,
            gpu_requested: config.gpu_device.is_some(),
            language: config.language.clone(),
            translate: config.translate,
            threads,
            context_window_optimization: config.context_window_optimization,
            initial_prompt: config.initial_prompt.clone(),
            last_language: Mutex::new(None),
        })
    }

    fn build_params<'a, 'b>(
        &'a self,
        selected_language: Option<&'a str>,
        duration_secs: f32,
        retry: bool,
    ) -> FullParams<'a, 'b> {
        let mut params = if retry {
            FullParams::new(SamplingStrategy::BeamSearch {
                beam_size: 5,
                patience: -1.0,
            })
        } else {
            FullParams::new(SamplingStrategy::Greedy { best_of: 1 })
        };

        match selected_language {
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

        // Prevent hallucination/looping by not conditioning on previous text.
        params.set_no_context(true);

        // Set initial prompt if configured
        if let Some(prompt) = &self.initial_prompt {
            params.set_initial_prompt(prompt);
            tracing::debug!("Using initial prompt: {:?}", prompt);
        }

        // Single-segment mode is faster for normal short dictation, but when
        // retrying a degenerate decode, let Whisper segment normally.
        if duration_secs < 30.0 && !retry {
            params.set_single_segment(true);
        }

        // Context-window reduction speeds up the normal path. Keep retries at
        // the model's default context so a too-small window cannot repeat the
        // same degenerate output.
        if self.context_window_optimization && !retry {
            if let Some(audio_ctx) = calculate_audio_ctx(duration_secs) {
                params.set_audio_ctx(audio_ctx);
                tracing::info!(
                    "Audio context optimization: using audio_ctx={} for {:.2}s clip",
                    audio_ctx,
                    duration_secs
                );
            }
        }

        params
    }

    fn run_full(
        &self,
        ctx: &WhisperContext,
        samples: &[f32],
        selected_language: Option<&str>,
        duration_secs: f32,
        retry: bool,
    ) -> Result<String, TranscribeError> {
        let mut state = ctx
            .create_state()
            .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;
        let params = self.build_params(selected_language, duration_secs, retry);

        state
            .full(params, samples)
            .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;

        let mut text = String::new();
        for segment in state.as_iter() {
            text.push_str(
                segment
                    .to_str()
                    .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?,
            );
        }

        Ok(text.trim().to_string())
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

impl WhisperTranscriber {
    /// Run one full transcription on the given context.
    ///
    /// Split out of `transcribe()` so that the *entire* run can be retried on the
    /// CPU fallback, not just the state creation. `state.full()` allocates its own
    /// compute buffers and can fail the same way `create_state()` does; wrapping
    /// only the latter would leave that path uncovered for the same amount of code.
    fn transcribe_on(
        &self,
        ctx: &WhisperContext,
        samples: &[f32],
        duration_secs: f32,
    ) -> Result<String, TranscribeError> {
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples)",
            duration_secs,
            samples.len()
        );

        let start = std::time::Instant::now();

        // Determine language based on configuration mode.
        //
        // The state is created INSIDE the branch that needs it. It used to be created
        // unconditionally here, and in single-language mode — the common case — it was
        // then dropped without a single call being made on it. That is a full KV-cache
        // allocation (18 MB of VRAM, measured) per transcription, for nothing, and it
        // doubled the number of chances to hit an out-of-memory condition on a busy card.
        let selected_language: Option<String> = if self.language.is_auto() {
            // Unconstrained auto-detection: let Whisper detect from all languages
            tracing::debug!("Using unconstrained language auto-detection");
            None
        } else if self.language.is_multiple() {
            // Constrained auto-detection: detect from allowed set only
            let allowed = self.language.as_vec();
            tracing::debug!("Using constrained language detection from: {:?}", allowed);
            let mut state = ctx
                .create_state()
                .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;
            Some(self.select_language_from_allowed(&mut state, samples, &allowed)?)
        } else {
            // Single language: use it directly
            let lang = self.language.primary().to_string();
            tracing::debug!("Using specified language: {}", lang);
            Some(lang)
        };

        // Record the language for output methods that benefit from a layout
        // hint (e.g. eitype --layout, dotool DOTOOL_XKB_LAYOUT). See
        // `Transcriber::last_detected_language`. Unconstrained auto-detect
        // leaves this as `None`; whisper-rs does not expose the language
        // chosen inside `full()` so we cannot recover it after the fact.
        if let Ok(mut guard) = self.last_language.lock() {
            *guard = selected_language.clone();
        }

        let mut result = self.run_full(
            ctx,
            samples,
            selected_language.as_deref(),
            duration_secs,
            false,
        )?;

        if duration_secs >= 1.0 && is_degenerate_transcript(&result) {
            tracing::warn!(
                "Whisper returned degenerate transcript {:?} for {:.2}s audio; retrying with beam search",
                result,
                duration_secs
            );
            let retry_result = self.run_full(
                ctx,
                samples,
                selected_language.as_deref(),
                duration_secs,
                true,
            )?;
            if is_degenerate_transcript(&retry_result) {
                tracing::warn!(
                    "Whisper retry also returned degenerate transcript {:?}; treating as empty",
                    retry_result
                );
                result.clear();
            } else {
                tracing::info!("Whisper retry recovered transcript");
                result = retry_result;
            }
        }

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

    /// Build the CPU fallback context on first use.
    ///
    /// Deliberately NOT cached on failure: if loading the model fails once because the
    /// machine was momentarily out of page cache, the next dictation should try again
    /// rather than inherit a permanent "no fallback" verdict.
    fn with_cpu_fallback<R>(
        &self,
        f: impl FnOnce(&WhisperContext) -> R,
    ) -> Result<R, TranscribeError> {
        let mut guard = self
            .cpu_fallback
            .lock()
            .map_err(|_| TranscribeError::InitFailed("CPU fallback mutex poisoned".to_string()))?;

        if guard.is_none() {
            tracing::warn!(
                "Building CPU fallback context from {:?} — the GPU could not serve this transcription",
                self.model_path
            );
            let start = std::time::Instant::now();

            let mut params = WhisperContextParameters::default();
            params.use_gpu(false);
            params.flash_attn(self.flash_attention);

            let ctx = WhisperContext::new_with_params(
                self.model_path
                    .to_str()
                    .ok_or_else(|| TranscribeError::ModelNotFound("Invalid path".to_string()))?,
                params,
            )
            .map_err(|e| TranscribeError::InitFailed(e.to_string()))?;

            tracing::info!(
                "CPU fallback context ready in {:.2}s",
                start.elapsed().as_secs_f32()
            );
            *guard = Some(ctx);
        }

        let ctx = guard
            .as_ref()
            .expect("CPU fallback context was just built or already present");
        Ok(f(ctx))
    }
}

impl Transcriber for WhisperTranscriber {
    /// Transcribe, and do not fail because the GPU is busy.
    ///
    /// The GPU holds the model persistently, but each transcription allocates a fresh
    /// state on the card. On a shared GPU that allocation can fail at any moment —
    /// measured here with a batch indexer holding 6.1 GB of an 8 GB card. When it does,
    /// the sentence the user just spoke is gone, and no later retry can recover it:
    /// the audio buffer is not kept, and the user has already stopped talking.
    ///
    /// So the CPU is the guaranteed floor and the GPU is an optimisation that is never
    /// allowed to carry the load. A CPU transcription is slower — seconds instead of
    /// tenths — but slow is a different category of outcome from lost.
    ///
    /// ANY inference error triggers the fallback, not just a recognised out-of-memory
    /// string. Matching on driver messages would mean a new ggml release, a different
    /// backend, or a reworded error silently turning "never fails" back into "fails".
    /// The cost of falling back when it was not strictly needed is one slow dictation;
    /// the cost of not falling back is a lost one.
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / 16000.0;

        let gpu_error = match self.transcribe_on(&self.ctx, samples, duration_secs) {
            Ok(text) => return Ok(text),
            Err(e) => e,
        };

        // No GPU was requested: the primary context IS the CPU one, and there is
        // nothing to fall back to. Report the real error rather than loading the
        // model a second time to fail identically.
        if !self.gpu_requested {
            return Err(gpu_error);
        }

        tracing::warn!(
            "GPU transcription failed ({}), falling back to CPU for this {:.2}s clip",
            gpu_error,
            duration_secs
        );

        match self.with_cpu_fallback(|ctx| self.transcribe_on(ctx, samples, duration_secs)) {
            Ok(Ok(text)) => {
                tracing::info!("CPU fallback transcribed the clip the GPU could not");
                Ok(text)
            }
            // The fallback ran and still failed: report ITS error, which describes what
            // actually stopped the transcription, not the GPU condition that led here.
            Ok(Err(cpu_error)) => {
                tracing::error!("CPU fallback also failed: {}", cpu_error);
                Err(cpu_error)
            }
            // The fallback could not even be built. The GPU error is the one worth
            // reporting — it is the cause; this is a consequence.
            Err(build_error) => {
                tracing::error!("Could not build the CPU fallback: {}", build_error);
                Err(gpu_error)
            }
        }
    }

    fn last_detected_language(&self) -> Option<String> {
        self.last_language.lock().ok().and_then(|g| g.clone())
    }
}

fn is_degenerate_transcript(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }

    !trimmed.chars().any(|c| c.is_alphanumeric())
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
/// Formula: max(duration_seconds * 50 + 128, 384), rounded up to multiple of 8
///
/// This optimization reduces transcription time for short recordings by
/// telling Whisper to use a smaller context window proportional to the
/// actual audio length, rather than the full 30-second batch window.
///
/// The conservative formula includes:
/// - Increased padding (128 instead of 64) for stability
/// - Minimum threshold of 384 (~7.7s context) to avoid instability with very short clips
/// - Alignment to multiple of 8 for GPU backend compatibility (Metal, Vulkan)
fn calculate_audio_ctx(duration_secs: f32) -> Option<i32> {
    const MIN_AUDIO_CTX: i32 = 384; // ~7.7s minimum context

    if duration_secs <= 22.5 {
        let raw_ctx = (duration_secs * 50.0) as i32 + 128;
        let bounded_ctx = raw_ctx.max(MIN_AUDIO_CTX);
        // Round up to next multiple of 8 for GPU backend alignment
        let aligned_ctx = (bounded_ctx + 7) / 8 * 8;
        Some(aligned_ctx)
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
        // Very short clips use minimum threshold (384), aligned to 8
        // 1s: max(50 + 128, 384) = 384, already aligned
        assert_eq!(calculate_audio_ctx(1.0), Some(384));

        // 5s: max(250 + 128, 384) = 384, already aligned
        assert_eq!(calculate_audio_ctx(5.0), Some(384));

        // 10s: max(500 + 128, 384) = 628, aligned to 632
        assert_eq!(calculate_audio_ctx(10.0), Some(632));

        // At threshold: max(1125 + 128, 384) = 1253, aligned to 1256
        assert_eq!(calculate_audio_ctx(22.5), Some(1256));
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
        // 1. When enabled: short clips get optimized audio_ctx (e.g., 384 min for short clips)
        // 2. When disabled: Whisper's default 1500 is used (not set explicitly)

        const WHISPER_DEFAULT_AUDIO_CTX: i32 = 1500;

        // With optimization enabled, 1s clip uses minimum threshold (384)
        let optimized_ctx = calculate_audio_ctx(1.0);
        assert_eq!(optimized_ctx, Some(384));
        assert!(optimized_ctx.unwrap() < WHISPER_DEFAULT_AUDIO_CTX);

        // With optimization disabled, we don't call calculate_audio_ctx,
        // so Whisper uses its default of 1500. This is handled in transcribe()
        // by checking self.context_window_optimization before applying.

        // Verify the optimization provides reduction (conservative formula still saves ~75%)
        let ratio = WHISPER_DEFAULT_AUDIO_CTX as f32 / optimized_ctx.unwrap() as f32;
        assert!(
            ratio > 3.0,
            "Optimization should reduce context by >3x for 1s clips"
        );
    }

    #[test]
    fn test_audio_ctx_alignment() {
        // Verify all results are aligned to multiple of 8 for GPU compatibility
        for duration in [1.0, 3.0, 5.0, 7.0, 10.0, 15.0, 20.0, 22.5] {
            if let Some(ctx) = calculate_audio_ctx(duration) {
                assert_eq!(
                    ctx % 8,
                    0,
                    "audio_ctx {} for {}s should be aligned to 8",
                    ctx,
                    duration
                );
            }
        }
    }

    #[test]
    fn test_degenerate_transcript_detection() {
        assert!(is_degenerate_transcript(""));
        assert!(is_degenerate_transcript("   "));
        assert!(is_degenerate_transcript("-"));
        assert!(is_degenerate_transcript(" - \n"));
        assert!(is_degenerate_transcript("..."));
        assert!(is_degenerate_transcript("—"));

        assert!(!is_degenerate_transcript("- Implement phase 1"));
        assert!(!is_degenerate_transcript("hello"));
        assert!(!is_degenerate_transcript("123"));
    }
}
