//! Cohere Transcribe via the HuggingFace Optimum-exported ONNX format.
//!
//! Replaces the cstr-shaped implementation that was the proof-of-concept in
//! 0.7.0-rc1. The HF Optimum export is the upstream-canonical shape (used
//! by `onnx-community/cohere-transcribe-03-2026-ONNX`) and ships in four
//! precisions (FP16, int8, q4, q4f16). q4 is the one that compiles cleanly
//! through MIGraphX 7.2 and unblocks AMD GPU acceleration for Cohere.
//!
//! ## Architecture
//!
//! Two ONNX sessions:
//!
//! ### Encoder (`encoder_model.onnx`)
//! ```text
//! input_features  : F32 [batch, frames, 128]   # 128-bin log-mel + per-feature CMVN
//! ↓
//! last_hidden_state : F32 [batch, T_enc, 1024] # encoder output, T_enc downsampled
//! ```
//!
//! ### Decoder (`decoder_model_merged.onnx`)
//! ```text
//! input_ids                            : I64 [batch, seq]
//! attention_mask                       : I64 [batch, total_seq]   # cumulative
//! position_ids                         : I64 [batch, seq]
//! num_logits_to_keep                   : I64 []                   # scalar, always 1
//! encoder_hidden_states                : F32 [batch, T_enc, 1024]
//! past_key_values.{0..7}.decoder.{key,value} : F32 [batch, 8, past_dec, 128]
//! past_key_values.{0..7}.encoder.{key,value} : F32 [batch, 8, past_enc, 128]
//! ↓
//! logits                               : F32 [batch, num_logits_to_keep, 16384]
//! present.{0..7}.decoder.{key,value}   : F32 [batch, 8, total_dec, 128]
//! present.{0..7}.encoder.{key,value}   : F32 [batch, 8, T_enc, 128]
//! ```
//!
//! The decoder is a "merged" model: same graph handles both the prefix-fill
//! pass (empty past, multi-token input_ids) and incremental generation
//! (full past, single-token input_ids). The encoder's K/V projections are
//! computed lazily on the first decoder call (when `past.encoder` is empty)
//! and reused for every subsequent call via `past_key_values.N.encoder.*`,
//! so we don't re-project the encoder output every step.
//!
//! ## Decoder prefix
//!
//! Cohere uses a Whisper-style multi-token prompt to set language and task.
//! For English transcription with punctuation, ITN, no timestamps, no
//! diarization, the prefix is:
//!
//! ```text
//! [<|startoftranscript|>=4, <|en|>=62, <|pnc|>=5, <|itn|>=8,
//!  <|notimestamp|>=11, <|nodiarize|>=13]
//! ```

use crate::config::CohereConfig;
use crate::error::TranscribeError;
use crate::transcribe::Transcriber;
use crate::transcribe::cohere_fbank::CohereFbank;
use ort::session::Session;
use ort::value::{DynTensor, DynValue, Tensor, TensorElementType, ValueType};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokenizers::Tokenizer;

// ---------------------------------------------------------------------------
// Architecture constants
// ---------------------------------------------------------------------------

const N_LAYERS: usize = 8;
const N_HEADS: usize = 8;
const HEAD_DIM: usize = 128;
const D_MODEL: usize = N_HEADS * HEAD_DIM; // 1024
const N_MELS: usize = 128;
const VOCAB_SIZE: usize = 16384;
const SAMPLE_RATE: usize = 16_000;

const TOK_EOS: i64 = 3;
const TOK_SOT: i64 = 4;
const TOK_PNC: i64 = 5;
const TOK_ITN: i64 = 8;
const TOK_NOTIMESTAMP: i64 = 11;
const TOK_NODIARIZE: i64 = 13;

fn lang_token(lang: &str) -> Option<i64> {
    Some(match lang {
        "ar" => 28,
        "de" => 76,
        "el" => 77,
        "en" => 62,
        "es" => 169,
        "fr" => 69,
        "it" => 97,
        "ja" => 98,
        "ko" => 110,
        "nl" => 60,
        "pl" => 148,
        "pt" => 149,
        "vi" => 194,
        "zh" => 50,
        _ => return None,
    })
}

const SUPPORTED_LANGUAGES: &[&str] = &[
    "ar", "de", "el", "en", "es", "fr", "it", "ja", "ko", "nl", "pl", "pt", "vi", "zh",
];

const MAX_TOKENS_PER_SECOND: f32 = 8.0;
const ABSOLUTE_MAX_TOKENS: usize = 1024;

// ---------------------------------------------------------------------------
// Transcriber
// ---------------------------------------------------------------------------

/// Per-step bundle of outputs the decoder loop pulls out of `SessionOutputs`
/// before the lock guard goes out of scope. Lets us release the decoder
/// mutex between steps.
struct StepOutputs {
    best_token: i64,
    present_dec: Vec<DynValue>,
    present_enc: Vec<DynValue>,
}

pub struct CohereTranscriber {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    tokenizer: Tokenizer,
    fbank: CohereFbank,
    prefix: Vec<i64>,
    /// Float dtype the encoder emits and the decoder expects on
    /// `encoder_hidden_states` and the `past_key_values.*` cache tensors.
    /// Float32 for the q4 / int8 / FP32 variants, Float16 for fp16 / q4f16.
    /// We discover this once at load time from the encoder's
    /// `last_hidden_state` output type so the decoder loop can build
    /// matching empty caches and we can extract the encoder output
    /// against the right primitive.
    float_dtype: TensorElementType,
}

impl CohereTranscriber {
    pub fn new(config: &CohereConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;
        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));
        Self::with_threads_and_lang(&model_dir, threads, &config.language)
    }

    pub fn from_dir(model_dir: &Path) -> Result<Self, TranscribeError> {
        Self::with_threads_and_lang(model_dir, num_cpus::get().min(4), "en")
    }

    pub fn with_threads(model_dir: &Path, threads: usize) -> Result<Self, TranscribeError> {
        Self::with_threads_and_lang(model_dir, threads, "en")
    }

    pub fn with_threads_and_lang(
        model_dir: &Path,
        threads: usize,
        language: &str,
    ) -> Result<Self, TranscribeError> {
        tracing::info!("Loading Cohere Transcribe model from {:?}", model_dir);
        let start = std::time::Instant::now();

        let encoder_file = model_dir.join("encoder_model.onnx");
        let decoder_file = model_dir.join("decoder_model_merged.onnx");
        let tokenizer_file = model_dir.join("tokenizer.json");

        for (label, path) in [
            ("encoder", &encoder_file),
            ("decoder", &decoder_file),
            ("tokenizer.json", &tokenizer_file),
        ] {
            if !path.exists() {
                return Err(TranscribeError::ModelNotFound(format!(
                    "Cohere {label} not found: {}\n  \
                     Download from https://huggingface.co/onnx-community/\
                     cohere-transcribe-03-2026-ONNX",
                    path.display(),
                )));
            }
        }

        let tokenizer = Tokenizer::from_file(&tokenizer_file).map_err(|e| {
            TranscribeError::InitFailed(format!(
                "Failed to load Cohere tokenizer from {}: {e}",
                tokenizer_file.display()
            ))
        })?;

        let encoder = build_session(&encoder_file, threads, "encoder", true)?;
        // Decoder pinned to CPU: ORT's CUDA GroupQueryAttention kernel rejects
        // the `attention_bias` input that the HF Optimum decoder export uses
        // (validated on GTX 1660 Ti, ORT 1.20 via pyke ort 2.0.0-rc.12). The
        // encoder still runs on GPU, where the 1.4GB-weight matmuls dominate
        // wall time; the smaller decoder runs CPU-side until ORT lands the
        // attention_bias kernel.
        let decoder = build_session(&decoder_file, threads, "decoder", false)?;

        // The HF Optimum exports use mixed precision: `encoder_hidden_states`
        // stays Float32 across every variant (q4/int8/FP32 keep encoder
        // outputs in FP32; the FP16-flavored variants narrow inside the
        // decoder), but the KV caches and `logits` follow the variant —
        // Float32 for q4/int8/FP32, Float16 for fp16/q4f16. Detect the KV
        // dtype from the decoder's `past_key_values.0.decoder.key` input
        // so we build matching empty caches and read logits at the right
        // primitive size.
        let float_dtype = decoder
            .inputs()
            .iter()
            .find(|i| i.name() == "past_key_values.0.decoder.key")
            .and_then(|i| match i.dtype() {
                ValueType::Tensor { ty, .. } => Some(*ty),
                _ => None,
            })
            .unwrap_or(TensorElementType::Float32);
        if float_dtype != TensorElementType::Float32
            && float_dtype != TensorElementType::Float16
        {
            return Err(TranscribeError::InitFailed(format!(
                "Cohere decoder past_key_values dtype {:?} is neither Float32 nor Float16",
                float_dtype,
            )));
        }

        let lang_id = lang_token(language).ok_or_else(|| {
            TranscribeError::InitFailed(format!(
                "Unsupported Cohere language '{}'. Supported: {}",
                language,
                SUPPORTED_LANGUAGES.join(", "),
            ))
        })?;

        let prefix = vec![
            TOK_SOT,
            lang_id,
            TOK_PNC,
            TOK_ITN,
            TOK_NOTIMESTAMP,
            TOK_NODIARIZE,
        ];

        tracing::info!(
            "Cohere model loaded in {:.2}s (vocab={}, language='{}', prefix={:?})",
            start.elapsed().as_secs_f32(),
            VOCAB_SIZE,
            language,
            prefix,
        );

        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            tokenizer,
            fbank: CohereFbank::new(),
            prefix,
            float_dtype,
        })
    }

    fn transcribe_samples(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        // 1. Feature extraction.
        let features_2d = self.fbank.extract(samples);
        let n_frames = features_2d.nrows();
        if n_frames == 0 {
            return Ok(String::new());
        }
        let features_flat: Vec<f32> = features_2d.iter().copied().collect();
        let enc_input = Tensor::<f32>::from_array(([1usize, n_frames, N_MELS], features_flat))
            .map_err(|e| TranscribeError::InferenceFailed(format!("encoder input: {e}")))?;

        // 2. Encoder forward. We hold on to the encoder output as a
        // DynValue (rather than extracting it into a Vec<f32>) so the same
        // tensor flows back into the decoder by reference, and we don't
        // have to know whether it's Float32 or Float16 to copy it. The
        // decoder loop just borrows it by name each step.
        let encoder_hidden: DynValue = {
            let mut enc = self
                .encoder
                .lock()
                .map_err(|e| TranscribeError::InferenceFailed(format!("encoder lock: {e}")))?;
            let mut enc_outputs = enc
                .run(ort::inputs!["input_features" => enc_input])
                .map_err(|e| TranscribeError::InferenceFailed(format!("encoder run: {e}")))?;
            enc_outputs.remove("last_hidden_state").ok_or_else(|| {
                TranscribeError::InferenceFailed("encoder missing last_hidden_state".to_string())
            })?
        };

        // 3. Decoder generation loop.
        let audio_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        let max_new = ((audio_secs * MAX_TOKENS_PER_SECOND) as usize)
            .max(self.prefix.len() + 16)
            .min(ABSOLUTE_MAX_TOKENS);

        let mut generated: Vec<i64> = Vec::with_capacity(max_new);
        let mut tokens_so_far: Vec<i64> = self.prefix.clone();

        // KV caches grow each step (decoder) or stay constant after step 0
        // (encoder). On step 0 we pass empty caches with shape [1, 8, 0, 128]
        // in whatever float dtype the model uses (Float32 for q4/int8/FP32
        // exports, Float16 for fp16/q4f16).
        let mut past_dec: Vec<DynValue> = (0..N_LAYERS * 2)
            .map(|_| empty_kv(self.float_dtype))
            .collect::<Result<Vec<_>, _>>()?;
        let mut past_enc: Vec<DynValue> = (0..N_LAYERS * 2)
            .map(|_| empty_kv(self.float_dtype))
            .collect::<Result<Vec<_>, _>>()?;

        for step in 0..max_new {
            let (input_ids_vec, position_ids_vec, attention_mask_vec): (
                Vec<i64>,
                Vec<i64>,
                Vec<i64>,
            ) = if step == 0 {
                let pos: Vec<i64> = (0..self.prefix.len() as i64).collect();
                let mask: Vec<i64> = vec![1; self.prefix.len()];
                (self.prefix.clone(), pos, mask)
            } else {
                let last = *tokens_so_far.last().unwrap();
                let pos = vec![tokens_so_far.len() as i64 - 1];
                let mask: Vec<i64> = vec![1; tokens_so_far.len()];
                (vec![last], pos, mask)
            };

            let seq_len = input_ids_vec.len();
            let total_len = if step == 0 {
                self.prefix.len()
            } else {
                tokens_so_far.len()
            };

            let input_ids = Tensor::<i64>::from_array(([1usize, seq_len], input_ids_vec))
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!("input_ids tensor: {e}"))
                })?;
            let attention_mask = Tensor::<i64>::from_array(([1usize, total_len], attention_mask_vec))
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!("attention_mask tensor: {e}"))
                })?;
            let position_ids = Tensor::<i64>::from_array(([1usize, seq_len], position_ids_vec))
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!("position_ids tensor: {e}"))
                })?;
            // Scalar (rank-0) tensor — the inspector showed
            // `num_logits_to_keep : Tensor<Int64>[]`. A 1-D `[1]` shape
            // breaks the lm_head Slice op which uses this as a Starts arg.
            let num_logits = Tensor::<i64>::from_array(([] as [usize; 0], vec![1_i64]))
                .map_err(|e| TranscribeError::InferenceFailed(format!("num_logits tensor: {e}")))?;

            let mut inputs: Vec<(Cow<str>, ort::session::SessionInputValue)> = Vec::new();
            inputs.push((Cow::Borrowed("input_ids"), input_ids.into()));
            inputs.push((Cow::Borrowed("attention_mask"), attention_mask.into()));
            inputs.push((Cow::Borrowed("position_ids"), position_ids.into()));
            inputs.push((Cow::Borrowed("num_logits_to_keep"), num_logits.into()));
            inputs.push((
                Cow::Borrowed("encoder_hidden_states"),
                ort::session::SessionInputValue::from(&encoder_hidden),
            ));

            for layer in 0..N_LAYERS {
                let dk_name = format!("past_key_values.{layer}.decoder.key");
                let dv_name = format!("past_key_values.{layer}.decoder.value");
                let ek_name = format!("past_key_values.{layer}.encoder.key");
                let ev_name = format!("past_key_values.{layer}.encoder.value");
                inputs.push((
                    Cow::Owned(dk_name),
                    ort::session::SessionInputValue::from(&past_dec[layer * 2]),
                ));
                inputs.push((
                    Cow::Owned(dv_name),
                    ort::session::SessionInputValue::from(&past_dec[layer * 2 + 1]),
                ));
                inputs.push((
                    Cow::Owned(ek_name),
                    ort::session::SessionInputValue::from(&past_enc[layer * 2]),
                ));
                inputs.push((
                    Cow::Owned(ev_name),
                    ort::session::SessionInputValue::from(&past_enc[layer * 2 + 1]),
                ));
            }

            // Run the decoder, extract everything we need, then drop the
            // lock guard before starting the next iteration.
            let StepOutputs {
                best_token,
                present_dec,
                present_enc,
            } = {
                let mut dec = self.decoder.lock().map_err(|e| {
                    TranscribeError::InferenceFailed(format!("decoder lock: {e}"))
                })?;
                let mut outputs = dec.run(inputs).map_err(|e| {
                    TranscribeError::InferenceFailed(format!("decoder step {step}: {e}"))
                })?;

                let best = argmax_logits(&outputs["logits"], self.float_dtype)?;

                let mut present_dec: Vec<DynValue> = Vec::with_capacity(N_LAYERS * 2);
                let mut present_enc: Vec<DynValue> = Vec::with_capacity(N_LAYERS * 2);
                for layer in 0..N_LAYERS {
                    let dk = outputs
                        .remove(&format!("present.{layer}.decoder.key"))
                        .ok_or_else(|| {
                            TranscribeError::InferenceFailed(format!(
                                "missing present.{layer}.decoder.key"
                            ))
                        })?;
                    let dv = outputs
                        .remove(&format!("present.{layer}.decoder.value"))
                        .ok_or_else(|| {
                            TranscribeError::InferenceFailed(format!(
                                "missing present.{layer}.decoder.value"
                            ))
                        })?;
                    let ek = outputs
                        .remove(&format!("present.{layer}.encoder.key"))
                        .ok_or_else(|| {
                            TranscribeError::InferenceFailed(format!(
                                "missing present.{layer}.encoder.key"
                            ))
                        })?;
                    let ev = outputs
                        .remove(&format!("present.{layer}.encoder.value"))
                        .ok_or_else(|| {
                            TranscribeError::InferenceFailed(format!(
                                "missing present.{layer}.encoder.value"
                            ))
                        })?;
                    present_dec.push(dk);
                    present_dec.push(dv);
                    present_enc.push(ek);
                    present_enc.push(ev);
                }

                StepOutputs {
                    best_token: best,
                    present_dec,
                    present_enc,
                }
            };

            if best_token == TOK_EOS {
                break;
            }
            generated.push(best_token);
            tokens_so_far.push(best_token);
            past_dec = present_dec;
            past_enc = present_enc;
        }

        let ids_u32: Vec<u32> = generated.iter().map(|&t| t as u32).collect();
        let text = self
            .tokenizer
            .decode(&ids_u32, true)
            .map_err(|e| TranscribeError::InferenceFailed(format!("tokenizer decode: {e}")))?;
        Ok(text.trim().to_string())
    }
}

/// Empty `[1, 8, 0, 128]` KV tensor in `dtype` (Float32 or Float16) for
/// the first decoder call. ort's raw-data constructors reject zero-sized
/// dimensions, so we go through `DynTensor::new(allocator, dtype, shape)`
/// which performs an empty allocation in the requested precision.
fn empty_kv(dtype: TensorElementType) -> Result<DynValue, TranscribeError> {
    let allocator = ort::memory::Allocator::default();
    let t = DynTensor::new(&allocator, dtype, [1usize, N_HEADS, 0, HEAD_DIM])
        .map_err(|e| TranscribeError::InferenceFailed(format!("empty kv: {e}")))?;
    Ok(t.into_dyn())
}

/// Argmax over the last logit row, working in either Float32 or Float16
/// per the model variant. f16 values are widened to f32 for the
/// comparison so we don't have to depend on `half`'s `Ord` impl across
/// versions.
fn argmax_logits(
    logits: &DynValue,
    dtype: TensorElementType,
) -> Result<i64, TranscribeError> {
    fn pick<F: Copy + PartialOrd, I: Iterator<Item = F>>(it: I) -> i64 {
        let mut best_idx = 0_i64;
        let mut best_val: Option<F> = None;
        for (i, v) in it.enumerate() {
            match best_val {
                None => {
                    best_val = Some(v);
                    best_idx = i as i64;
                }
                Some(b) if v > b => {
                    best_val = Some(v);
                    best_idx = i as i64;
                }
                _ => {}
            }
        }
        best_idx
    }

    match dtype {
        TensorElementType::Float32 => {
            let (shape, data) = logits
                .try_extract_tensor::<f32>()
                .map_err(|e| TranscribeError::InferenceFailed(format!("logits f32: {e}")))?;
            check_logits_shape(shape)?;
            Ok(pick(data.iter().copied()))
        }
        TensorElementType::Float16 => {
            let (shape, data) = logits
                .try_extract_tensor::<half::f16>()
                .map_err(|e| TranscribeError::InferenceFailed(format!("logits f16: {e}")))?;
            check_logits_shape(shape)?;
            // Widen to f32 for the comparison; the relative ordering is
            // the same and we already pay one allocation worth of work.
            Ok(pick(data.iter().map(|h| h.to_f32())))
        }
        other => Err(TranscribeError::InferenceFailed(format!(
            "unsupported logits dtype {:?}",
            other
        ))),
    }
}

fn check_logits_shape(shape: &[i64]) -> Result<(), TranscribeError> {
    if shape.len() != 3 || shape[2] as usize != VOCAB_SIZE {
        return Err(TranscribeError::InferenceFailed(format!(
            "logits shape {:?} != [1, 1, {VOCAB_SIZE}]",
            shape
        )));
    }
    Ok(())
}

impl Transcriber for CohereTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        let start = std::time::Instant::now();
        let text = self.transcribe_samples(samples)?;
        tracing::info!(
            "Cohere transcription completed in {:.2}s: {:?}",
            start.elapsed().as_secs_f32(),
            text.chars().take(80).collect::<String>(),
        );
        Ok(text)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_model_path(name: &str) -> Result<PathBuf, TranscribeError> {
    if let Ok(env) = std::env::var("VOXTYPE_COHERE_MODEL_DIR") {
        return Ok(PathBuf::from(env));
    }
    let dirs = directories::ProjectDirs::from("io", "voxtype", "voxtype").ok_or_else(|| {
        TranscribeError::ModelNotFound(
            "Could not resolve model directory (no ProjectDirs)".to_string(),
        )
    })?;
    Ok(dirs.data_dir().join("models").join(name))
}

fn build_session(
    path: &Path,
    threads: usize,
    label: &str,
    use_gpu: bool,
) -> Result<Session, TranscribeError> {
    let builder = Session::builder()
        .map_err(|e| TranscribeError::InitFailed(format!("{label} builder: {e}")))?
        .with_intra_threads(threads)
        .map_err(|e| TranscribeError::InitFailed(format!("{label} threads: {e}")))?;

    let mut builder = if use_gpu {
        super::onnx_ep::register_gpu_eps(builder, "Cohere", label)
            .map_err(|e| TranscribeError::InitFailed(format!("{label} EPs: {e}")))?
    } else {
        builder
    };

    builder.commit_from_file(path).map_err(|e| {
        TranscribeError::InitFailed(format!(
            "Failed to load Cohere {label} from {:?}: {e}",
            path
        ))
    })
}
