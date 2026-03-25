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

/// Resolve model name to directory path
fn resolve_model_path(model: &str, quantized: bool) -> Result<PathBuf, TranscribeError> {
    // If it's already an absolute path, use it directly
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    // Build directory name variants to search for
    let quant_suffix = if quantized { "-int8" } else { "-fp16" };
    let base_name = if model.starts_with("openvino-whisper-") {
        model.to_string()
    } else if model.starts_with("whisper-") {
        format!("openvino-{}{}-ov", model, quant_suffix)
    } else {
        format!("openvino-whisper-{}{}-ov", model, quant_suffix)
    };

    // Also try without quantization suffix (user may have named it simply)
    let simple_name = if model.starts_with("openvino-whisper-") {
        model.to_string()
    } else if model.starts_with("whisper-") {
        format!("openvino-{}", model)
    } else {
        format!("openvino-whisper-{}", model)
    };

    // Search locations
    let models_dir = crate::config::Config::models_dir();
    let search_paths = [
        models_dir.join(&base_name),
        models_dir.join(&simple_name),
        PathBuf::from(&base_name),
        PathBuf::from(&simple_name),
        PathBuf::from("models").join(&base_name),
        PathBuf::from("models").join(&simple_name),
    ];

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

    let hf_repo = format!("whisper-{}{}-ov", model, quant_suffix);

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
