//! OpenVINO GenAI Whisper speech-to-text transcription
//!
//! Uses the OpenVINO GenAI WhisperPipeline to run Whisper models on Intel NPU, CPU,
//! or GPU. The pipeline handles mel spectrogram extraction, encoder-decoder inference,
//! and tokenization internally.
//!
//! Models are in OpenVINO IR format from HuggingFace (OpenVINO/whisper-* repos),
//! exported via `optimum-cli export openvino`.

use super::Transcriber;
use crate::config::OpenVinoConfig;
use crate::error::TranscribeError;
use openvino_genai::WhisperPipeline;
use std::path::PathBuf;
use std::sync::Mutex;

/// OpenVINO GenAI Whisper transcriber for Intel NPU/CPU/GPU.
///
/// Pipeline creation is deferred to `prepare()` (called when recording starts),
/// hiding the load latency behind recording time. The pipeline is cached and
/// reused across transcriptions. If `prepare()` was not called, creation happens
/// on first `transcribe()` call.
pub struct OpenVinoTranscriber {
    pipeline: Mutex<Option<WhisperPipeline>>,
    model_dir: PathBuf,
    config: OpenVinoConfig,
}

impl OpenVinoTranscriber {
    /// Create a new OpenVINO GenAI Whisper transcriber.
    ///
    /// Resolves the model directory and optionally creates the pipeline immediately
    /// (when `on_demand_loading` is false). The expensive pipeline creation can be
    /// deferred to `prepare()` or first `transcribe()`.
    pub fn new(config: &OpenVinoConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model, config.quantized)?;

        tracing::info!(
            "Initializing OpenVINO GenAI Whisper from {:?} (device={}, quantized={})",
            model_dir,
            config.device,
            config.quantized
        );

        // Sanity check that the model directory has expected files
        let encoder_xml = model_dir.join("openvino_encoder_model.xml");
        if !encoder_xml.exists() {
            return Err(TranscribeError::ModelNotFound(format!(
                "OpenVINO Whisper encoder model not found: {}\n  \
                 Run 'voxtype setup model' to download, or manually from:\n  \
                 https://huggingface.co/OpenVINO/whisper-{}",
                encoder_xml.display(),
                config.model
            )));
        }

        if config.threads.is_some() {
            tracing::warn!(
                "OpenVINO GenAI WhisperPipeline does not support thread count configuration; \
                 the 'threads' setting will be ignored"
            );
        }

        let pipeline = if config.on_demand_loading {
            None
        } else {
            Some(Self::create_pipeline(&model_dir, config)?)
        };

        tracing::info!("OpenVINO GenAI Whisper initialized");

        Ok(Self {
            pipeline: Mutex::new(pipeline),
            model_dir,
            config: config.clone(),
        })
    }

    /// Load the OpenVINO GenAI shared library, using a custom path if configured.
    fn load_library(config: &OpenVinoConfig) -> Result<(), TranscribeError> {
        if let Some(ref dir) = config.openvino_dir {
            let lib_path = find_genai_library(dir)?;
            // Preload OpenVINO dependency libraries with RTLD_GLOBAL so that dlopen
            // can resolve the DT_NEEDED entries in libopenvino_genai_c.so. The OpenVINO
            // shared libraries don't set RPATH/RUNPATH, and glibc caches LD_LIBRARY_PATH
            // at startup so setting it at runtime has no effect.
            if let Some(lib_dir) = lib_path.parent() {
                preload_openvino_deps(lib_dir);
            }
            tracing::info!("Loading OpenVINO GenAI library from: {}", lib_path.display());
            openvino_genai::load_from(&lib_path).map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load OpenVINO GenAI library from {}: {}\n  \
                     Ensure libopenvino_genai_c.so exists in the specified openvino_dir.",
                    lib_path.display(),
                    e
                ))
            })
        } else {
            openvino_genai::load().map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load OpenVINO GenAI library: {}\n  \
                     Install OpenVINO GenAI: pip install openvino-genai\n  \
                     Or set openvino_dir in [openvino] config to the library directory.",
                    e
                ))
            })
        }
    }

    /// Create the WhisperPipeline for the configured device.
    fn create_pipeline(
        model_dir: &std::path::Path,
        config: &OpenVinoConfig,
    ) -> Result<WhisperPipeline, TranscribeError> {
        let start = std::time::Instant::now();

        Self::load_library(config)?;

        let model_path_str = model_dir.to_str().ok_or_else(|| {
            TranscribeError::InitFailed("Model path contains invalid UTF-8".to_string())
        })?;

        let is_npu = config.device.to_uppercase() == "NPU";

        let pipeline = WhisperPipeline::new(model_path_str, &config.device).map_err(|e| {
            if is_npu {
                TranscribeError::InitFailed(format!(
                    "Failed to create OpenVINO GenAI Whisper pipeline for NPU: {}\n  \
                     Ensure intel-npu-driver is installed.\n  \
                     Check: ls /dev/accel/accel*\n  \
                     Or set device = \"CPU\" in [openvino] config.",
                    e
                ))
            } else {
                TranscribeError::InitFailed(format!(
                    "Failed to create OpenVINO GenAI Whisper pipeline for {}: {}",
                    config.device, e
                ))
            }
        })?;

        tracing::info!(
            "OpenVINO GenAI Whisper pipeline created in {:.2}s (device={})",
            start.elapsed().as_secs_f32(),
            config.device,
        );

        Ok(pipeline)
    }

    /// Ensure the pipeline is created, creating on first use if needed.
    fn ensure_pipeline(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<WhisperPipeline>>, TranscribeError> {
        let mut guard = self.pipeline.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Pipeline lock poisoned: {}", e))
        })?;

        if guard.is_none() {
            tracing::info!(
                "Pipeline not yet created, creating now (prepare() was not called)"
            );
            *guard = Some(Self::create_pipeline(&self.model_dir, &self.config)?);
        }

        Ok(guard)
    }
}

impl Transcriber for OpenVinoTranscriber {
    fn prepare(&self) {
        let mut guard = match self.pipeline.lock() {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("Pipeline lock error in prepare(): {}", e);
                return;
            }
        };

        if guard.is_some() {
            tracing::debug!("Pipeline already created, skipping prepare()");
            return;
        }

        tracing::info!(
            "Creating OpenVINO GenAI Whisper pipeline for {} (triggered by prepare())...",
            self.config.device
        );
        match Self::create_pipeline(&self.model_dir, &self.config) {
            Ok(p) => {
                *guard = Some(p);
                tracing::info!("OpenVINO GenAI pipeline creation complete");
            }
            Err(e) => {
                tracing::error!("Failed to create pipeline in prepare(): {}", e);
            }
        }
    }

    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / 16000.0;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with OpenVINO GenAI (device={})",
            duration_secs,
            samples.len(),
            self.config.device
        );

        let start = std::time::Instant::now();

        // Get pipeline and run inference
        let mut guard = self.ensure_pipeline()?;
        let pipeline = guard.as_mut().unwrap();

        // Get config from the pipeline (inherits model-specific token IDs).
        // A standalone WhisperGenerationConfig::new() uses generic defaults that may
        // not match the model; WhisperGenerationConfig::from_json() with the model's
        // generation_config.json is the alternative for standalone creation.
        let mut gen_config = pipeline.get_generation_config().map_err(|e| {
            TranscribeError::InferenceFailed(format!(
                "Failed to get generation config: {}",
                e
            ))
        })?;

        // Only set language/task on multilingual models (*.en models are English-only
        // and reject language/task overrides)
        let is_multilingual = gen_config
            .get_is_multilingual()
            .unwrap_or(false);

        if is_multilingual {
            // GenAI expects language tokens in "<|xx|>" format (matching lang_to_id keys
            // in generation_config.json), while voxtype config uses bare codes like "en"
            let lang = &self.config.language;
            let lang_token = if lang.starts_with("<|") {
                lang.to_string()
            } else {
                format!("<|{}|>", lang)
            };
            gen_config.set_language(&lang_token).map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to set language: {}", e))
            })?;

            let task = if self.config.translate {
                "translate"
            } else {
                "transcribe"
            };
            gen_config.set_task(task).map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to set task: {}", e))
            })?;
        } else if self.config.translate {
            tracing::warn!(
                "Translation requested but model is not multilingual; ignoring translate setting"
            );
        }

        gen_config.set_return_timestamps(false).map_err(|e| {
            TranscribeError::InferenceFailed(format!(
                "Failed to set return_timestamps: {}",
                e
            ))
        })?;

        let results = pipeline.generate(samples, Some(&gen_config)).map_err(|e| {
            TranscribeError::InferenceFailed(format!(
                "OpenVINO GenAI inference failed: {}",
                e
            ))
        })?;

        let text = results.get_string().map_err(|e| {
            TranscribeError::InferenceFailed(format!(
                "Failed to get transcription string: {}",
                e
            ))
        })?;

        let result = text.trim().to_string();

        // Log performance metrics if available
        if let Ok(metrics) = results.get_perf_metrics() {
            if let Ok((gen_dur, _)) = metrics.get_generate_duration() {
                tracing::debug!("GenAI generate duration: {:.0}ms", gen_dur);
            }
        }

        tracing::info!(
            "OpenVINO GenAI transcription completed in {:.2}s: {:?}",
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

/// Find the libopenvino_genai_c shared library in a directory.
fn find_genai_library(dir: &str) -> Result<PathBuf, TranscribeError> {
    let dir_path = PathBuf::from(dir);
    if !dir_path.is_dir() {
        return Err(TranscribeError::InitFailed(format!(
            "openvino_dir is not a directory: {}",
            dir
        )));
    }

    let lib_name = format!("{}openvino_genai_c{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
    let direct = dir_path.join(&lib_name);
    if direct.is_file() {
        return Ok(direct);
    }

    // Search known subdirectories
    for subdir in &[
        "runtime/lib/intel64",
        "runtime/lib/intel64/Release",
        ".",
    ] {
        let path = dir_path.join(subdir).join(&lib_name);
        if path.is_file() {
            return Ok(path);
        }
    }

    Err(TranscribeError::InitFailed(format!(
        "{} not found in {}\n  \
         Set openvino_dir to the directory containing the library,\n  \
         or to the OpenVINO installation root.",
        lib_name, dir
    )))
}

/// Preload OpenVINO dependency libraries from the given directory using
/// `RTLD_LAZY | RTLD_GLOBAL`. This makes their symbols globally available so
/// that the subsequent dlopen of `libopenvino_genai_c.so` can resolve its
/// DT_NEEDED entries without requiring LD_LIBRARY_PATH to be set before
/// process startup.
fn preload_openvino_deps(lib_dir: &std::path::Path) {
    use std::ffi::CString;

    // Order matters: libopenvino.so first (base dependency), then the others.
    let deps = ["libopenvino.so", "libopenvino_c.so", "libopenvino_genai.so"];

    for name in &deps {
        let path = lib_dir.join(name);
        if !path.exists() {
            continue;
        }
        let Some(c_path) = path.to_str().and_then(|s| CString::new(s).ok()) else {
            continue;
        };
        let handle =
            unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL) };
        if handle.is_null() {
            tracing::warn!("Failed to preload {}", path.display());
        } else {
            tracing::debug!("Preloaded {}", path.display());
            // Intentionally not calling dlclose — keep symbols available.
        }
    }
}

/// Check if a model name already includes a quantization suffix (-int4, -int8, -fp16)
fn has_quant_suffix(name: &str) -> bool {
    name.ends_with("-int4") || name.ends_with("-int8") || name.ends_with("-fp16")
}

/// Resolve model name to directory path.
///
/// Handles several naming conventions:
/// - Absolute paths: used directly
/// - Full dir names: "openvino-whisper-base.en-int8-ov"
/// - Short names with quantization: "base.en-int8" (from `voxtype setup model`)
/// - Short names without quantization: "base.en" (uses `quantized` flag)
/// - Distil models: "distil-large-v2-int8" → "openvino-distil-whisper-large-v2-int8-ov"
fn resolve_model_path(model: &str, quantized: bool) -> Result<PathBuf, TranscribeError> {
    // If it's already an absolute path, use it directly
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    // If the model name already has a quantization suffix, don't add another one.
    // Names from `voxtype setup model` include quantization (e.g., "base.en-int8").
    let already_quantized = has_quant_suffix(model);
    let quant_suffix = if already_quantized {
        ""
    } else if quantized {
        "-int8"
    } else {
        "-fp16"
    };

    // Build candidate directory names.
    // Models from setup have names like "base.en-int8" → dir "openvino-whisper-base.en-int8-ov"
    // Distil models: "distil-large-v2-int8" → dir "openvino-distil-whisper-large-v2-int8-ov"
    let mut candidates: Vec<String> = Vec::new();

    if model.starts_with("openvino-") {
        // Already a full directory name (e.g., "openvino-whisper-base.en-int8-ov")
        candidates.push(model.to_string());
    } else if model.starts_with("whisper-") {
        // e.g., "whisper-base.en-int8" → "openvino-whisper-base.en-int8-ov"
        candidates.push(format!("openvino-{}{}-ov", model, quant_suffix));
        candidates.push(format!("openvino-{}-ov", model));
    } else if let Some(rest) = model.strip_prefix("distil-") {
        // e.g., "distil-large-v2-int8" → "openvino-distil-whisper-large-v2-int8-ov"
        candidates.push(format!("openvino-distil-whisper-{}{}-ov", rest, quant_suffix));
        candidates.push(format!("openvino-distil-whisper-{}-ov", rest));
        // Also try the non-distil pattern in case naming differs
        candidates.push(format!("openvino-whisper-{}{}-ov", model, quant_suffix));
        candidates.push(format!("openvino-whisper-{}-ov", model));
    } else {
        // Short name: "base.en-int8" or "base.en"
        candidates.push(format!("openvino-whisper-{}{}-ov", model, quant_suffix));
        candidates.push(format!("openvino-whisper-{}-ov", model));
    }

    // Search locations
    let models_dir = crate::config::Config::models_dir();
    let mut search_paths: Vec<PathBuf> = Vec::new();
    for candidate in &candidates {
        search_paths.push(models_dir.join(candidate));
    }
    for candidate in &candidates {
        search_paths.push(PathBuf::from(candidate));
        search_paths.push(PathBuf::from("models").join(candidate));
    }

    for search_path in &search_paths {
        if search_path.exists() && search_path.join("openvino_encoder_model.xml").exists() {
            return Ok(search_path.clone());
        }
    }

    // Not found - build helpful error message
    let searched: Vec<String> = search_paths
        .iter()
        .map(|p| format!("  - {}", p.display()))
        .collect();

    let model_with_quant = if already_quantized {
        model.to_string()
    } else {
        format!("{}{}", model, quant_suffix)
    };
    let hf_repo = format!("whisper-{}-ov", model_with_quant);

    Err(TranscribeError::ModelNotFound(format!(
        "OpenVINO Whisper model '{}' not found. Looked in:\n{}\n\n  \
         Run 'voxtype setup model' to download, or manually from:\n  \
         https://huggingface.co/OpenVINO/{}",
        model,
        searched.join("\n"),
        hf_repo
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_model_path_absolute() {
        let result = resolve_model_path("/nonexistent/path", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_model_path_not_found() {
        let result = resolve_model_path("nonexistent-model", true);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
        assert!(err.contains("huggingface.co"));
    }

    #[test]
    fn test_has_quant_suffix() {
        assert!(has_quant_suffix("base.en-int8"));
        assert!(has_quant_suffix("tiny-int4"));
        assert!(has_quant_suffix("large-v3-fp16"));
        assert!(has_quant_suffix("distil-large-v2-int8"));
        assert!(!has_quant_suffix("base.en"));
        assert!(!has_quant_suffix("large-v3"));
        assert!(!has_quant_suffix("tiny"));
    }

    #[test]
    fn test_resolve_no_double_quant_suffix() {
        // When model name already has quantization (from `voxtype setup model`),
        // should NOT produce doubled suffixes like "base.en-int8-int8"
        let result = resolve_model_path("base.en-int8", true);
        match result {
            Ok(path) => {
                // Model exists on disk - verify it resolved to the right dir
                let dir_name = path.file_name().unwrap().to_str().unwrap();
                assert_eq!(dir_name, "openvino-whisper-base.en-int8-ov");
            }
            Err(err) => {
                let err = err.to_string();
                assert!(
                    err.contains("openvino-whisper-base.en-int8-ov"),
                    "Expected 'openvino-whisper-base.en-int8-ov' in error, got: {}",
                    err
                );
                assert!(
                    !err.contains("base.en-int8-int8"),
                    "Found doubled quantization suffix in error: {}",
                    err
                );
            }
        }
    }

    #[test]
    fn test_resolve_short_name_gets_quant_suffix() {
        // Short name without quantization should get suffix from `quantized` flag
        let result = resolve_model_path("base.en", true);
        match result {
            Ok(path) => {
                let dir_name = path.file_name().unwrap().to_str().unwrap();
                assert_eq!(dir_name, "openvino-whisper-base.en-int8-ov");
            }
            Err(err) => {
                let err = err.to_string();
                assert!(
                    err.contains("openvino-whisper-base.en-int8-ov"),
                    "Expected int8 suffix for quantized=true, got: {}",
                    err
                );
            }
        }

        // Use a model name unlikely to exist on disk to test fp16 path
        let result = resolve_model_path("nonexistent-model", false);
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("openvino-whisper-nonexistent-model-fp16-ov"),
            "Expected fp16 suffix for quantized=false, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_distil_model_path() {
        // Use a distil model that won't exist on disk
        let result = resolve_model_path("distil-large-v2-int4", true);
        match result {
            Ok(path) => {
                let dir_name = path.file_name().unwrap().to_str().unwrap();
                assert_eq!(dir_name, "openvino-distil-whisper-large-v2-int4-ov");
            }
            Err(err) => {
                let err = err.to_string();
                assert!(
                    err.contains("openvino-distil-whisper-large-v2-int4-ov"),
                    "Expected distil dir pattern in error, got: {}",
                    err
                );
            }
        }
    }

    /// Real-life integration test: loads a WAV file and transcribes with OpenVINO GenAI.
    /// Requires: model files in ~/.local/share/voxtype/models/, OpenVINO GenAI libs, NPU device.
    /// Run with: cargo test --features openvino-whisper -- test_openvino_real --nocapture --ignored
    #[test]
    #[ignore]
    fn test_openvino_real_transcription() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("debug")
            .try_init();

        // Load WAV file (16-bit PCM, mono, 16kHz)
        let wav_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sensevoice/ja.wav");
        assert!(wav_path.exists(), "Test WAV not found: {:?}", wav_path);

        let mut reader = hound::WavReader::open(&wav_path).expect("Failed to open WAV");
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, 16000, "Expected 16kHz audio");
        assert_eq!(spec.channels, 1, "Expected mono audio");

        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();
        println!(
            "Loaded {} samples ({:.2}s)",
            samples.len(),
            samples.len() as f32 / 16000.0
        );

        // Create transcriber - use env vars for device and model override
        let device = std::env::var("VOXTYPE_OPENVINO_DEVICE").unwrap_or_else(|_| "CPU".to_string());
        let model = std::env::var("VOXTYPE_OPENVINO_MODEL").unwrap_or_else(|_| "base".to_string());
        let config = OpenVinoConfig {
            model,
            device: device.clone(),
            quantized: true,
            openvino_dir: std::env::var("VOXTYPE_OPENVINO_DIR").ok(),
            ..OpenVinoConfig::default()
        };

        let transcriber =
            OpenVinoTranscriber::new(&config).expect("Failed to create OpenVINO transcriber");

        // Prepare (create pipeline)
        println!("Creating pipeline for NPU...");
        transcriber.prepare();

        // Transcribe
        println!("Transcribing...");
        let result = transcriber.transcribe(&samples);
        match &result {
            Ok(text) => println!("Transcription result: {:?}", text),
            Err(e) => println!("Transcription error: {}", e),
        }
        assert!(result.is_ok(), "Transcription failed: {:?}", result.err());

        let text = result.unwrap();
        assert!(!text.is_empty(), "Transcription produced empty text");
        println!("SUCCESS: {:?}", text);
    }
}
