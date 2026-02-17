//! Moonshine-based speech-to-text transcription
//!
//! Uses Moonshine's encoder-decoder transformer model via ONNX Runtime (ort crate)
//! for fast, local transcription. Moonshine is optimized for edge devices and
//! processes variable-length audio without the 30-second padding that Whisper requires.
//!
//! Model architecture:
//! - Encoder: processes raw audio waveform, outputs encoded representations
//! - Decoder: autoregressive transformer with KV cache, generates tokens
//! - Tokenizer: BPE tokenizer (tokenizer.json) for token-to-text conversion
//!
//! Available models (English only):
//! - moonshine-tiny (27M params, ~52 MB) - fastest
//! - moonshine-base (61M params, ~120 MB) - better accuracy

use super::Transcriber;
use crate::config::MoonshineConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokenizers::Tokenizer;

/// Special token IDs for Moonshine decoder.
/// These match the tokenizer.json configuration for all current HuggingFace
/// Moonshine ONNX models (tiny, base, and multilingual variants).
const DECODER_START_TOKEN_ID: i64 = 1;
const EOS_TOKEN_ID: i64 = 2;

/// Maximum tokens to generate (safety limit)
/// Moonshine generates roughly 6-7 tokens per second of audio
const MAX_TOKENS_PER_SECOND: f32 = 8.0;
const ABSOLUTE_MAX_TOKENS: usize = 512;

/// Moonshine-based transcriber using ONNX Runtime
pub struct MoonshineTranscriber {
    /// ONNX session for the encoder model (Mutex because run() needs &mut)
    encoder: Mutex<Session>,
    /// ONNX session for the decoder model (Mutex because run() needs &mut)
    decoder: Mutex<Session>,
    /// BPE tokenizer for decoding token IDs to text
    tokenizer: Tokenizer,
    /// Whether using quantized model variant
    quantized: bool,
    /// Decoder input names (cached from model metadata)
    decoder_input_names: Vec<String>,
    /// Decoder output names (cached from model metadata)
    decoder_output_names: Vec<String>,
    /// Number of attention heads (8 for base, 6 for tiny)
    num_heads: usize,
    /// Dimension per attention head (52 for base, 44 for tiny)
    head_dim: usize,
}

impl MoonshineTranscriber {
    /// Create a new Moonshine transcriber
    pub fn new(config: &MoonshineConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;
        let quantized = config.quantized;

        tracing::info!(
            "Loading Moonshine model from {:?} (quantized={})",
            model_dir,
            quantized
        );
        let start = std::time::Instant::now();

        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));

        // Select model files based on quantized preference
        let (encoder_file, decoder_file) = if quantized {
            let enc_q = model_dir.join("encoder_model_quantized.onnx");
            let dec_q = model_dir.join("decoder_model_merged_quantized.onnx");
            if enc_q.exists() && dec_q.exists() {
                (enc_q, dec_q)
            } else {
                tracing::warn!("Quantized models not found, falling back to full precision");
                (
                    model_dir.join("encoder_model.onnx"),
                    model_dir.join("decoder_model_merged.onnx"),
                )
            }
        } else {
            (
                model_dir.join("encoder_model.onnx"),
                model_dir.join("decoder_model_merged.onnx"),
            )
        };

        // Validate model files exist
        if !encoder_file.exists() {
            return Err(TranscribeError::ModelNotFound(format!(
                "Moonshine encoder model not found: {}\n  \
                 Run 'voxtype setup model' to download Moonshine models.",
                encoder_file.display()
            )));
        }
        if !decoder_file.exists() {
            return Err(TranscribeError::ModelNotFound(format!(
                "Moonshine decoder model not found: {}\n  \
                 Run 'voxtype setup model' to download Moonshine models.",
                decoder_file.display()
            )));
        }

        // Load tokenizer
        let tokenizer_path = model_dir.join("tokenizer.json");
        if !tokenizer_path.exists() {
            return Err(TranscribeError::InitFailed(format!(
                "Moonshine tokenizer not found: {}\n  \
                 Ensure tokenizer.json is in the model directory.",
                tokenizer_path.display()
            )));
        }

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| TranscribeError::InitFailed(format!("Failed to load tokenizer: {}", e)))?;

        // Create ONNX sessions
        let encoder = Session::builder()
            .map_err(|e| {
                TranscribeError::InitFailed(format!("ONNX encoder session builder failed: {}", e))
            })?
            .with_intra_threads(threads)
            .map_err(|e| {
                TranscribeError::InitFailed(format!("Failed to set encoder threads: {}", e))
            })?
            .commit_from_file(&encoder_file)
            .map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load Moonshine encoder from {:?}: {}",
                    encoder_file, e
                ))
            })?;

        let decoder = Session::builder()
            .map_err(|e| {
                TranscribeError::InitFailed(format!("ONNX decoder session builder failed: {}", e))
            })?
            .with_intra_threads(threads)
            .map_err(|e| {
                TranscribeError::InitFailed(format!("Failed to set decoder threads: {}", e))
            })?
            .commit_from_file(&decoder_file)
            .map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load Moonshine decoder from {:?}: {}",
                    decoder_file, e
                ))
            })?;

        // Cache input/output names from model metadata
        let decoder_input_names: Vec<String> = decoder
            .inputs()
            .iter()
            .map(|i| i.name().to_string())
            .collect();
        let decoder_output_names: Vec<String> = decoder
            .outputs()
            .iter()
            .map(|o| o.name().to_string())
            .collect();

        // Log model input/output info for debugging
        tracing::debug!(
            "Encoder inputs: {:?}",
            encoder
                .inputs()
                .iter()
                .map(|i| i.name())
                .collect::<Vec<_>>()
        );
        tracing::debug!(
            "Encoder outputs: {:?}",
            encoder
                .outputs()
                .iter()
                .map(|o| o.name())
                .collect::<Vec<_>>()
        );
        tracing::debug!("Decoder inputs: {:?}", decoder_input_names);
        tracing::debug!("Decoder outputs: {:?}", decoder_output_names);

        // Detect num_heads and head_dim from KV cache input shape.
        // KV cache inputs have shape [batch, num_heads, seq_len, head_dim].
        // We find the first past_key_values input and read dimensions 1 and 3.
        // Shape deref to &[i64] where -1 means dynamic.
        let (num_heads, head_dim) = decoder
            .inputs()
            .iter()
            .find(|i| i.name().starts_with("past_key_values"))
            .and_then(|input| {
                if let ort::value::ValueType::Tensor { ref shape, .. } = *input.dtype() {
                    let dims: &[i64] = shape;
                    if dims.len() == 4 && dims[1] > 0 && dims[3] > 0 {
                        Some((dims[1] as usize, dims[3] as usize))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                tracing::warn!(
                    "Could not detect KV cache dimensions from model metadata, \
                     assuming base model (num_heads=8, head_dim=52)"
                );
                (8, 52)
            });

        tracing::info!(
            "Moonshine model loaded in {:.2}s (num_heads={}, head_dim={}, encoder + decoder + tokenizer)",
            start.elapsed().as_secs_f32(),
            num_heads,
            head_dim,
        );

        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            tokenizer,
            quantized,
            decoder_input_names,
            decoder_output_names,
            num_heads,
            head_dim,
        })
    }

    /// Run inference: encode audio then decode tokens autoregressively
    fn run_inference(&self, samples: &[f32]) -> Result<Vec<u32>, TranscribeError> {
        let audio_len = samples.len();
        let duration_secs = audio_len as f32 / 16000.0;

        // --- Encoder ---
        let encoder_start = std::time::Instant::now();

        // Build encoder input: shape [1, audio_length]
        let input_tensor = Tensor::<f32>::from_array(([1usize, audio_len], samples.to_vec()))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create encoder input tensor: {}",
                    e
                ))
            })?;

        let mut encoder = self.encoder.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock encoder: {}", e))
        })?;

        // Cache encoder output name before running (to avoid borrow conflicts)
        let encoder_output_name = encoder
            .outputs()
            .first()
            .map(|o| o.name().to_string())
            .unwrap_or_else(|| "last_hidden_state".to_string());

        let mut encoder_outputs = encoder.run(ort::inputs![input_tensor]).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Encoder inference failed: {}", e))
        })?;

        tracing::debug!(
            "Encoder completed in {:.2}s",
            encoder_start.elapsed().as_secs_f32()
        );

        // Get encoder hidden states - take ownership from session outputs
        // We need this value to persist across decoder steps
        let encoder_hidden = encoder_outputs
            .remove(&encoder_output_name)
            .ok_or_else(|| {
                TranscribeError::InferenceFailed("Encoder produced no output".to_string())
            })?;

        // Drop encoder outputs and release encoder lock early
        drop(encoder_outputs);
        drop(encoder);

        // --- Decoder (autoregressive loop) ---
        let decoder_start = std::time::Instant::now();
        let max_tokens =
            ((duration_secs * MAX_TOKENS_PER_SECOND) as usize).clamp(16, ABSOLUTE_MAX_TOKENS);

        let mut generated_tokens: Vec<i64> = vec![DECODER_START_TOKEN_ID];

        let mut decoder = self.decoder.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock decoder: {}", e))
        })?;

        // Identify KV cache input/output names (sorted to maintain consistent ordering)
        let mut kv_input_names: Vec<&str> = self
            .decoder_input_names
            .iter()
            .filter(|n| n.starts_with("past_key_values"))
            .map(|n| n.as_str())
            .collect();
        kv_input_names.sort();

        let mut kv_output_names: Vec<&str> = self
            .decoder_output_names
            .iter()
            .filter(|n| n.starts_with("present"))
            .map(|n| n.as_str())
            .collect();
        kv_output_names.sort();

        tracing::debug!(
            "KV cache: {} input slots, {} output slots",
            kv_input_names.len(),
            kv_output_names.len()
        );

        // KV cache shape: [batch=1, num_heads, seq_len, head_dim]
        // Dimensions detected from model metadata during construction
        let num_heads = self.num_heads;
        let head_dim = self.head_dim;

        // Separate encoder and decoder KV cache names for proper handling.
        // The merged decoder model outputs empty encoder KV on step 1+ (batch=0)
        // because encoder cross-attention is only computed on step 0. We must
        // save step 0's encoder KV and reuse it on all subsequent steps.
        let mut decoder_kv_input_names: Vec<&str> = kv_input_names
            .iter()
            .filter(|n| n.contains(".decoder."))
            .copied()
            .collect();
        decoder_kv_input_names.sort();

        let mut encoder_kv_input_names: Vec<&str> = kv_input_names
            .iter()
            .filter(|n| n.contains(".encoder."))
            .copied()
            .collect();
        encoder_kv_input_names.sort();

        let mut decoder_kv_output_names: Vec<&str> = kv_output_names
            .iter()
            .filter(|n| n.contains(".decoder."))
            .copied()
            .collect();
        decoder_kv_output_names.sort();

        let mut encoder_kv_output_names: Vec<&str> = kv_output_names
            .iter()
            .filter(|n| n.contains(".encoder."))
            .copied()
            .collect();
        encoder_kv_output_names.sort();

        // Track decoder KV cache (grows each step) and encoder KV cache (fixed after step 0)
        let mut decoder_kv_cache: Vec<ort::value::DynValue> = Vec::new();
        let mut encoder_kv_cache: Vec<ort::value::DynValue> = Vec::new();

        for step in 0..max_tokens {
            // Build input_ids tensor
            let input_ids = if step == 0 {
                Tensor::<i64>::from_array((
                    [1usize, generated_tokens.len()],
                    generated_tokens.clone(),
                ))
            } else {
                Tensor::<i64>::from_array((
                    [1usize, 1usize],
                    vec![*generated_tokens.last().unwrap()],
                ))
            }
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create input_ids tensor: {}",
                    e
                ))
            })?;

            let mut inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> =
                Vec::new();

            // Add input_ids
            inputs.push((std::borrow::Cow::Borrowed("input_ids"), input_ids.into()));

            // Add encoder_hidden_states
            inputs.push((
                std::borrow::Cow::Borrowed("encoder_hidden_states"),
                ort::session::SessionInputValue::from(&encoder_hidden),
            ));

            // Add decoder KV cache inputs
            if step == 0 {
                // First step: dummy decoder KV tensors (use_cache_branch=false ignores them)
                let dummy_size = num_heads * head_dim;
                for kv_name in &decoder_kv_input_names {
                    let dummy_kv = Tensor::<f32>::from_array((
                        [1usize, num_heads, 1usize, head_dim],
                        vec![0.0f32; dummy_size],
                    ))
                    .map_err(|e| {
                        TranscribeError::InferenceFailed(format!(
                            "Failed to create dummy KV tensor: {}",
                            e
                        ))
                    })?;
                    inputs.push((std::borrow::Cow::Borrowed(kv_name), dummy_kv.into()));
                }
            } else {
                // Subsequent steps: use decoder KV from previous step
                for (i, kv_name) in decoder_kv_input_names.iter().enumerate() {
                    inputs.push((
                        std::borrow::Cow::Borrowed(kv_name),
                        ort::session::SessionInputValue::from(&decoder_kv_cache[i]),
                    ));
                }
            }

            // Add encoder KV cache inputs
            if step == 0 {
                // First step: dummy encoder KV tensors (use_cache_branch=false ignores them)
                let dummy_size = num_heads * head_dim;
                for kv_name in &encoder_kv_input_names {
                    let dummy_kv = Tensor::<f32>::from_array((
                        [1usize, num_heads, 1usize, head_dim],
                        vec![0.0f32; dummy_size],
                    ))
                    .map_err(|e| {
                        TranscribeError::InferenceFailed(format!(
                            "Failed to create dummy KV tensor: {}",
                            e
                        ))
                    })?;
                    inputs.push((std::borrow::Cow::Borrowed(kv_name), dummy_kv.into()));
                }
            } else {
                // Subsequent steps: reuse step 0's encoder KV (it never changes)
                for (i, kv_name) in encoder_kv_input_names.iter().enumerate() {
                    inputs.push((
                        std::borrow::Cow::Borrowed(kv_name),
                        ort::session::SessionInputValue::from(&encoder_kv_cache[i]),
                    ));
                }
            }

            // Add use_cache_branch flag
            let use_cache = Tensor::<bool>::from_array(([1], vec![step > 0])).map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create use_cache tensor: {}",
                    e
                ))
            })?;
            inputs.push((
                std::borrow::Cow::Borrowed("use_cache_branch"),
                use_cache.into(),
            ));

            // Run decoder step
            let mut outputs = decoder.run(inputs).map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Decoder inference failed at step {}: {}",
                    step, e
                ))
            })?;

            // Extract logits - shape is [1, seq_len, 32768]
            let logits_val = &outputs["logits"];

            let (shape, logits_data) = logits_val.try_extract_tensor::<f32>().map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to extract logits: {}", e))
            })?;

            let shape_dims: &[i64] = shape;

            // Get last position logits
            let vocab_logits: &[f32] = if shape_dims.len() == 3 {
                let vocab_size = shape_dims[2] as usize;
                let seq_len = shape_dims[1] as usize;
                let offset = (seq_len - 1) * vocab_size;
                &logits_data[offset..offset + vocab_size]
            } else if shape_dims.len() == 2 {
                let vocab_size = shape_dims[1] as usize;
                &logits_data[..vocab_size]
            } else {
                return Err(TranscribeError::InferenceFailed(format!(
                    "Unexpected logits shape: {:?}",
                    shape_dims
                )));
            };

            // Greedy decode: argmax
            let next_token = vocab_logits
                .iter()
                .enumerate()
                .max_by(|(_, a): &(usize, &f32), (_, b): &(usize, &f32)| {
                    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(idx, _)| idx as i64)
                .ok_or_else(|| {
                    TranscribeError::InferenceFailed("Empty logits vector".to_string())
                })?;

            // Check for EOS
            if next_token == EOS_TOKEN_ID {
                tracing::debug!("Decoder reached EOS at step {}", step);
                break;
            }

            generated_tokens.push(next_token);

            // Collect decoder KV cache for next step
            let mut new_decoder_cache = Vec::new();
            for kv_out_name in &decoder_kv_output_names {
                if let Some(value) = outputs.remove(kv_out_name) {
                    new_decoder_cache.push(value);
                }
            }
            decoder_kv_cache = new_decoder_cache;

            // On step 0, save encoder KV cache (reused on all subsequent steps)
            if step == 0 {
                for kv_out_name in &encoder_kv_output_names {
                    if let Some(value) = outputs.remove(kv_out_name) {
                        encoder_kv_cache.push(value);
                    }
                }
            }
        }

        tracing::debug!(
            "Decoder completed in {:.2}s ({} tokens)",
            decoder_start.elapsed().as_secs_f32(),
            generated_tokens.len() - 1
        );

        // Convert i64 tokens to u32 for tokenizer, skip the start token
        let token_ids: Vec<u32> = generated_tokens.iter().skip(1).map(|&t| t as u32).collect();

        Ok(token_ids)
    }
}

impl Transcriber for MoonshineTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / 16000.0;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with Moonshine (quantized={})",
            duration_secs,
            samples.len(),
            self.quantized
        );

        let start = std::time::Instant::now();

        // Run encoder + decoder
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
            "Moonshine transcription completed in {:.2}s: {:?}",
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
fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    // If it's already an absolute path, use it directly
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    // Map short names to directory names
    // Handles: "tiny", "base", "base-ja", "tiny-ko", etc.
    let model_dir_name = if model.starts_with("moonshine-") {
        model.to_string()
    } else {
        format!("moonshine-{}", model)
    };

    // Check models directory
    let models_dir = crate::config::Config::models_dir();
    let model_path = models_dir.join(&model_dir_name);

    if model_path.exists() {
        return Ok(model_path);
    }

    // Check current directory
    let cwd_path = PathBuf::from(&model_dir_name);
    if cwd_path.exists() {
        return Ok(cwd_path);
    }

    // Check ./models/
    let local_models_path = PathBuf::from("models").join(&model_dir_name);
    if local_models_path.exists() {
        return Ok(local_models_path);
    }

    Err(TranscribeError::ModelNotFound(format!(
        "Moonshine model '{}' not found. Looked in:\n  \
         - {}\n  \
         - {}\n  \
         - {}\n\n\
         Run 'voxtype setup model' to download, or manually from:\n  \
         tiny: https://huggingface.co/onnx-community/moonshine-tiny-ONNX\n  \
         base: https://huggingface.co/onnx-community/moonshine-base-ONNX",
        model,
        model_path.display(),
        cwd_path.display(),
        local_models_path.display()
    )))
}

/// Detect whether a model directory contains quantized models
#[allow(dead_code)]
fn has_quantized_models(path: &Path) -> bool {
    path.join("encoder_model_quantized.onnx").exists()
        && path.join("decoder_model_merged_quantized.onnx").exists()
}

/// Detect whether a model directory contains full-precision models
#[allow(dead_code)]
fn has_full_models(path: &Path) -> bool {
    path.join("encoder_model.onnx").exists() && path.join("decoder_model_merged.onnx").exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_resolve_model_path_short_names() {
        let result = resolve_model_path("nonexistent-model");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent-model"));
    }

    #[test]
    fn test_resolve_model_path_absolute() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Create expected model files so the path exists
        fs::write(model_path.join("encoder_model.onnx"), b"dummy").unwrap();

        let resolved = resolve_model_path(model_path.to_str().unwrap());
        assert!(resolved.is_ok());
        assert_eq!(resolved.unwrap(), model_path);
    }

    #[test]
    fn test_has_quantized_models() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        assert!(!has_quantized_models(path));

        fs::write(path.join("encoder_model_quantized.onnx"), b"dummy").unwrap();
        fs::write(path.join("decoder_model_merged_quantized.onnx"), b"dummy").unwrap();

        assert!(has_quantized_models(path));
    }

    #[test]
    fn test_has_full_models() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        assert!(!has_full_models(path));

        fs::write(path.join("encoder_model.onnx"), b"dummy").unwrap();
        fs::write(path.join("decoder_model_merged.onnx"), b"dummy").unwrap();

        assert!(has_full_models(path));
    }

    #[test]
    fn test_resolve_model_path_not_found() {
        let result = resolve_model_path("/nonexistent/path/to/model");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TranscribeError::ModelNotFound(_)));
    }
}
