//! Whisper-based speech-to-text transcription
//!
//! Uses whisper.cpp via the whisper-rs crate for fast, local transcription.
//!
//! Supports three language modes:
//! - Single language: Use a specific language for transcription
//! - Auto-detect: Let Whisper detect from all ~99 supported languages
//! - Constrained auto-detect: Detect from a user-specified subset of languages

use super::{RuntimeMetadata, TimedSegment, Transcriber};
use crate::config::{Config, LanguageConfig, WhisperConfig};
use crate::error::TranscribeError;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

/// Language for the next transcription, set by the hotkey listener from
/// `hotkey.language_modifiers`. Takes precedence over the configured language.
pub static LANGUAGE_OVERRIDE: Mutex<Option<String>> = Mutex::new(None);

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
    /// Profile vocabulary selected for the current recording.
    active_vocabulary: Mutex<Vec<String>>,
    /// Two-letter code for the language used during the most recent
    /// `transcribe()` call. Populated for single-language and constrained
    /// auto-detection modes; left empty for unconstrained auto-detection
    /// (since whisper-rs does not currently expose the chosen language
    /// from the full() pipeline). Read via [`Transcriber::last_detected_language`].
    last_language: Mutex<Option<String>>,
    runtime: RuntimeMetadata,
    cpu_context: bool,
    decoded: AtomicBool,
    /// Kept between transcriptions, since creating one allocates the KV
    /// caches and compute buffers again.
    state: Mutex<Option<WhisperState>>,
}

impl WhisperTranscriber {
    /// Create a new whisper transcriber
    pub fn new(config: &WhisperConfig) -> Result<Self, TranscribeError> {
        let model_path = resolve_model_path(&config.model)?;

        tracing::info!("Loading whisper model from {:?}", model_path);
        let start = std::time::Instant::now();

        let mut ctx_params = WhisperContextParameters::default();
        // An explicit gpu_device always wins. Otherwise, if the user asked for
        // a vendor with VOXTYPE_VULKAN_DEVICE, resolve that to an index and
        // set it: the loader hint alone has been observed not to take effect,
        // leaving a hybrid machine on its iGPU while reporting otherwise
        // (#577).
        let device_index = config.gpu_device.or_else(|| {
            let vendor = crate::setup::gpu::get_selected_gpu_vendor()?;
            match crate::setup::gpu::resolve_vulkan_device_index(vendor) {
                Some(idx) => {
                    tracing::info!(
                        "VOXTYPE_VULKAN_DEVICE={:?} resolved to Vulkan device index {}",
                        vendor,
                        idx
                    );
                    Some(idx)
                }
                None => {
                    tracing::warn!(
                        "VOXTYPE_VULKAN_DEVICE={:?} could not be matched to a Vulkan \
                         device (is vulkan-tools installed?); leaving device selection \
                         to the loader. Set [whisper] gpu_device to pick one directly.",
                        vendor
                    );
                    None
                }
            }
        });
        if let Some(device) = device_index {
            tracing::info!("Using GPU device index {}", device);
            ctx_params.gpu_device(device);
        }
        ctx_params.flash_attn(config.flash_attention);
        if config.flash_attention {
            tracing::info!("Flash attention enabled");
        }

        let cpu_context = !ctx_params.use_gpu;
        let ctx = WhisperContext::new_with_params(
            model_path
                .to_str()
                .ok_or_else(|| TranscribeError::ModelNotFound("Invalid path".to_string()))?,
            ctx_params,
        )
        .map_err(|e| TranscribeError::InitFailed(e.to_string()))?;

        tracing::info!("Model loaded in {:.2}s", start.elapsed().as_secs_f32());
        let runtime = RuntimeMetadata {
            model: Some(model_path.display().to_string()),
            backend: None,
            model_load_ms: Some(start.elapsed().as_millis() as u64),
        };

        if let Some(backend) = GPU_BACKEND {
            report_gpu_outcome(backend, device_index.unwrap_or(0), &ggml_gpu_devices());
        }

        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));

        Ok(Self {
            ctx,
            language: config.language.clone(),
            translate: config.translate,
            threads,
            context_window_optimization: config.context_window_optimization,
            initial_prompt: config.initial_prompt.clone(),
            active_vocabulary: Mutex::new(Vec::new()),
            last_language: Mutex::new(None),
            runtime,
            cpu_context,
            decoded: AtomicBool::new(false),
            state: Mutex::new(None),
        })
    }

    fn build_params<'a, 'b>(
        &self,
        selected_language: Option<&'a str>,
        duration_secs: f32,
        retry: bool,
        timed: bool,
        prompt: Option<&'a str>,
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
        if let Some(prompt) = prompt {
            params.set_initial_prompt(prompt);
            tracing::debug!("Using initial prompt: {:?}", prompt);
        }

        // Single-segment mode is faster for normal short dictation, but when
        // retrying a degenerate decode, let Whisper segment normally. A timed
        // decode wants Whisper's own segments, so it never merges them.
        if duration_secs < 30.0 && !retry && !timed {
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
        samples: &[f32],
        selected_language: Option<&str>,
        duration_secs: f32,
        retry: bool,
    ) -> Result<String, TranscribeError> {
        let segments =
            self.run_full_segments(samples, selected_language, duration_secs, retry, false)?;
        let text: String = segments.iter().map(|s| s.text.as_str()).collect();
        Ok(text.trim().to_string())
    }

    /// One decode, returning Whisper's segments with their timestamps.
    /// `timed` keeps Whisper's own segmentation instead of asking for one
    /// merged segment.
    fn run_full_segments(
        &self,
        samples: &[f32],
        selected_language: Option<&str>,
        duration_secs: f32,
        retry: bool,
        timed: bool,
    ) -> Result<Vec<RawSegment>, TranscribeError> {
        let vocabulary = self
            .active_vocabulary
            .lock()
            .map(|terms| terms.clone())
            .unwrap_or_default();
        let prompt = super::prompt_with_vocabulary(self.initial_prompt.as_deref(), &vocabulary);
        let params = self.build_params(
            selected_language,
            duration_secs,
            retry,
            timed,
            prompt.as_deref(),
        );

        self.with_state(|state| {
            state
                .full(params, samples)
                .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;
            self.decoded.store(true, Ordering::Relaxed);

            let mut segments = Vec::new();
            for segment in state.as_iter() {
                segments.push(RawSegment {
                    start_cs: segment.start_timestamp(),
                    end_cs: segment.end_timestamp(),
                    text: segment
                        .to_str()
                        .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?
                        .to_string(),
                });
            }
            Ok(segments)
        })
    }

    fn with_state<R>(
        &self,
        f: impl FnOnce(&mut WhisperState) -> Result<R, TranscribeError>,
    ) -> Result<R, TranscribeError> {
        with_cached(
            &self.state,
            || {
                self.ctx
                    .create_state()
                    .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))
            },
            f,
        )
    }

    /// The language to decode in, per the configured mode, recorded for
    /// [`Transcriber::last_detected_language`].
    fn select_language(&self, samples: &[f32]) -> Result<Option<String>, TranscribeError> {
        let override_language = LANGUAGE_OVERRIDE.lock().ok().and_then(|g| g.clone());
        let selected_language: Option<String> = if let Some(lang) = override_language {
            tracing::info!("Using language from hotkey modifier: {}", lang);
            Some(lang)
        } else if self.language.is_auto() {
            // Unconstrained auto-detection: let Whisper detect from all languages
            tracing::debug!("Using unconstrained language auto-detection");
            None
        } else if self.language.is_multiple() {
            // Constrained auto-detection: detect from allowed set only
            let allowed = self.language.as_vec();
            tracing::debug!("Using constrained language detection from: {:?}", allowed);
            let lang = self
                .with_state(|state| self.select_language_from_allowed(state, samples, &allowed))?;
            Some(lang)
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
        Ok(selected_language)
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
    /// Creates the Whisper state while the user speaks. A state in use
    /// already exists, so a busy lock is left alone.
    fn prepare(&self) {
        let Ok(mut state) = self.state.try_lock() else {
            return;
        };
        if state.is_none() {
            match self.ctx.create_state() {
                Ok(created) => *state = Some(created),
                Err(e) => tracing::warn!("Failed to create the Whisper state: {}", e),
            }
        }
    }

    fn set_vocabulary(&self, terms: &[String]) {
        if let Ok(mut active) = self.active_vocabulary.lock() {
            active.clear();
            active.extend_from_slice(terms);
        }
    }

    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        self.decoded.store(false, Ordering::Relaxed);
        // A failure below must not leave the previous call's language
        // standing as this call's.
        if let Ok(mut guard) = self.last_language.lock() {
            *guard = None;
        }
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

        let selected_language = self.select_language(samples)?;

        let mut result =
            self.run_full(samples, selected_language.as_deref(), duration_secs, false)?;

        if self.context_window_optimization && calculate_audio_ctx(duration_secs).is_some() {
            if let Some(trimmed) = drop_restarted_copy(&result) {
                tracing::warn!(
                    "Whisper repeated the transcript for {:.2}s audio; keeping the first copy",
                    duration_secs
                );
                result = trimmed;
            }
        }

        if duration_secs >= 1.0 && has_repetition_loop(&result) {
            tracing::warn!(
                "Whisper looped on a phrase for {:.2}s audio; retrying with beam search",
                duration_secs
            );
            let retry_result =
                self.run_full(samples, selected_language.as_deref(), duration_secs, true)?;
            if !is_degenerate_transcript(&retry_result) {
                result = retry_result;
            }
            result = collapse_repetition_loops(&result);
        } else if duration_secs >= 1.0 && is_degenerate_transcript(&result) {
            tracing::warn!(
                "Whisper returned degenerate transcript {:?} for {:.2}s audio; retrying with beam search",
                result,
                duration_secs
            );
            let retry_result =
                self.run_full(samples, selected_language.as_deref(), duration_secs, true)?;
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

        super::log_transcription("Transcription", start, &result);

        Ok(result)
    }

    /// Whisper times its own segments, so one decode of a whole meeting
    /// chunk yields one line per utterance (roughly a sentence or a
    /// pause-delimited phrase) with real start and end times. Meeting mode
    /// relies on this to keep a single call per chunk for Whisper.
    fn transcribe_timed(&self, samples: &[f32]) -> Result<Vec<TimedSegment>, TranscribeError> {
        self.decoded.store(false, Ordering::Relaxed);
        // A failure below must not leave the previous call's language
        // standing as this call's.
        if let Ok(mut guard) = self.last_language.lock() {
            *guard = None;
        }
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }
        let duration_secs = samples.len() as f32 / 16000.0;
        let start = std::time::Instant::now();
        let language = self.select_language(samples)?;

        let raw =
            self.run_full_segments(samples, language.as_deref(), duration_secs, false, true)?;
        let mut segments = timed_segments(&raw, duration_secs);
        if segments.is_empty() && duration_secs >= 1.0 && !raw.is_empty() {
            tracing::warn!(
                "Whisper returned only degenerate segments for {:.2}s audio; retrying with beam search",
                duration_secs
            );
            let raw =
                self.run_full_segments(samples, language.as_deref(), duration_secs, true, true)?;
            segments = timed_segments(&raw, duration_secs);
        }

        tracing::info!(
            "Timed transcription of {:.2}s completed in {:.2}s: {} segment(s)",
            duration_secs,
            start.elapsed().as_secs_f32(),
            segments.len()
        );
        Ok(segments)
    }

    fn last_detected_language(&self) -> Option<String> {
        self.last_language.lock().ok().and_then(|g| g.clone())
    }

    fn runtime_metadata(&self) -> RuntimeMetadata {
        let mut runtime = self.runtime.clone();
        // A completed decode with GPU disabled proves CPU execution. With GPU
        // enabled the context may fall back, so do not guess from build flags
        // or the requested device. Loading alone is not a completed decode.
        if self.cpu_context && self.decoded.load(Ordering::Relaxed) {
            runtime.backend = Some("cpu".into());
        }
        runtime
    }
}

/// One segment as whisper.cpp reports it: times in centiseconds.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RawSegment {
    pub(super) start_cs: i64,
    pub(super) end_cs: i64,
    pub(super) text: String,
}

/// Whisper's segments as [`TimedSegment`]s: text trimmed, times clamped to
/// the audio, and segments with no words (degenerate decodes) dropped.
pub(super) fn timed_segments(raw: &[RawSegment], duration_secs: f32) -> Vec<TimedSegment> {
    raw.iter()
        .filter_map(|r| {
            let text = r.text.trim();
            if is_degenerate_transcript(text) {
                return None;
            }
            let start = (r.start_cs.max(0) as f32 / 100.0).min(duration_secs);
            let end = (r.end_cs.max(0) as f32 / 100.0).clamp(start, duration_secs);
            Some(TimedSegment {
                text: text.to_string(),
                start_secs: start,
                end_secs: end,
            })
        })
        .collect()
}

/// Runs `f` on the value in `slot`, creating it first if the slot is empty.
/// A caller that finds the slot in use, as parallel eager chunks do, gets a
/// fresh value instead of waiting. The value is dropped if `f` fails, since a
/// failed run can leave it half-updated.
fn with_cached<T, R>(
    slot: &Mutex<Option<T>>,
    create: impl FnOnce() -> Result<T, TranscribeError>,
    f: impl FnOnce(&mut T) -> Result<R, TranscribeError>,
) -> Result<R, TranscribeError> {
    let Ok(mut cached) = slot.try_lock() else {
        return f(&mut create()?);
    };
    if cached.is_none() {
        *cached = Some(create()?);
    }
    let result = f(cached.as_mut().unwrap());
    if result.is_err() {
        *cached = None;
    }
    result
}

/// Shortest phrase, in words, whose back-to-back repeats count as a decoder loop.
/// Shorter repeats ("no no no") are often what was said.
const LOOP_MIN_WORDS: usize = 3;

fn loop_key(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Finds a phrase of at least LOOP_MIN_WORDS words repeated three or more
/// times in a row. Returns (start, phrase length, copies) in words.
fn find_repetition_loop(words: &[&str]) -> Option<(usize, usize, usize)> {
    let keys: Vec<String> = words.iter().map(|w| loop_key(w)).collect();
    for len in LOOP_MIN_WORDS..=keys.len() / 3 {
        for start in 0..=keys.len() - 3 * len {
            let mut copies = 1;
            while start + (copies + 1) * len <= keys.len()
                && keys[start..start + len]
                    == keys[start + copies * len..start + (copies + 1) * len]
            {
                copies += 1;
            }
            if copies >= 3 {
                return Some((start, len, copies));
            }
        }
    }
    None
}

/// How many opening words a copy at the end must repeat to count as a restart
const RESTART_MIN_WORDS: usize = 6;

/// With a reduced audio context, Whisper sometimes reaches the end of the
/// speech and transcribes the clip again from the start. If the text ends with
/// a copy of its own opening, returns the text before that copy.
fn drop_restarted_copy(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 2 * RESTART_MIN_WORDS {
        return None;
    }
    let keys: Vec<String> = words.iter().map(|w| loop_key(w)).collect();
    (RESTART_MIN_WORDS..=keys.len() - RESTART_MIN_WORDS)
        .find(|&start| keys[start..] == keys[..keys.len() - start])
        .map(|start| words[..start].join(" "))
}

fn has_repetition_loop(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    find_repetition_loop(&words).is_some()
}

/// Keeps one copy of each phrase Whisper looped on
fn collapse_repetition_loops(text: &str) -> String {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    while let Some((start, len, copies)) = find_repetition_loop(&words) {
        words.drain(start + len..start + copies * len);
    }
    words.join(" ")
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
    resolve_model_path_in(&Config::models_dir(), model)
}

pub(crate) fn model_filename(model: &str) -> Option<&str> {
    Some(match model {
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
        other if other.ends_with(".bin") => other,
        _ => return None,
    })
}

/// Resolve local overrides with the same aliases and search paths as inference.
/// Recording admission uses this without loading a model or opening audio.
pub(crate) fn resolve_model_path_in(
    models_dir: &std::path::Path,
    model: &str,
) -> Result<PathBuf, TranscribeError> {
    // If it's already an absolute path, use it directly
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    let model_filename = model_filename(model).ok_or_else(|| {
        TranscribeError::ModelNotFound(format!(
            "Unknown model: '{}'. Valid models: tiny, base, small, medium, large-v3, large-v3-turbo",
            model
        ))
    })?;

    // Look in the data directory
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
        "Model '{}' not found. Looked in:\n  - {}\n  - {}\n  - {}\n\nDownload with: voxtype setup --download --model {}",
        model,
        model_path.display(),
        cwd_path.display(),
        local_models_path.display(),
        model
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

/// The GPU backend compiled into whisper.cpp, if any. CPU-only builds never
/// ask whisper.cpp for a GPU, so there is nothing to fall back from.
const GPU_BACKEND: Option<&str> = if cfg!(feature = "gpu-vulkan") {
    Some("vulkan")
} else if cfg!(feature = "gpu-cuda") {
    Some("cuda")
} else if cfg!(feature = "gpu-hipblas") {
    Some("hip")
} else if cfg!(feature = "gpu-metal") {
    Some("metal")
} else {
    None
};

/// Names of the GPU devices ggml registered, in the order whisper.cpp counts
/// them for `gpu_device`. This is the same enumeration
/// `whisper_backend_init_gpu` walks, so an empty list (or one shorter than
/// the requested index) is exactly the case where whisper.cpp logs
/// `no GPU found` and quietly runs on CPU.
#[cfg(any(
    feature = "gpu-vulkan",
    feature = "gpu-cuda",
    feature = "gpu-hipblas",
    feature = "gpu-metal"
))]
fn ggml_gpu_devices() -> Vec<String> {
    use whisper_rs::whisper_rs_sys as sys;
    let mut names = Vec::new();
    // SAFETY: the ggml backend registry is a process-wide static that
    // whisper.cpp has already initialized by loading the model; these calls
    // only read it. Device names are NUL-terminated strings owned by ggml.
    unsafe {
        for i in 0..sys::ggml_backend_dev_count() {
            let dev = sys::ggml_backend_dev_get(i);
            let kind = sys::ggml_backend_dev_type(dev);
            if kind == sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU
                || kind == sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU
            {
                let desc = sys::ggml_backend_dev_description(dev);
                let name = if desc.is_null() {
                    sys::ggml_backend_dev_name(dev)
                } else {
                    desc
                };
                names.push(
                    std::ffi::CStr::from_ptr(name)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    names
}

#[cfg(not(any(
    feature = "gpu-vulkan",
    feature = "gpu-cuda",
    feature = "gpu-hipblas",
    feature = "gpu-metal"
)))]
fn ggml_gpu_devices() -> Vec<String> {
    Vec::new()
}

/// What a model load on a GPU build actually got.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GpuOutcome {
    /// Running on this device.
    Gpu(String),
    /// ggml registered no GPU at all.
    NoGpu,
    /// GPUs exist, but `[whisper] gpu_device` points past the end of the list.
    BadIndex { requested: i32, found: usize },
}

pub(crate) fn gpu_outcome(devices: &[String], requested: i32) -> GpuOutcome {
    match usize::try_from(requested).ok().and_then(|i| devices.get(i)) {
        Some(name) => GpuOutcome::Gpu(name.clone()),
        None if devices.is_empty() => GpuOutcome::NoGpu,
        None => GpuOutcome::BadIndex {
            requested,
            found: devices.len(),
        },
    }
}

/// The notification for a CPU fallback: what happened and the exact fix.
pub(crate) fn fallback_message(backend: &str, outcome: &GpuOutcome) -> Option<String> {
    let fix = match backend {
        "vulkan" => {
            "Install your GPU's Vulkan driver (Arch: vulkan-radeon, vulkan-intel or \
             nvidia-utils; Debian/Ubuntu/Fedora: mesa-vulkan-drivers, or the NVIDIA \
             driver), check it with `vulkaninfo --summary`, then restart voxtype"
        }
        "cuda" => {
            "Install the NVIDIA driver and CUDA runtime (Arch: nvidia-utils cuda), \
             check it with `nvidia-smi`, then restart voxtype"
        }
        "hip" => {
            "Install ROCm (Arch: rocm-hip-runtime), check it with `rocminfo`, \
             then restart voxtype"
        }
        _ => "Check that this machine has a supported GPU, then restart voxtype",
    };
    match outcome {
        GpuOutcome::Gpu(_) => None,
        GpuOutcome::NoGpu => Some(format!(
            "This GPU build of Whisper found no {} GPU and is transcribing on CPU. {}. \
             Details: voxtype info accel",
            backend_label(backend),
            fix
        )),
        GpuOutcome::BadIndex { requested, found } => Some(format!(
            "[whisper] gpu_device = {} but only {} GPU(s) were found (numbered from 0), \
             so Whisper is transcribing on CPU. Fix gpu_device in config.toml, \
             then restart voxtype. Details: voxtype info accel",
            requested, found
        )),
    }
}

fn backend_label(backend: &str) -> &'static str {
    match backend {
        "vulkan" => "Vulkan",
        "cuda" => "CUDA",
        "hip" => "ROCm",
        "metal" => "Metal",
        _ => "usable",
    }
}

/// Publish the outcome for `voxtype info accel`, and tell the user once per
/// daemon start when a GPU build is running on CPU. Transcription carries on
/// either way: CPU is slow, not broken.
fn report_gpu_outcome(backend: &str, requested: i32, devices: &[String]) {
    use crate::setup::accel::{self, AccelState};
    let outcome = gpu_outcome(devices, requested);
    match fallback_message(backend, &outcome) {
        None => {
            if let GpuOutcome::Gpu(name) = outcome {
                tracing::info!("Whisper is running on GPU: {}", name);
                accel::publish(AccelState::Gpu, Some(backend), Some(name));
            }
        }
        Some(message) => {
            tracing::warn!("{}", message);
            let detail = match outcome {
                GpuOutcome::BadIndex { requested, found } => {
                    format!("gpu_device = {} but {} GPU(s) found", requested, found)
                }
                _ => format!("ggml found no {} GPU", backend_label(backend)),
            };
            if accel::publish(AccelState::CpuFallback, Some(backend), Some(detail)) {
                crate::notification::send_sync("Voxtype: Whisper is running on CPU", &message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(start_cs: i64, end_cs: i64, text: &str) -> RawSegment {
        RawSegment {
            start_cs,
            end_cs,
            text: text.to_string(),
        }
    }

    /// What keeping the state saves, measured on a real model. Ignored by
    /// default because it needs one:
    ///
    /// VOXTYPE_BENCH_MODEL=/path/ggml-base.en.bin VOXTYPE_BENCH_WAV=/path/clip.wav \
    ///   cargo test --release --lib whisper_state_reuse_timing -- --ignored --nocapture
    ///
    /// Each "fresh" run drops the cached state first, which is what every
    /// transcription paid before the state was kept.
    #[test]
    #[ignore]
    fn whisper_state_reuse_timing() {
        let model = std::env::var("VOXTYPE_BENCH_MODEL").expect("VOXTYPE_BENCH_MODEL");
        let wav = std::env::var("VOXTYPE_BENCH_WAV").expect("VOXTYPE_BENCH_WAV");
        let runs: usize = std::env::var("VOXTYPE_BENCH_RUNS")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(5);
        let reader = hound::WavReader::open(&wav).unwrap();
        let spec = reader.spec();
        assert_eq!(
            (spec.sample_rate, spec.channels),
            (16_000, 1),
            "16 kHz mono WAV"
        );
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => {
                let max = (1_i64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .into_samples::<i32>()
                    .map(|s| s.unwrap() as f32 / max)
                    .collect()
            }
            hound::SampleFormat::Float => {
                reader.into_samples::<f32>().map(|s| s.unwrap()).collect()
            }
        };
        let config = WhisperConfig {
            model,
            ..Default::default()
        };
        let t = WhisperTranscriber::new(&config).unwrap();
        let median = |mut v: Vec<f64>| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 2]
        };
        let time = |f: &dyn Fn()| {
            let start = std::time::Instant::now();
            f();
            start.elapsed().as_secs_f64() * 1000.0
        };
        // Warm up once so file caches and the first allocation are paid.
        t.transcribe(&samples).unwrap();
        let mut fresh = Vec::new();
        let mut reused = Vec::new();
        let mut create = Vec::new();
        for _ in 0..runs {
            *t.state.lock().unwrap() = None;
            fresh.push(time(&|| {
                t.transcribe(&samples).unwrap();
            }));
            reused.push(time(&|| {
                t.transcribe(&samples).unwrap();
            }));
            *t.state.lock().unwrap() = None;
            create.push(time(&|| t.prepare()));
        }
        eprintln!(
            "whisper state timing ({} runs, {:.2}s clip): fresh state median {:.1} ms, \
             reused state median {:.1} ms, state creation median {:.1} ms",
            runs,
            samples.len() as f32 / 16_000.0,
            median(fresh),
            median(reused),
            median(create),
        );
    }

    #[test]
    fn with_cached_reuses_the_value() {
        let slot = Mutex::new(None);
        let mut created = 0;
        for _ in 0..3 {
            with_cached(
                &slot,
                || {
                    created += 1;
                    Ok(0)
                },
                |n: &mut i32| {
                    *n += 1;
                    Ok(())
                },
            )
            .unwrap();
        }
        assert_eq!(created, 1);
        assert_eq!(*slot.lock().unwrap(), Some(3));
    }

    #[test]
    fn with_cached_drops_the_value_after_an_error() {
        let slot = Mutex::new(Some(1));
        let result: Result<(), _> = with_cached(
            &slot,
            || Ok(0),
            |_| Err(TranscribeError::InferenceFailed("failed".into())),
        );
        assert!(result.is_err());
        assert!(slot.lock().unwrap().is_none());
    }

    #[test]
    fn with_cached_uses_a_fresh_value_while_the_slot_is_busy() {
        let slot = Mutex::new(Some(1));
        let _busy = slot.lock().unwrap();
        let value = with_cached(&slot, || Ok(7), |n: &mut i32| Ok(*n)).unwrap();
        assert_eq!(value, 7);
    }

    #[test]
    fn whisper_segments_become_timed_segments() {
        let got = timed_segments(
            &[
                raw(0, 250, " We ship on Friday."),
                raw(250, 400, " ..."),
                raw(420, 980, " Agreed, Friday it is."),
                // Whisper can run a timestamp past the end of the audio.
                raw(2950, 3100, " Bye."),
            ],
            30.0,
        );
        let got: Vec<(String, f32, f32)> = got
            .into_iter()
            .map(|s| (s.text, s.start_secs, s.end_secs))
            .collect();
        assert_eq!(
            got,
            vec![
                ("We ship on Friday.".to_string(), 0.0, 2.5),
                ("Agreed, Friday it is.".to_string(), 4.2, 9.8),
                ("Bye.".to_string(), 29.5, 30.0),
            ]
        );
    }

    #[test]
    fn segment_times_never_run_backwards_or_negative() {
        let got = timed_segments(&[raw(-5, 100, "a"), raw(500, 300, "b")], 10.0);
        assert_eq!((got[0].start_secs, got[0].end_secs), (0.0, 1.0));
        assert_eq!((got[1].start_secs, got[1].end_secs), (5.0, 5.0));
    }

    #[test]
    fn test_model_filename() {
        assert_eq!(get_model_filename("base.en"), "ggml-base.en.bin");
        assert_eq!(
            get_model_filename("/models/custom.bin"),
            "/models/custom.bin"
        );
    }

    #[test]
    fn gpu_outcome_matches_whisper_cpp_device_selection() {
        let gpus = vec![
            "NVIDIA GeForce RTX 3090".to_string(),
            "AMD iGPU".to_string(),
        ];
        assert_eq!(
            gpu_outcome(&gpus, 0),
            GpuOutcome::Gpu("NVIDIA GeForce RTX 3090".to_string())
        );
        assert_eq!(
            gpu_outcome(&gpus, 1),
            GpuOutcome::Gpu("AMD iGPU".to_string())
        );
        assert_eq!(
            gpu_outcome(&gpus, 2),
            GpuOutcome::BadIndex {
                requested: 2,
                found: 2
            }
        );
        assert_eq!(gpu_outcome(&[], 0), GpuOutcome::NoGpu);
    }

    #[test]
    fn fallback_message_names_the_fix() {
        assert!(fallback_message("vulkan", &GpuOutcome::Gpu("x".into())).is_none());

        let no_gpu = fallback_message("vulkan", &GpuOutcome::NoGpu).unwrap();
        assert!(no_gpu.contains("no Vulkan GPU"));
        assert!(no_gpu.contains("vulkaninfo --summary"));
        assert!(no_gpu.contains("voxtype info accel"));

        let bad = fallback_message(
            "vulkan",
            &GpuOutcome::BadIndex {
                requested: 3,
                found: 1,
            },
        )
        .unwrap();
        assert!(bad.contains("gpu_device = 3"));
        assert!(bad.contains("only 1 GPU(s)"));
    }

    #[test]
    fn test_drop_restarted_copy() {
        // Transcripts Whisper returned with the reduced audio context
        assert_eq!(
            drop_restarted_copy(
                "It has required my password to login. It has required my password to login."
            )
            .as_deref(),
            Some("It has required my password to login.")
        );
        assert_eq!(
            drop_restarted_copy(
                "Can you look into fixing that bug then in the original 654 PR driver? \
                 We can make another PR that fixes that to their branch. \
                 Can you look into fixing that bug then in the original 654 PR driver?"
            )
            .as_deref(),
            Some(
                "Can you look into fixing that bug then in the original 654 PR driver? \
                 We can make another PR that fixes that to their branch."
            )
        );
        // An opening said twice with different words after it is kept
        assert_eq!(
            drop_restarted_copy(
                "It has required my password to login. It has required my password to unlock it."
            ),
            None
        );
        assert_eq!(drop_restarted_copy("It has required my password."), None);
        assert_eq!(drop_restarted_copy(""), None);
    }

    #[test]
    fn test_repetition_loops() {
        let looped = "I mean, what's the status of the project? ".repeat(15);
        assert!(has_repetition_loop(&looped));
        assert_eq!(
            collapse_repetition_loops(&looped),
            "I mean, what's the status of the project?"
        );
        assert_eq!(
            collapse_repetition_loops("So, go on. Go on. Go on. Go on. Then stop."),
            "So, go on. Go on. Go on. Go on. Then stop."
        );
        assert_eq!(
            collapse_repetition_loops("Fine. It works now. It works now. It works now. Done."),
            "Fine. It works now. Done."
        );
        assert!(!has_repetition_loop("no no no, not that one"));
        assert!(!has_repetition_loop("one two three one two three"));
        assert!(!has_repetition_loop(""));
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
