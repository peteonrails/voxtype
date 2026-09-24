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
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Directory OpenVINO persists compiled device blobs to (`CACHE_DIR`
/// property), so NPU/GPU graph compilation isn't repeated on every pipeline
/// init. Same `$XDG_CACHE_HOME/voxtype/<name>` convention as the MIGraphX
/// model cache in `setup/binary.rs`. Each model gets its own subdirectory
/// under this — see `scoped_model_cache_dir`.
fn openvino_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("voxtype")
        .join("openvino")
}

/// Per-model cache subdirectory: `<base>/<model dir name>/`. Scoping the
/// cache per model lets setup answer "is *this* model compiled?" — a shared
/// flat directory can only answer "was anything ever compiled?".
fn scoped_model_cache_dir(base: &Path, model_dir: &Path) -> PathBuf {
    match model_dir.file_name() {
        Some(name) => base.join(name),
        None => base.to_path_buf(),
    }
}

/// Whether this model already has a compiled device blob in its per-model
/// cache directory. Lets setup skip the NPU compile when there is nothing
/// left to do.
pub fn has_compiled_blob(config: &OpenVinoConfig) -> bool {
    resolve_model_path(&config.model, config.quantized)
        .map(|model_dir| {
            contains_compiled_blob(&scoped_model_cache_dir(&openvino_cache_dir(), &model_dir))
        })
        .unwrap_or(false)
}

/// Model-directory checks shared by transcriber construction and the
/// setup-time NPU compile.
fn require_model_files(model_dir: &Path, model_name: &str) -> Result<(), TranscribeError> {
    let encoder_xml = model_dir.join("openvino_encoder_model.xml");
    if !encoder_xml.exists() {
        return Err(TranscribeError::ModelNotFound(format!(
            "OpenVINO Whisper encoder model not found: {}\n  \
             Run 'voxtype setup model' to download, or manually from:\n  \
             https://huggingface.co/OpenVINO/whisper-{}",
            encoder_xml.display(),
            model_name
        )));
    }
    OpenVinoTranscriber::require_preprocessor_config(model_dir, model_name)
}

/// The validated inputs every WhisperPipeline construction needs regardless
/// of device: model files present, GenAI library loaded, per-model cache
/// directory created. Built once, then `build()` compiles for one explicit
/// device. The runtime fallback chain and the setup-time NPU compile both go
/// through this so the two sequences can't diverge.
struct PipelineInputs {
    model_path: String,
    cache_dir: PathBuf,
}

impl PipelineInputs {
    fn new(model_dir: &Path, config: &OpenVinoConfig) -> Result<Self, TranscribeError> {
        require_model_files(model_dir, &config.model)?;
        OpenVinoTranscriber::load_library(config)?;

        let model_path = model_dir
            .to_str()
            .ok_or_else(|| {
                TranscribeError::InitFailed("Model path contains invalid UTF-8".to_string())
            })?
            .to_string();

        // CACHE_DIR persists the device's compiled model blob to disk so
        // subsequent pipeline inits skip recompilation. This matters most
        // on NPU (compiling the graph to NPU-ISA) and GPU (OpenCL kernel
        // compilation) — both are expensive on cold start and otherwise pay
        // that cost on every daemon restart. CPU accepts the property too
        // but has little to gain from it (its compile step is cheap).
        let cache_dir = scoped_model_cache_dir(&openvino_cache_dir(), model_dir);
        fs::create_dir_all(&cache_dir).map_err(|error| {
            TranscribeError::InitFailed(format!(
                "Failed to create OpenVINO cache directory {}: {}",
                cache_dir.display(),
                error
            ))
        })?;

        Ok(Self {
            model_path,
            cache_dir,
        })
    }

    fn build(&self, device: &str) -> Result<WhisperPipeline, openvino_genai::SetupError> {
        let cache_dir_str = self.cache_dir.to_string_lossy();
        let props: &[(&str, &str)] = &[("CACHE_DIR", &cache_dir_str)];
        WhisperPipeline::with_properties(&self.model_path, device, props)
    }
}

/// Compile one downloaded model specifically for the NPU and wait until
/// OpenVINO has persisted its compiled blob.
///
/// Setup deliberately uses a strict NPU initialization here. The normal
/// transcription path falls back to GPU and CPU when an accelerator is not
/// available, but that would let `voxtype setup model` claim that NPU setup
/// succeeded without ever producing an NPU cache entry.
pub fn precompile_npu_model(config: &OpenVinoConfig) -> Result<PathBuf, TranscribeError> {
    let model_dir = resolve_model_path(&config.model, config.quantized)?;
    let inputs = PipelineInputs::new(&model_dir, config)?;

    // WhisperPipeline construction is synchronous: it returns only after the
    // NPU compiler has finished (or failed) and CACHE_DIR has been serviced.
    let _pipeline = inputs.build("NPU").map_err(|error| {
        TranscribeError::InitFailed(format!(
            "Failed to compile OpenVINO Whisper model '{}' for NPU: {}\n\n{}",
            config.model,
            error,
            config.installation_guidance(),
        ))
    })?;

    if !contains_compiled_blob(&inputs.cache_dir) {
        return Err(TranscribeError::InitFailed(format!(
            "OpenVINO finished compiling '{}' for NPU but did not create a cache blob in {}",
            config.model,
            inputs.cache_dir.display()
        )));
    }

    Ok(inputs.cache_dir)
}

fn contains_compiled_blob(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            contains_compiled_blob(&path)
        } else {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("blob"))
        }
    })
}

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

fn device_candidates(configured: &str) -> Vec<&str> {
    let mut candidates = vec![configured];
    for device in ["GPU", "CPU"] {
        if !candidates
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(device))
        {
            candidates.push(device);
        }
    }
    candidates
}

impl OpenVinoTranscriber {
    fn require_preprocessor_config(
        model_dir: &std::path::Path,
        model_name: &str,
    ) -> Result<(), TranscribeError> {
        let path = model_dir.join("preprocessor_config.json");
        if path.exists() {
            return Ok(());
        }

        Err(TranscribeError::ModelNotFound(format!(
            "OpenVINO Whisper model at {} is missing preprocessor_config.json. \
             OpenVINO reads this file on the first transcription, so pipeline startup \
             can otherwise succeed before inference fails with an opaque \"unknown exception\".\n  \
             Re-download the model with:\n  \
             voxtype setup --download --model {}",
            model_dir.display(),
            model_name,
        )))
    }

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
        require_model_files(&model_dir, &config.model)?;
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
            let lib_path = find_genai_library(dir).map_err(|error| {
                TranscribeError::InitFailed(format!(
                    "{}\n\n{}",
                    error,
                    config.installation_guidance()
                ))
            })?;
            // Preload OpenVINO dependency libraries with RTLD_GLOBAL so that dlopen
            // can resolve the DT_NEEDED entries in libopenvino_genai_c.so. The OpenVINO
            // shared libraries don't set RPATH/RUNPATH, and glibc caches LD_LIBRARY_PATH
            // at startup so setting it at runtime has no effect.
            if let Some(lib_dir) = lib_path.parent() {
                preload_openvino_deps(lib_dir);
            }
            tracing::info!(
                "Loading OpenVINO GenAI library from: {}",
                lib_path.display()
            );
            openvino_genai::load_from(&lib_path).map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load OpenVINO GenAI library from {}: {}\n  \
                     Ensure libopenvino_genai_c.so exists in the specified openvino_dir.\n\n{}",
                    lib_path.display(),
                    e,
                    config.installation_guidance(),
                ))
            })
        } else {
            openvino_genai::load().map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load OpenVINO GenAI library: {}\n  \
                     Automatic discovery did not find libopenvino_genai_c.so.\n\n{}",
                    e,
                    config.installation_guidance(),
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

        let inputs = PipelineInputs::new(model_dir, config)?;

        // NPU compiling a large model for the first time is *slow*, not
        // broken -- confirmed live: large-v3-int4's first NPU compile on
        // one machine took ~887s (the VPU compiler's tiling-strategy
        // search hits many ERROR_INPUT_TOO_BIG rejections along the way,
        // which look alarming in OV_NPU_LOG_LEVEL=LOG_DEBUG output but
        // aren't fatal -- it keeps searching and gets there). That cost
        // is genuinely one-time: CACHE_DIR above persists the compiled
        // blob, so every load after the first is fast (~2s, same
        // ballpark as a small model). Silence here for minutes with no
        // other feedback reads exactly like a hang, so say so up front
        // rather than let someone reasonably conclude it's stuck and
        // kill it before the compile ever finishes.
        if !config.device.eq_ignore_ascii_case("CPU") {
            tracing::info!(
                "Compiling for {} for the first time can take several minutes for a large \
                 model (confirmed: large-v3-int4 ~15min on one machine) -- this is normal, \
                 not a hang, and only happens once per model (cached afterward at {:?})",
                config.device,
                inputs.cache_dir,
            );
        }

        // Fallback chain: configured device, then GPU, then CPU -- for
        // when the configured device genuinely errors (missing driver,
        // no NPU on this chip, unsupported op, ...) rather than just
        // being slow to compile, which the warning above already covers.
        // CPU last since it's the slowest but always available; GPU
        // ahead of it since a large model that errors on NPU is more
        // likely to still run acceptably on GPU than fall back all the
        // way to CPU. Skips a device already tried (e.g. configured
        // device is already GPU or CPU) rather than retrying it.
        let candidates = device_candidates(&config.device);

        let mut pipeline = None;
        let mut first_error: Option<(String, openvino_genai::SetupError)> = None;
        for (i, device) in candidates.iter().enumerate() {
            match inputs.build(device) {
                Ok(p) => {
                    if i > 0 {
                        tracing::warn!(
                            "OpenVINO device '{}' failed to initialize ({}); fell back to '{}'. \
                             If '{}' was NPU, note a long compile isn't what triggers this --\
                             see this function's own comment on that -- so a genuine error here \
                             usually means something more fundamental (missing driver, \
                             unsupported op, no NPU on this chip).",
                            candidates[0],
                            first_error
                                .as_ref()
                                .map(|(_, e)| e.to_string())
                                .unwrap_or_default(),
                            device,
                            candidates[0],
                        );
                    }
                    pipeline = Some(p);
                    break;
                }
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some((device.to_string(), e));
                    }
                }
            }
        }

        let pipeline = pipeline.ok_or_else(|| {
            let (first_device, first_e) = first_error.expect("candidates is never empty");
            TranscribeError::InitFailed(format!(
                "Failed to create OpenVINO GenAI Whisper pipeline on any of {:?} -- first \
                 failure was on '{}': {}\n\n{}",
                candidates,
                first_device,
                first_e,
                config.installation_guidance(),
            ))
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
            tracing::info!("Pipeline not yet created, creating now (prepare() was not called)");
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
            TranscribeError::InferenceFailed(format!("Failed to get generation config: {}", e))
        })?;

        // Only set language/task on multilingual models (*.en models are English-only
        // and reject language/task overrides)
        let is_multilingual = gen_config.get_is_multilingual().unwrap_or(false);

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
            TranscribeError::InferenceFailed(format!("Failed to set return_timestamps: {}", e))
        })?;

        let results = pipeline.generate(samples, Some(&gen_config)).map_err(|e| {
            TranscribeError::InferenceFailed(format!("OpenVINO GenAI inference failed: {}", e))
        })?;

        let text = results.get_string().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to get transcription string: {}", e))
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

    let lib_name = format!(
        "{}openvino_genai_c{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    // Search known subdirectories, accepting both unversioned and versioned sonames.
    for subdir in &["runtime/lib/intel64", "runtime/lib/intel64/Release", "."] {
        let base_dir = dir_path.join(subdir);
        if let Some(path) = find_library_file(&base_dir, &lib_name) {
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
        let Some(path) = find_library_file(lib_dir, name) else {
            continue;
        };
        let Some(c_path) = path.to_str().and_then(|s| CString::new(s).ok()) else {
            continue;
        };
        let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL) };
        if handle.is_null() {
            tracing::warn!("Failed to preload {}", path.display());
        } else {
            tracing::debug!("Preloaded {}", path.display());
            // Intentionally not calling dlclose — keep symbols available.
        }
    }
}

fn find_library_file(dir: &std::path::Path, lib_name: &str) -> Option<PathBuf> {
    let direct = dir.join(lib_name);
    if direct.is_file() {
        return Some(direct);
    }

    let prefix = format!("{}.", lib_name);
    fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with(&prefix))
                .unwrap_or(false)
        })
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
        candidates.push(format!(
            "openvino-distil-whisper-{}{}-ov",
            rest, quant_suffix
        ));
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
    fn device_fallback_order_prefers_gpu_before_cpu() {
        assert_eq!(device_candidates("NPU"), ["NPU", "GPU", "CPU"]);
        assert_eq!(device_candidates("AUTO"), ["AUTO", "GPU", "CPU"]);
        assert_eq!(device_candidates("GPU"), ["GPU", "CPU"]);
        assert_eq!(device_candidates("gpu"), ["gpu", "CPU"]);
        assert_eq!(device_candidates("CPU"), ["CPU", "GPU"]);
    }

    #[test]
    fn compiled_blob_detection_checks_nested_cache_directories() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("NPU").join("model");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(!contains_compiled_blob(temp.path()));

        std::fs::write(nested.join("compiled.BLOB"), b"cache").unwrap();
        assert!(contains_compiled_blob(temp.path()));
    }

    #[test]
    fn compiled_blob_check_is_scoped_per_model() {
        let base = tempfile::tempdir().unwrap();
        let model_a = scoped_model_cache_dir(
            base.path(),
            Path::new("/models/openvino-whisper-tiny-int4-ov"),
        );
        let model_b = scoped_model_cache_dir(
            base.path(),
            Path::new("/models/openvino-whisper-base.en-int8-ov"),
        );
        assert_ne!(model_a, model_b);

        std::fs::create_dir_all(&model_a).unwrap();
        std::fs::write(model_a.join("compiled.blob"), b"cache").unwrap();

        // A blob compiled for model A must not satisfy model B's check.
        assert!(contains_compiled_blob(&model_a));
        assert!(!contains_compiled_blob(&model_b));
    }

    #[test]
    fn incomplete_model_fails_before_openvino_first_inference() {
        let temp = tempfile::tempdir().expect("create temporary model directory");

        let error = OpenVinoTranscriber::require_preprocessor_config(temp.path(), "medium-int4")
            .expect_err("missing preprocessor config must fail before pipeline use")
            .to_string();
        assert!(error.contains("preprocessor_config.json"));
        assert!(error.contains("first transcription"));
        assert!(error.contains("unknown exception"));
        assert!(error.contains("voxtype setup --download --model medium-int4"));

        std::fs::write(temp.path().join("preprocessor_config.json"), b"{}")
            .expect("create preprocessor config fixture");
        OpenVinoTranscriber::require_preprocessor_config(temp.path(), "medium-int4")
            .expect("complete model should pass preprocessor validation");
    }

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

    #[test]
    fn missing_runtime_error_has_configured_device_guidance() {
        let config = OpenVinoConfig {
            device: "GPU".to_string(),
            openvino_dir: Some("/definitely/not/an/openvino/sdk".to_string()),
            ..OpenVinoConfig::default()
        };

        let error = OpenVinoTranscriber::load_library(&config)
            .expect_err("a nonexistent SDK path must fail")
            .to_string();
        assert!(error.contains("libopenvino_genai_c.so"));
        assert!(error.contains("openvino-intel-gpu-plugin"));
        assert!(error.contains("intel-compute-runtime"));
        assert!(!error.contains("intel-npu-driver"));
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
