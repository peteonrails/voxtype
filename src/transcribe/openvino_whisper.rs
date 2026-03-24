//! OpenVINO Whisper speech-to-text transcription
//!
//! Uses OpenVINO Runtime to run Whisper encoder-decoder models on Intel NPU, CPU,
//! or GPU. Models are in OpenVINO IR format (.xml + .bin) from HuggingFace
//! (OpenVINO/whisper-* repos).
//!
//! Architecture:
//! - Encoder: processes mel spectrogram [1, 80, 3000], outputs hidden states
//! - Decoder: autoregressive transformer, generates tokens with greedy decoding
//! - Tokenizer: Whisper BPE tokenizer for token-to-text conversion
//!
//! The mel spectrogram is extracted using the shared fbank module with Whisper
//! settings (Hann window, no pre-emphasis, 80 mel bins).

use super::fbank::{FbankConfig, FbankExtractor};
use super::Transcriber;
use crate::config::OpenVinoConfig;
use crate::error::TranscribeError;
use openvino::{Core, DeviceType, ElementType, InferRequest, Shape, Tensor};
use std::path::PathBuf;
use std::sync::Mutex;
use tokenizers::Tokenizer;

/// Whisper special token IDs (standard across all Whisper model sizes)
const SOT_TOKEN: i64 = 50258; // <|startoftranscript|>
const EOT_TOKEN: i64 = 50257; // <|endoftext|>
const TRANSCRIBE_TOKEN: i64 = 50359; // <|transcribe|>
const TRANSLATE_TOKEN: i64 = 50358; // <|translate|>
const NO_TIMESTAMPS_TOKEN: i64 = 50363; // <|notimestamps|>

/// Language token offset: language tokens start at 50259
/// e.g., <|en|> = 50259, <|zh|> = 50260, etc.
const LANGUAGE_TOKEN_BASE: i64 = 50259;

/// Maximum tokens to generate (Whisper limit)
const MAX_NEW_TOKENS: usize = 448;

/// Whisper mel spectrogram parameters
const WHISPER_SAMPLE_RATE: usize = 16000;
const WHISPER_N_FRAMES: usize = 3000; // 30 seconds at 100 fps (10ms hop)
const WHISPER_N_MELS: usize = 80;

/// Whisper language code to token ID mapping
fn language_token_id(lang: &str) -> i64 {
    // Subset of Whisper language codes - full list has 99 languages
    let offset = match lang {
        "en" => 0,
        "zh" => 1,
        "de" => 2,
        "es" => 3,
        "ru" => 4,
        "ko" => 5,
        "fr" => 6,
        "ja" => 7,
        "pt" => 8,
        "tr" => 9,
        "pl" => 10,
        "ca" => 11,
        "nl" => 12,
        "ar" => 13,
        "sv" => 14,
        "it" => 15,
        "id" => 16,
        "hi" => 17,
        "fi" => 18,
        "vi" => 19,
        "he" => 20,
        "uk" => 21,
        "el" => 22,
        "ms" => 23,
        "cs" => 24,
        "ro" => 25,
        "da" => 26,
        "hu" => 27,
        "ta" => 28,
        "no" => 29,
        "th" => 30,
        "ur" => 31,
        "hr" => 32,
        "bg" => 33,
        "lt" => 34,
        "la" => 35,
        "mi" => 36,
        "ml" => 37,
        "cy" => 38,
        "sk" => 39,
        "te" => 40,
        "fa" => 41,
        "lv" => 42,
        "bn" => 43,
        "sr" => 44,
        "az" => 45,
        "sl" => 46,
        "kn" => 47,
        "et" => 48,
        "mk" => 49,
        "br" => 50,
        "eu" => 51,
        "is" => 52,
        "hy" => 53,
        "ne" => 54,
        "mn" => 55,
        "bs" => 56,
        "kk" => 57,
        "sq" => 58,
        "sw" => 59,
        "gl" => 60,
        "mr" => 61,
        "pa" => 62,
        "si" => 63,
        "km" => 64,
        "sn" => 65,
        "yo" => 66,
        "so" => 67,
        "af" => 68,
        "oc" => 69,
        "ka" => 70,
        "be" => 71,
        "tg" => 72,
        "sd" => 73,
        "gu" => 74,
        "am" => 75,
        "yi" => 76,
        "lo" => 77,
        "uz" => 78,
        "fo" => 79,
        "ht" => 80,
        "ps" => 81,
        "tk" => 82,
        "nn" => 83,
        "mt" => 84,
        "sa" => 85,
        "lb" => 86,
        "my" => 87,
        "bo" => 88,
        "tl" => 89,
        "mg" => 90,
        "as" => 91,
        "tt" => 92,
        "haw" => 93,
        "ln" => 94,
        "ha" => 95,
        "ba" => 96,
        "jw" => 97,
        "su" => 98,
        _ => {
            tracing::warn!(
                "Unknown language code '{}', defaulting to English",
                lang
            );
            0 // English
        }
    };
    LANGUAGE_TOKEN_BASE + offset
}

/// Cached inference requests from compiled OpenVINO models.
/// Created during `prepare()` and reused across transcriptions.
struct CompiledModels {
    encoder_request: InferRequest,
    decoder_request: InferRequest,
}

/// OpenVINO-based Whisper transcriber for Intel NPU/CPU/GPU.
///
/// Model compilation is deferred to `prepare()` (called when recording starts),
/// hiding the compilation latency behind recording time. Compiled models are
/// cached and reused across transcriptions. If `prepare()` was not called,
/// compilation happens on first `transcribe()` call.
pub struct OpenVinoTranscriber {
    /// Cached compiled models (None until prepare() or first transcribe())
    compiled: Mutex<Option<CompiledModels>>,
    /// BPE tokenizer for decoding token IDs to text
    tokenizer: Tokenizer,
    /// Mel spectrogram extractor (Whisper-specific config)
    mel_extractor: FbankExtractor,
    /// Resolved model directory path
    model_dir: PathBuf,
    /// Configuration
    config: OpenVinoConfig,
}

impl OpenVinoTranscriber {
    /// Create a new OpenVINO Whisper transcriber.
    ///
    /// This only validates model files and loads the tokenizer (lightweight).
    /// The expensive model compilation is deferred to `prepare()` or first `transcribe()`.
    pub fn new(config: &OpenVinoConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model, config.quantized)?;

        tracing::info!(
            "Initializing OpenVINO Whisper from {:?} (device={}, quantized={})",
            model_dir,
            config.device,
            config.quantized
        );

        // Validate model files exist (fast check, no loading)
        for (filename, desc) in [
            ("openvino_encoder_model.xml", "encoder XML"),
            ("openvino_encoder_model.bin", "encoder BIN"),
            ("openvino_decoder_model.xml", "decoder XML"),
            ("openvino_decoder_model.bin", "decoder BIN"),
        ] {
            let path = model_dir.join(filename);
            if !path.exists() {
                return Err(TranscribeError::ModelNotFound(format!(
                    "OpenVINO Whisper {} not found: {}\n  \
                     Run 'voxtype setup model' to download, or manually from:\n  \
                     https://huggingface.co/OpenVINO/whisper-{}",
                    desc,
                    path.display(),
                    config.model
                )));
            }
        }

        // Load tokenizer (lightweight, needed for both prepare and transcribe)
        let tokenizer_path = model_dir.join("tokenizer.json");
        if !tokenizer_path.exists() {
            return Err(TranscribeError::InitFailed(format!(
                "OpenVINO Whisper tokenizer not found: {}\n  \
                 Ensure tokenizer.json is in the model directory.",
                tokenizer_path.display()
            )));
        }
        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            TranscribeError::InitFailed(format!("Failed to load tokenizer: {}", e))
        })?;

        let mel_extractor = FbankExtractor::new(FbankConfig::whisper());

        tracing::info!(
            "OpenVINO Whisper initialized (tokenizer loaded, model compilation deferred to prepare())"
        );

        Ok(Self {
            compiled: Mutex::new(None),
            tokenizer,
            mel_extractor,
            model_dir,
            config: config.clone(),
        })
    }

    /// Compile models for the target device (NPU/CPU/GPU).
    /// This is the expensive operation that `prepare()` hides behind recording time.
    /// Parse device string into DeviceType
    fn parse_device(device_str: &str) -> DeviceType<'static> {
        match device_str.to_uppercase().as_str() {
            "NPU" => DeviceType::NPU,
            "CPU" => DeviceType::CPU,
            "GPU" => DeviceType::GPU,
            _ => DeviceType::Other(std::borrow::Cow::Owned(device_str.to_uppercase())),
        }
    }

    fn compile_models(model_dir: &std::path::Path, config: &OpenVinoConfig) -> Result<CompiledModels, TranscribeError> {
        let start = std::time::Instant::now();

        let mut core = Core::new().map_err(|e| {
            TranscribeError::InitFailed(format!(
                "OpenVINO initialization failed: {}\n  \
                 Install OpenVINO: https://docs.openvino.ai/latest/openvino_docs_install_guides_installing_openvino_linux.html\n  \
                 Or: pip install openvino (includes shared libraries)",
                e
            ))
        })?;

        let encoder_xml = model_dir.join("openvino_encoder_model.xml");
        let encoder_bin = model_dir.join("openvino_encoder_model.bin");
        let decoder_xml = model_dir.join("openvino_decoder_model.xml");
        let decoder_bin = model_dir.join("openvino_decoder_model.bin");

        let is_npu = config.device.to_uppercase() == "NPU";

        // Load and compile encoder
        tracing::debug!("Compiling encoder model for {}...", config.device);
        let encoder_model = core
            .read_model_from_file(
                encoder_xml.to_str().unwrap_or_default(),
                encoder_bin.to_str().unwrap_or_default(),
            )
            .map_err(|e| {
                TranscribeError::InitFailed(format!("Failed to load encoder model: {}", e))
            })?;

        let mut compiled_encoder = core.compile_model(&encoder_model, Self::parse_device(&config.device)).map_err(|e| {
            if is_npu {
                TranscribeError::InitFailed(format!(
                    "Failed to compile encoder for NPU: {}\n  \
                     NPU device may not be available. Ensure intel-npu-driver is installed.\n  \
                     Check: ls /dev/accel/accel*\n  \
                     Or set device = \"CPU\" in [openvino] config to use CPU fallback.",
                    e
                ))
            } else {
                TranscribeError::InitFailed(format!(
                    "Failed to compile encoder for {}: {}",
                    config.device, e
                ))
            }
        })?;

        let encoder_request = compiled_encoder.create_infer_request().map_err(|e| {
            TranscribeError::InitFailed(format!("Failed to create encoder request: {}", e))
        })?;

        // Load and compile decoder
        tracing::debug!("Compiling decoder model for {}...", config.device);
        let decoder_model = core
            .read_model_from_file(
                decoder_xml.to_str().unwrap_or_default(),
                decoder_bin.to_str().unwrap_or_default(),
            )
            .map_err(|e| {
                TranscribeError::InitFailed(format!("Failed to load decoder model: {}", e))
            })?;

        let mut compiled_decoder = core.compile_model(&decoder_model, Self::parse_device(&config.device)).map_err(|e| {
            TranscribeError::InitFailed(format!(
                "Failed to compile decoder for {}: {}",
                config.device, e
            ))
        })?;

        let decoder_request = compiled_decoder.create_infer_request().map_err(|e| {
            TranscribeError::InitFailed(format!("Failed to create decoder request: {}", e))
        })?;

        tracing::info!(
            "OpenVINO models compiled in {:.2}s (device={})",
            start.elapsed().as_secs_f32(),
            config.device,
        );

        Ok(CompiledModels {
            encoder_request,
            decoder_request,
        })
    }

    /// Ensure models are compiled, compiling on first use if needed.
    /// Returns a mutable reference to the compiled models.
    fn ensure_compiled(&self) -> Result<std::sync::MutexGuard<'_, Option<CompiledModels>>, TranscribeError> {
        let mut guard = self.compiled.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock compiled models: {}", e))
        })?;

        if guard.is_none() {
            tracing::info!("Models not yet compiled, compiling now (prepare() was not called)");
            *guard = Some(Self::compile_models(&self.model_dir, &self.config)?);
        }

        Ok(guard)
    }

    /// Extract mel spectrogram and pad/transpose to Whisper format [1, 80, 3000]
    fn extract_mel(&self, samples: &[f32]) -> Vec<f32> {
        let fbank = self.mel_extractor.extract(samples);
        let num_frames = fbank.nrows();
        let num_mels = fbank.ncols();

        // Pad or truncate to WHISPER_N_FRAMES (3000)
        let target_frames = WHISPER_N_FRAMES;

        // Output in [1, 80, 3000] layout (batch, mels, frames) - row-major
        let mut mel = vec![0.0f32; WHISPER_N_MELS * target_frames];
        let frames_to_copy = num_frames.min(target_frames);

        for mel_idx in 0..num_mels.min(WHISPER_N_MELS) {
            for frame_idx in 0..frames_to_copy {
                mel[mel_idx * target_frames + frame_idx] = fbank[[frame_idx, mel_idx]];
            }
            // Remaining frames are already zero (padding)
        }

        mel
    }

    /// Build the decoder prompt token sequence
    fn build_decoder_prompt(&self) -> Vec<i64> {
        let lang_token = language_token_id(&self.config.language);
        let task_token = if self.config.translate {
            TRANSLATE_TOKEN
        } else {
            TRANSCRIBE_TOKEN
        };

        vec![SOT_TOKEN, lang_token, task_token, NO_TIMESTAMPS_TOKEN]
    }

    /// Run the full inference pipeline using cached compiled models
    fn run_inference(&self, samples: &[f32]) -> Result<Vec<u32>, TranscribeError> {
        let duration_secs = samples.len() as f32 / WHISPER_SAMPLE_RATE as f32;

        // --- Mel spectrogram extraction ---
        let mel_start = std::time::Instant::now();
        let mel_data = self.extract_mel(samples);
        tracing::debug!(
            "Mel extraction completed in {:.2}s",
            mel_start.elapsed().as_secs_f32()
        );

        // Get compiled models (compiles on first use if prepare() wasn't called)
        let mut guard = self.ensure_compiled()?;
        let models = guard.as_mut().unwrap();

        // --- Encoder ---
        let encoder_start = std::time::Instant::now();

        // Create mel tensor [1, 80, 3000]
        let mel_shape = Shape::new(&[1, WHISPER_N_MELS as i64, WHISPER_N_FRAMES as i64])
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to create mel shape: {}", e))
            })?;
        let mut mel_tensor = Tensor::new(ElementType::F32, &mel_shape).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to create mel tensor: {}", e))
        })?;
        mel_tensor
            .get_data_mut::<f32>()
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to get mel tensor data: {}", e))
            })?
            .copy_from_slice(&mel_data);

        // Set input and run encoder
        models
            .encoder_request
            .set_input_tensor_by_index(0,&mel_tensor)
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to set encoder input: {}", e))
            })?;

        models.encoder_request.infer().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Encoder inference failed: {}", e))
        })?;

        tracing::debug!(
            "Encoder completed in {:.2}s",
            encoder_start.elapsed().as_secs_f32()
        );

        // --- Decoder (autoregressive loop) ---
        let decoder_start = std::time::Instant::now();
        let prompt = self.build_decoder_prompt();
        let max_tokens = ((duration_secs * 6.0) as usize).clamp(16, MAX_NEW_TOKENS);

        let mut generated_tokens: Vec<i64> = prompt.clone();

        // Get encoder output to feed into decoder
        let encoder_output = models.encoder_request.get_output_tensor_by_index(0).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to get encoder output: {}", e))
        })?;

        for step in 0..max_tokens {
            // Create input_ids tensor
            let input_tokens: Vec<i64> = if step == 0 {
                generated_tokens.clone()
            } else {
                vec![*generated_tokens.last().unwrap()]
            };

            let ids_shape = Shape::new(&[1, input_tokens.len() as i64]).map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create input_ids shape: {}",
                    e
                ))
            })?;
            let mut ids_tensor = Tensor::new(ElementType::I64, &ids_shape).map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create input_ids tensor: {}",
                    e
                ))
            })?;
            ids_tensor
                .get_data_mut::<i64>()
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!(
                        "Failed to get input_ids data: {}",
                        e
                    ))
                })?
                .copy_from_slice(&input_tokens);

            // Set decoder inputs: input_ids + encoder_hidden_states
            models
                .decoder_request
                .set_input_tensor_by_index(0,&ids_tensor)
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!(
                        "Failed to set decoder input_ids: {}",
                        e
                    ))
                })?;

            models
                .decoder_request
                .set_input_tensor_by_index(1,&encoder_output)
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!(
                        "Failed to set encoder hidden states: {}",
                        e
                    ))
                })?;

            // Run decoder step
            models.decoder_request.infer().map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Decoder inference failed at step {}: {}",
                    step, e
                ))
            })?;

            // Extract logits from output 0
            let logits_tensor =
                models.decoder_request.get_output_tensor_by_index(0).map_err(|e| {
                    TranscribeError::InferenceFailed(format!(
                        "Failed to get decoder output: {}",
                        e
                    ))
                })?;

            let logits_shape = logits_tensor.get_shape().map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to get logits shape: {}", e))
            })?;
            let logits_dims = logits_shape.get_dimensions();
            let logits_data = logits_tensor.get_data::<f32>().map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to extract logits data: {}", e))
            })?;

            // Get last position logits for greedy decoding
            let vocab_size = *logits_dims.last().unwrap_or(&0) as usize;
            if vocab_size == 0 {
                return Err(TranscribeError::InferenceFailed(
                    "Decoder produced empty logits".to_string(),
                ));
            }

            let last_position_offset = logits_data.len() - vocab_size;
            let vocab_logits = &logits_data[last_position_offset..];

            // Greedy decode: argmax
            let next_token = vocab_logits
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| {
                    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(idx, _)| idx as i64)
                .ok_or_else(|| {
                    TranscribeError::InferenceFailed("Empty logits vector".to_string())
                })?;

            // Check for end of text
            if next_token == EOT_TOKEN {
                tracing::debug!("Decoder reached EOT at step {}", step);
                break;
            }

            generated_tokens.push(next_token);
        }

        tracing::debug!(
            "Decoder completed in {:.2}s ({} tokens)",
            decoder_start.elapsed().as_secs_f32(),
            generated_tokens.len() - prompt.len()
        );

        // Convert tokens to u32, skip the prompt tokens
        let token_ids: Vec<u32> = generated_tokens
            .iter()
            .skip(prompt.len())
            .map(|&t| t as u32)
            .collect();

        Ok(token_ids)
    }
}

impl Transcriber for OpenVinoTranscriber {
    /// Compile models for the target device, caching the result.
    ///
    /// Called when recording starts to hide compilation latency behind recording time.
    /// The compiled models are reused on all subsequent transcriptions.
    /// If not called, compilation happens lazily on first `transcribe()`.
    fn prepare(&self) {
        let mut guard = match self.compiled.lock() {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("Failed to lock compiled models in prepare(): {}", e);
                return;
            }
        };

        if guard.is_some() {
            tracing::debug!("Models already compiled, skipping prepare()");
            return;
        }

        tracing::info!("Compiling OpenVINO models for {} (triggered by prepare())...", self.config.device);
        match Self::compile_models(&self.model_dir, &self.config) {
            Ok(models) => {
                *guard = Some(models);
                tracing::info!("OpenVINO model compilation complete");
            }
            Err(e) => {
                tracing::error!("Failed to compile models in prepare(): {}", e);
                // Will retry in transcribe() via ensure_compiled()
            }
        }
    }

    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / WHISPER_SAMPLE_RATE as f32;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with OpenVINO Whisper (device={})",
            duration_secs,
            samples.len(),
            self.config.device
        );

        let start = std::time::Instant::now();

        let token_ids = self.run_inference(samples)?;

        // Decode tokens to text
        let text = self
            .tokenizer
            .decode(&token_ids, true) // skip_special_tokens = true
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Tokenizer decode failed: {}", e))
            })?;

        let result = text.trim().to_string();

        tracing::info!(
            "OpenVINO Whisper transcription completed in {:.2}s: {:?}",
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

    let hf_repo = if model.contains('.') {
        // e.g., "base.en" -> "whisper-base.en-int8-ov"
        format!("whisper-{}{}-ov", model, quant_suffix)
    } else {
        format!("whisper-{}{}-ov", model, quant_suffix)
    };

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
    fn test_language_token_ids() {
        assert_eq!(language_token_id("en"), 50259);
        assert_eq!(language_token_id("zh"), 50260);
        assert_eq!(language_token_id("fr"), 50265);
        assert_eq!(language_token_id("ja"), 50266);
    }

    #[test]
    fn test_unknown_language_defaults_to_english() {
        assert_eq!(language_token_id("xx"), 50259); // Should default to English
    }

    #[test]
    fn test_decoder_prompt_transcribe() {
        let config = OpenVinoConfig {
            language: "en".to_string(),
            translate: false,
            ..OpenVinoConfig::default()
        };
        // We can't construct a full transcriber without models, but we can test the prompt logic
        let lang_token = language_token_id(&config.language);
        let task_token = TRANSCRIBE_TOKEN;
        let prompt = vec![SOT_TOKEN, lang_token, task_token, NO_TIMESTAMPS_TOKEN];
        assert_eq!(prompt, vec![50258, 50259, 50359, 50363]);
    }

    #[test]
    fn test_decoder_prompt_translate() {
        let config = OpenVinoConfig {
            language: "fr".to_string(),
            translate: true,
            ..OpenVinoConfig::default()
        };
        let lang_token = language_token_id(&config.language);
        let task_token = TRANSLATE_TOKEN;
        let prompt = vec![SOT_TOKEN, lang_token, task_token, NO_TIMESTAMPS_TOKEN];
        assert_eq!(prompt, vec![50258, 50265, 50358, 50363]);
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

    /// Real-life integration test: loads a WAV file and transcribes with OpenVINO.
    /// Requires: model files in ~/.local/share/voxtype/models/, OpenVINO libs, NPU device.
    /// Run with: OPENVINO_INSTALL_DIR=... cargo test --features openvino-whisper -- test_openvino_real --nocapture --ignored
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
        println!("Loaded {} samples ({:.2}s)", samples.len(), samples.len() as f32 / 16000.0);

        // Create transcriber with NPU device
        let config = OpenVinoConfig {
            model: "base.en".to_string(),
            device: "NPU".to_string(),
            quantized: true,
            ..OpenVinoConfig::default()
        };

        let transcriber = OpenVinoTranscriber::new(&config)
            .expect("Failed to create OpenVINO transcriber");

        // Prepare (compile models)
        println!("Compiling models for NPU...");
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
