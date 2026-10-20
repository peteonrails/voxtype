//! Parakeet-based speech-to-text transcription
//!
//! Uses NVIDIA's Parakeet model via the parakeet-rs crate for fast, local transcription.
//! This module is only available when the `parakeet` feature is enabled.
//!
//! Supports two model architectures:
//! - CTC (Connectionist Temporal Classification): faster, character-level output
//! - TDT (Token-Duration-Transducer): recommended, proper punctuation and word boundaries

use super::lines::{lines_from_words, TimedWord};
use super::{TimedSegment, Transcriber};
use crate::config::{ParakeetConfig, ParakeetModelType};
use crate::error::TranscribeError;
#[cfg(any(
    feature = "parakeet-cuda",
    feature = "parakeet-migraphx",
    feature = "parakeet-tensorrt"
))]
use parakeet_rs::ExecutionProvider;
use parakeet_rs::{
    ExecutionConfig, Parakeet, ParakeetTDT, TimedToken, Transcriber as ParakeetTranscriberTrait,
    TranscriptionResult,
};
use std::path::PathBuf;
use std::sync::Mutex;

/// Parakeet models are fixed at 16kHz mono.
const SAMPLE_RATE: usize = 16_000;

/// Internal enum to hold either CTC or TDT model instance
enum ParakeetModel {
    /// CTC model (character-level, faster)
    Ctc(Mutex<Parakeet>),
    /// TDT model (token-level, better quality output)
    Tdt(Mutex<ParakeetTDT>),
}

/// Parakeet-based transcriber using ONNX Runtime
pub struct ParakeetTranscriber {
    /// Parakeet model instance (CTC or TDT)
    model: ParakeetModel,
    /// Model type for logging
    model_type: ParakeetModelType,
    /// Longest audio, in samples, sent through the model in one pass.
    /// Zero disables windowing.
    max_window: usize,
    /// Audio shared between consecutive windows, in samples.
    window_overlap: usize,
}

impl ParakeetTranscriber {
    /// Create a new Parakeet transcriber
    pub fn new(config: &ParakeetConfig) -> Result<Self, TranscribeError> {
        let model_path = resolve_model_path(&config.model)?;

        // Determine model type: use config override or auto-detect from directory
        let model_type = config
            .model_type
            .unwrap_or_else(|| detect_model_type(&model_path));

        tracing::info!(
            "Loading Parakeet {:?} model from {:?}",
            model_type,
            model_path
        );
        let start = std::time::Instant::now();

        // Configure execution provider based on feature flags
        let exec_config = build_execution_config(config.threads);

        let model = match model_type {
            ParakeetModelType::Ctc => {
                let parakeet =
                    Parakeet::from_pretrained(&model_path, exec_config).map_err(|e| {
                        TranscribeError::InitFailed(format!("Parakeet CTC init failed: {}", e))
                    })?;
                ParakeetModel::Ctc(Mutex::new(parakeet))
            }
            ParakeetModelType::Tdt => {
                let parakeet =
                    ParakeetTDT::from_pretrained(&model_path, exec_config).map_err(|e| {
                        TranscribeError::InitFailed(format!("Parakeet TDT init failed: {}", e))
                    })?;
                ParakeetModel::Tdt(Mutex::new(parakeet))
            }
        };

        tracing::info!(
            "Parakeet {:?} model loaded in {:.2}s",
            model_type,
            start.elapsed().as_secs_f32()
        );

        Ok(Self {
            model,
            model_type,
            max_window: (config.max_window_secs.max(0.0) * SAMPLE_RATE as f32) as usize,
            window_overlap: (config.window_overlap_secs.max(0.0) * SAMPLE_RATE as f32) as usize,
        })
    }
}

impl Transcriber for ParakeetTranscriber {
    fn short_clips_are_cheap(&self) -> bool {
        true
    }

    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        // The TDT exports carry a fixed-size attention bias. Past it ONNX
        // Runtime does not degrade, it aborts the run and the recording is
        // lost (#288). Cut long audio into windows the model can actually
        // take and stitch the transcripts back together.
        if self.max_window > 0 && samples.len() > self.max_window {
            let spans = crate::transcribe::window::windows(
                samples.len(),
                self.max_window,
                self.window_overlap,
            );
            tracing::info!(
                "Parakeet: {:.1}s exceeds the {:.0}s window; transcribing in {} \
                 overlapping passes",
                samples.len() as f32 / SAMPLE_RATE as f32,
                self.max_window as f32 / SAMPLE_RATE as f32,
                spans.len()
            );
            let mut merged = String::new();
            for (start, end) in spans {
                let text = self.transcribe_one(&samples[start..end])?;
                merged = crate::transcribe::window::merge_overlapping(&merged, &text);
            }
            return Ok(merged);
        }

        self.transcribe_one(samples)
    }

    /// A line per sentence, timed by Parakeet's own token times, for meeting
    /// mode's per-utterance clips. Audio too long for one pass is windowed
    /// by `transcribe` and comes back as one line over the clip.
    fn transcribe_timed(&self, samples: &[f32]) -> Result<Vec<TimedSegment>, TranscribeError> {
        if samples.is_empty() || (self.max_window > 0 && samples.len() > self.max_window) {
            let text = self.transcribe(samples)?;
            return Ok(if text.is_empty() {
                vec![]
            } else {
                vec![TimedSegment {
                    text,
                    start_secs: 0.0,
                    end_secs: samples.len() as f32 / SAMPLE_RATE as f32,
                }]
            });
        }
        let start = std::time::Instant::now();
        let result = self.infer(samples)?;
        let mut lines = lines_from_tokens(&result.tokens);
        let text = result.text.trim();
        if lines.is_empty() && !text.is_empty() {
            lines.push(TimedSegment {
                text: text.to_string(),
                start_secs: 0.0,
                end_secs: samples.len() as f32 / SAMPLE_RATE as f32,
            });
        }
        tracing::info!(
            "Parakeet {:?} timed transcription completed in {:.2}s: {} line(s)",
            self.model_type,
            start.elapsed().as_secs_f32(),
            lines.len()
        );
        Ok(lines)
    }
}

impl ParakeetTranscriber {
    /// One forward pass over audio already known to fit the model, with
    /// parakeet-rs's per-token times.
    fn infer(&self, samples: &[f32]) -> Result<TranscriptionResult, TranscribeError> {
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with Parakeet {:?}",
            samples.len() as f32 / SAMPLE_RATE as f32,
            samples.len(),
            self.model_type
        );
        let lock_failed = |e: String| {
            TranscribeError::InferenceFailed(format!("Failed to lock Parakeet mutex: {}", e))
        };
        // Default timestamp mode: tokens, whose text joins back into the
        // transcript exactly.
        match &self.model {
            ParakeetModel::Ctc(parakeet) => parakeet
                .lock()
                .map_err(|e| lock_failed(e.to_string()))?
                .transcribe_samples(samples.to_vec(), 16000, 1, None)
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!(
                        "Parakeet CTC inference failed: {}",
                        e
                    ))
                }),
            ParakeetModel::Tdt(parakeet) => parakeet
                .lock()
                .map_err(|e| lock_failed(e.to_string()))?
                .transcribe_samples(samples.to_vec(), 16000, 1, None)
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!(
                        "Parakeet TDT inference failed: {}",
                        e
                    ))
                }),
        }
    }

    /// One forward pass over audio already known to fit the model.
    fn transcribe_one(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        let start = std::time::Instant::now();
        let text = self.infer(samples)?.text.trim().to_string();
        tracing::info!(
            "Parakeet {:?} transcription completed in {:.2}s: {:?}",
            self.model_type,
            start.elapsed().as_secs_f32(),
            if text.chars().count() > 50 {
                format!("{}...", text.chars().take(50).collect::<String>())
            } else {
                text.clone()
            }
        );
        Ok(text)
    }
}

/// The longest a single token is taken to last. parakeet-rs ends a
/// transducer token where the next one starts, so the last word before a
/// pause would otherwise stretch across the whole pause and hide it.
const MAX_TOKEN_SECS: f32 = 0.5;

/// Words from Parakeet's timed tokens (a word starts at a token beginning
/// with a space or the sentencepiece marker), each ending at most
/// [`MAX_TOKEN_SECS`] after its last token began.
fn words_from_tokens(tokens: &[TimedToken]) -> Vec<TimedWord> {
    let mut words: Vec<TimedWord> = Vec::new();
    for token in tokens {
        let piece = token.text.replace('\u{2581}', " ");
        let end = token.end.min(token.start + MAX_TOKEN_SECS).max(token.start);
        match words.last_mut() {
            Some(word) if !piece.starts_with(' ') => {
                word.text.push_str(&piece);
                word.end_secs = end;
            }
            _ => words.push(TimedWord {
                text: piece.trim_start().to_string(),
                start_secs: token.start,
                end_secs: end,
            }),
        }
    }
    words
}

/// Meeting lines from Parakeet's timed tokens, by the shared word rules in
/// [`super::lines`].
pub(crate) fn lines_from_tokens(tokens: &[TimedToken]) -> Vec<TimedSegment> {
    lines_from_words(&words_from_tokens(tokens))
}

/// Build the ONNX Runtime session config: the execution provider the
/// features select, with `threads` intra-op threads when it is set. Unset
/// keeps parakeet-rs's default of 4.
pub(super) fn build_execution_config(threads: Option<usize>) -> Option<ExecutionConfig> {
    let config = provider_execution_config();
    match threads {
        Some(n) => Some(config.unwrap_or_default().with_intra_threads(n)),
        None => config,
    }
}

/// Execution provider config based on compile-time feature flags
fn provider_execution_config() -> Option<ExecutionConfig> {
    #[cfg(feature = "parakeet-cuda")]
    {
        if probe_cuda_runtime() {
            if !crate::setup::parakeet::cudnn_present_or_warn() {
                return None;
            }
            tracing::info!("Configuring CUDA execution provider for NVIDIA GPU acceleration");
            return Some(ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cuda));
        }
        tracing::warn!("CUDA not available or incompatible, falling back to CPU inference");
        return None;
    }

    #[cfg(feature = "parakeet-tensorrt")]
    {
        if probe_cuda_runtime() {
            if !crate::setup::parakeet::cudnn_present_or_warn() {
                return None;
            }
            tracing::info!("Configuring TensorRT execution provider for NVIDIA GPU acceleration");
            return Some(
                ExecutionConfig::new().with_execution_provider(ExecutionProvider::TensorRT),
            );
        }
        tracing::warn!("CUDA not available or incompatible, falling back to CPU inference");
        return None;
    }

    #[cfg(feature = "parakeet-migraphx")]
    {
        tracing::info!("Configuring MIGraphX execution provider for AMD GPU acceleration");
        return Some(ExecutionConfig::new().with_execution_provider(ExecutionProvider::MIGraphX));
    }

    #[cfg(not(any(
        feature = "parakeet-cuda",
        feature = "parakeet-tensorrt",
        feature = "parakeet-migraphx"
    )))]
    {
        None
    }
}

/// Probe CUDA runtime availability and version compatibility.
///
/// The bundled ONNX Runtime (from the `ort` crate) is built against CUDA 12.x.
/// If the system has a different major CUDA version, ONNX Runtime will segfault
/// during EP initialization rather than returning an error.
///
/// Returns true if CUDA looks compatible, false if it should be skipped.
#[cfg(any(feature = "parakeet-cuda", feature = "parakeet-tensorrt"))]
fn probe_cuda_runtime() -> bool {
    // Null-terminated library names to try, in order of preference
    let lib_names: &[&[u8]] = &[
        b"libcudart.so\0",
        b"libcudart.so.12\0",
        b"libcudart.so.13\0",
    ];

    let mut handle = std::ptr::null_mut();
    for name in lib_names {
        handle = unsafe { libc::dlopen(name.as_ptr() as *const libc::c_char, libc::RTLD_LAZY) };
        if !handle.is_null() {
            break;
        }
    }

    if handle.is_null() {
        tracing::error!(
            "CUDA runtime library (libcudart.so) not found. \
             Cannot initialize CUDA execution provider.\n  \
             Install the CUDA toolkit, or use a CPU backend instead."
        );
        return false;
    }

    let sym = unsafe {
        libc::dlsym(
            handle,
            b"cudaRuntimeGetVersion\0".as_ptr() as *const libc::c_char,
        )
    };

    if sym.is_null() {
        tracing::warn!("Could not find cudaRuntimeGetVersion in CUDA runtime library");
        unsafe { libc::dlclose(handle) };
        // Can't determine version, proceed and hope for the best
        return true;
    }

    // cudaRuntimeGetVersion signature: cudaError_t cudaRuntimeGetVersion(int *runtimeVersion)
    // Version is encoded as (major * 1000 + minor * 10)
    type CudaRuntimeGetVersion = unsafe extern "C" fn(*mut i32) -> i32;
    let get_version: CudaRuntimeGetVersion = unsafe { std::mem::transmute(sym) };

    let mut version: i32 = 0;
    let result = unsafe { get_version(&mut version) };
    unsafe { libc::dlclose(handle) };

    if result != 0 {
        tracing::warn!("cudaRuntimeGetVersion failed (error code {})", result);
        return true;
    }

    let major = version / 1000;
    let minor = (version % 1000) / 10;
    tracing::info!("Detected CUDA runtime version: {}.{}", major, minor);

    // ort 2.0.0-rc.12 picks the cu12 or cu13 prebuilt at compile time from
    // ORT_CUDA_VERSION (see ort-sys/build/download/resolve.rs). build.rs
    // mirrors that selection into VOXTYPE_BUILD_CUDA_MAJOR so this probe
    // accepts only the runtime version the bundled EP can actually talk to.
    // A mismatched major would crash ort's CUDA EP during initialization.
    //
    // Voxtype ships separate voxtype-onnx-cuda-12 and voxtype-onnx-cuda-13
    // binaries. `voxtype setup gpu --enable` symlinks voxtype-onnx-cuda to
    // whichever variant matches the host's CUDA runtime.
    //
    // Load-dynamic builds skip this check: there is no bundled ORT to
    // mismatch — the binary dlopens whatever libonnxruntime the system
    // provides, and ORT does its own kernel-image lookup against the host
    // CUDA at session-create time. v0.7.3 cuda-13 forgot to set
    // ORT_CUDA_VERSION=13 in its Dockerfile so VOXTYPE_BUILD_CUDA_MAJOR
    // baked in the default ("12"), and this probe falsely rejected
    // Blackwell hosts with "this binary's bundled ONNX Runtime requires
    // CUDA 12.x" (#386). The Dockerfile fix sets the env var properly,
    // and gating the check on `not(feature = "onnx-load-dynamic")`
    // closes the design hole so future load-dynamic builds don't depend
    // on remembering to set it.
    #[cfg(not(feature = "onnx-load-dynamic"))]
    {
        const EXPECTED_CUDA_MAJOR: i32 = match env!("VOXTYPE_BUILD_CUDA_MAJOR").as_bytes() {
            b"13" => 13,
            _ => 12,
        };

        if major != EXPECTED_CUDA_MAJOR {
            tracing::error!(
                "CUDA version mismatch: found CUDA {major}.{minor}, but this binary's \
                 bundled ONNX Runtime requires CUDA {EXPECTED_CUDA_MAJOR}.x. \
                 Continuing would crash the process.\n  \
                 Options:\n  \
                 1. Install the matching voxtype-onnx-cuda-{EXPECTED_CUDA_MAJOR} package\n  \
                 2. Switch to voxtype-onnx-cuda-{} for your CUDA version (`voxtype setup gpu --enable` \
                 auto-detects and points the symlink at the right one)\n  \
                 3. Build from source with --features parakeet-load-dynamic to link \
                 against your system's ONNX Runtime instead",
                major,
            );
            return false;
        }
    }

    true
}

/// Auto-detect model type from directory structure
///
/// TDT models have: encoder-model.onnx, decoder_joint-model.onnx, vocab.txt
/// CTC models have: model.onnx (or model_int8.onnx), tokenizer.json
fn detect_model_type(path: &PathBuf) -> ParakeetModelType {
    // Check for TDT model structure
    let has_encoder =
        path.join("encoder-model.onnx").exists() || path.join("encoder-model.onnx.data").exists();
    let has_decoder = path.join("decoder_joint-model.onnx").exists();

    if has_encoder && has_decoder {
        tracing::debug!("Auto-detected TDT model (found encoder + decoder ONNX files)");
        return ParakeetModelType::Tdt;
    }

    // Check for CTC model structure
    let has_ctc_model = path.join("model.onnx").exists() || path.join("model_int8.onnx").exists();
    let has_tokenizer = path.join("tokenizer.json").exists();

    if has_ctc_model && has_tokenizer {
        tracing::debug!("Auto-detected CTC model (found model.onnx + tokenizer.json)");
        return ParakeetModelType::Ctc;
    }

    // Default to TDT (recommended for most use cases)
    tracing::warn!(
        "Could not auto-detect model type from {:?}, defaulting to TDT. \
        Set model_type in config to override.",
        path
    );
    ParakeetModelType::Tdt
}

/// Resolve model name to directory path
pub(super) fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    // If it's already an absolute path, use it directly
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    // Check models directory
    let models_dir = crate::config::Config::models_dir();
    let model_path = models_dir.join(model);

    if model_path.exists() {
        return Ok(model_path);
    }

    // Check current directory
    let cwd_path = PathBuf::from(model);
    if cwd_path.exists() {
        return Ok(cwd_path);
    }

    // Check ./models/
    let local_models_path = PathBuf::from("models").join(model);
    if local_models_path.exists() {
        return Ok(local_models_path);
    }

    Err(TranscribeError::ModelNotFound(format!(
        "Parakeet model '{}' not found. Looked in:\n  - {}\n  - {}\n  - {}\n\n\
        Run: {}",
        model,
        model_path.display(),
        cwd_path.display(),
        local_models_path.display(),
        crate::model_catalog::download_command("parakeet", model),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Tokens as parakeet-rs's TDT decoder gives them: a word starts with a
    /// space, and a token ends where the next one starts.
    fn tokens(spec: &[(&str, f32)]) -> Vec<TimedToken> {
        spec.iter()
            .enumerate()
            .map(|(i, &(text, start))| TimedToken {
                text: text.to_string(),
                start,
                end: spec.get(i + 1).map_or(start + 0.08, |n| n.1),
            })
            .collect()
    }

    fn spans(lines: &[TimedSegment]) -> Vec<(&str, f32, f32)> {
        lines
            .iter()
            .map(|l| (l.text.as_str(), l.start_secs, l.end_secs))
            .collect()
    }

    #[test]
    fn a_line_per_sentence_timed_by_its_tokens() {
        let t = tokens(&[
            (" Hel", 0.32),
            ("lo", 0.48),
            (".", 0.64),
            (" See", 3.20),
            (" you", 3.44),
            (" at", 3.60),
            (" 3", 3.76),
            (".", 3.84),
            ("5", 3.92),
            ("?", 4.00),
        ]);
        let lines = lines_from_tokens(&t);
        assert_eq!(
            spans(&lines),
            vec![("Hello.", 0.32, 1.14), ("See you at 3.5?", 3.2, 4.08)],
            "the pause after the first sentence is not counted in it, and 3.5 is not cut"
        );
        let joined: String = t.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(
            lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            joined.trim()
        );
    }

    #[test]
    fn a_pause_without_punctuation_starts_a_new_line() {
        let t = tokens(&[
            (" yes", 0.1),
            (" right", 0.4),
            (" so", 1.6),
            (" anyway", 1.8),
        ]);
        assert_eq!(
            spans(&lines_from_tokens(&t)),
            vec![("yes right", 0.1, 0.9), ("so anyway", 1.6, 1.88)]
        );
        // Sentencepiece markers, as a raw vocabulary spells them.
        let t = tokens(&[("\u{2581}ok", 0.1), ("\u{2581}then", 0.3)]);
        assert_eq!(spans(&lines_from_tokens(&t)), vec![("ok then", 0.1, 0.38)]);
        // Nothing but punctuation is not a line.
        assert!(lines_from_tokens(&tokens(&[(".", 0.0)])).is_empty());
        assert!(lines_from_tokens(&[]).is_empty());
    }

    #[test]
    fn threads_set_the_onnx_intra_op_thread_count() {
        let config = build_execution_config(Some(12)).expect("threads set");
        assert_eq!(config.intra_threads, 12);
    }

    #[cfg(not(any(
        feature = "parakeet-cuda",
        feature = "parakeet-tensorrt",
        feature = "parakeet-migraphx"
    )))]
    #[test]
    fn unset_threads_keep_the_parakeet_rs_default_on_the_cpu() {
        assert!(build_execution_config(None).is_none());
    }
    use tempfile::TempDir;

    #[test]
    fn test_detect_model_type_tdt_with_encoder_and_decoder() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Create TDT model structure
        fs::write(model_path.join("encoder-model.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("decoder_joint-model.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("vocab.txt"), b"dummy").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Tdt);
    }

    #[test]
    fn test_detect_model_type_tdt_with_encoder_data_file() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // TDT model with .onnx.data file (large models split data)
        fs::write(model_path.join("encoder-model.onnx.data"), b"dummy").unwrap();
        fs::write(model_path.join("decoder_joint-model.onnx"), b"dummy").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Tdt);
    }

    #[test]
    fn test_detect_model_type_ctc_with_model_and_tokenizer() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Create CTC model structure
        fs::write(model_path.join("model.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("tokenizer.json"), b"{}").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Ctc);
    }

    #[test]
    fn test_detect_model_type_ctc_with_int8_model() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // CTC model with quantized int8 variant
        fs::write(model_path.join("model_int8.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("tokenizer.json"), b"{}").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Ctc);
    }

    #[test]
    fn test_detect_model_type_defaults_to_tdt_when_ambiguous() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Empty directory - should default to TDT
        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Tdt);
    }

    #[test]
    fn test_detect_model_type_defaults_to_tdt_with_partial_files() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Only encoder without decoder - ambiguous, defaults to TDT
        fs::write(model_path.join("encoder-model.onnx"), b"dummy").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Tdt);
    }

    #[test]
    fn test_detect_model_type_ctc_requires_both_files() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Only model.onnx without tokenizer - should not detect as CTC
        fs::write(model_path.join("model.onnx"), b"dummy").unwrap();

        let detected = detect_model_type(&model_path);
        // Falls through to default (TDT) because CTC requires both files
        assert_eq!(detected, ParakeetModelType::Tdt);
    }

    #[test]
    fn test_resolve_model_path_absolute() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Create a dummy file so the path exists
        fs::write(model_path.join("model.onnx"), b"dummy").unwrap();

        let resolved = resolve_model_path(model_path.to_str().unwrap());
        assert!(resolved.is_ok());
        assert_eq!(resolved.unwrap(), model_path);
    }

    #[test]
    fn test_resolve_model_path_not_found() {
        let result = resolve_model_path("/nonexistent/path/to/model");
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(matches!(err, TranscribeError::ModelNotFound(_)));
    }
}
