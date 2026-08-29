//! OpenVINO GenAI Whisper transcription (Intel NPU / GPU / CPU).
//!
//! Uses OpenVINO GenAI's `WhisperPipeline` via the `openvino-genai` crate.
//! The model is an OpenVINO IR directory (`openvino_encoder_model.xml`,
//! `openvino_decoder_model.xml`, `tokenizer.json`, ...), e.g. the
//! `OpenVINO/whisper-*-ov` repos on HuggingFace.
//!
//! Selected when `engine = "openvino"` and built with `--features openvino`.
//! The pipeline object is `!Sync`, so it is wrapped in a `Mutex` (same pattern
//! as the ONNX engines wrapping `ort::Session`).

use super::Transcriber;
use crate::config::OpenVinoConfig;
use crate::error::TranscribeError;
use openvino_genai::WhisperPipeline;
use std::path::PathBuf;
use std::sync::Mutex;

/// Directory OpenVINO persists compiled device blobs to (`CACHE_DIR`
/// property), so NPU/GPU graph compilation isn't repeated on every
/// pipeline init. Same `$XDG_CACHE_HOME/voxtype/<name>` convention as the
/// MIGraphX model cache in `setup/binary.rs`.
fn openvino_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("voxtype")
        .join("openvino")
}

/// OpenVINO GenAI Whisper transcriber.
pub struct OpenVinoTranscriber {
    /// The GenAI pipeline. `!Sync`, hence the mutex.
    pipeline: Mutex<WhisperPipeline>,
    /// Device actually in use ("NPU", "GPU", or "CPU").
    #[allow(dead_code)]
    device: String,
    /// Language passed to the generation config.
    language: String,
    /// True for English-only checkpoints (tiny.en/base.en/small.en/...).
    /// Their vocabulary has no task/language special tokens at all, so
    /// calling `set_language`/`set_task` on them looks up a token that
    /// doesn't exist. CPU/GPU tolerate this (silently ignored); the NPU
    /// backend throws (surfaces as a bare "unknown exception" through the
    /// C API — this is what nova-npu's OpenVINOTranscriber works around
    /// by skipping both calls for `.en` models; ported the same rule).
    english_only: bool,
}

impl OpenVinoTranscriber {
    pub fn new(config: &OpenVinoConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;
        let model_str = model_dir
            .to_str()
            .ok_or_else(|| TranscribeError::InitFailed("Invalid model path".to_string()))?;

        tracing::info!(
            "Loading OpenVINO Whisper model from {:?} on {}",
            model_dir,
            config.device
        );
        let start = std::time::Instant::now();

        // CACHE_DIR persists the device's compiled model blob to disk so
        // subsequent pipeline inits skip recompilation. This matters most
        // on NPU (compiling the graph to NPU-ISA) and GPU (OpenCL kernel
        // compilation) — both are expensive on cold start and otherwise
        // pay that cost on every daemon restart. CPU accepts the property
        // too but has little to gain from it (its compile step is cheap).
        let cache_dir = openvino_cache_dir();
        let cache_dir_str = cache_dir.to_string_lossy();
        let props: &[(&str, &str)] = &[("CACHE_DIR", &cache_dir_str)];

        let pipeline = match WhisperPipeline::with_properties(model_str, &config.device, props) {
            Ok(p) => p,
            Err(e) if config.device != "CPU" => {
                tracing::warn!(
                    "OpenVINO device '{}' failed to initialize ({}); falling back to CPU",
                    config.device,
                    e
                );
                WhisperPipeline::with_properties(model_str, "CPU", props).map_err(|e2| {
                    TranscribeError::InitFailed(format!(
                        "OpenVINO pipeline init failed on CPU: {}",
                        e2
                    ))
                })?
            }
            Err(e) => {
                return Err(TranscribeError::InitFailed(format!(
                    "OpenVINO pipeline init failed on {}: {}",
                    config.device, e
                )))
            }
        };

        tracing::info!(
            "OpenVINO model loaded in {:.2}s",
            start.elapsed().as_secs_f32()
        );

        Ok(Self {
            pipeline: Mutex::new(pipeline),
            device: config.device.clone(),
            language: config.language.clone(),
            english_only: config.model.ends_with(".en"),
        })
    }
}

impl Transcriber for OpenVinoTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        let mut pipeline = self.pipeline.lock().map_err(|_| {
            TranscribeError::InferenceFailed("OpenVINO pipeline lock poisoned".to_string())
        })?;

        // Start from the pipeline's OWN generation config (derived from the
        // loaded model's generation_config.json: EOS/decoder-start token
        // ids, is_multilingual, etc.), not a bare `::new()` default — a
        // disconnected default config is missing that model-specific state
        // entirely, which is what the official C sample's
        // `ov_genai_whisper_pipeline_get_generation_config` pattern avoids.
        let mut gen_cfg = pipeline.get_generation_config().map_err(|e| {
            TranscribeError::InferenceFailed(format!("OpenVINO generation config: {}", e))
        })?;
        // English-only checkpoints have no task/language tokens in their
        // vocabulary at all — see the `english_only` field doc.
        if !self.english_only {
            gen_cfg.set_language(&self.language).map_err(|e| {
                TranscribeError::InferenceFailed(format!("OpenVINO set_language: {}", e))
            })?;
            gen_cfg.set_task("transcribe").map_err(|e| {
                TranscribeError::InferenceFailed(format!("OpenVINO set_task: {}", e))
            })?;
        }

        let results = pipeline.generate(samples, Some(&gen_cfg)).map_err(|e| {
            TranscribeError::InferenceFailed(format!("OpenVINO generate failed: {}", e))
        })?;

        results.get_string().map_err(|e| {
            TranscribeError::InferenceFailed(format!("OpenVINO get_string failed: {}", e))
        })
    }

    fn last_detected_language(&self) -> Option<String> {
        Some(self.language.clone())
    }
}

/// Resolve the model argument to an OpenVINO IR directory.
///
/// Accepts a path to an existing IR directory, or a short name
/// (`tiny.en`, `base.en`, `small.en`, `base`, `small`) resolved under
/// `<models_dir>/openvino/<name>`.
fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    let path = PathBuf::from(model);
    if path.join("openvino_encoder_model.xml").exists() {
        return Ok(path);
    }

    let models_dir = crate::config::Config::models_dir();
    let model_path = models_dir.join("openvino").join(model);
    if model_path.join("openvino_encoder_model.xml").exists() {
        return Ok(model_path);
    }

    Err(TranscribeError::ModelNotFound(format!(
        "OpenVINO model '{}' not found. Looked in:\n  \
         - {}\n  \
         - {}\n\n\
         Download an OpenVINO IR whisper model, e.g.:\n  \
         huggingface-cli download OpenVINO/whisper-base.en-int8-ov --local-dir {}\n\n\
         or run 'voxtype setup model' to pick one.",
        model,
        path.display(),
        model_path.display(),
        model_path.display()
    )))
}
