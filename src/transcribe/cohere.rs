//! Cohere Transcribe speech-to-text (feature-gated)
//!
//! Uses Cohere Labs' Cohere Transcribe model via ONNX Runtime. Wired into
//! the engine factory, CLI, and `[cohere]` config section. Compile in via
//! `cargo build --features cohere` (CPU) or `--features cohere-cuda`
//! / `--features cohere-tensorrt` for GPU acceleration.
//!
//! ## Model architecture (verified against `cstr/cohere-transcribe-onnx-int8`)
//!
//! Encoder (`cohere-encoder.int8.onnx`):
//! ```text
//! inputs:
//!   audio            : F32 [1, n_samples]    # raw 16 kHz PCM
//! outputs:
//!   n_layer_cross_k  : F32 [8, 1, T_enc, 1024]   # precomputed cross-attn K
//!   n_layer_cross_v  : F32 [8, 1, T_enc, 1024]   # precomputed cross-attn V
//! ```
//!
//! `T_enc = (n_samples / 1280) + 1`. The encoder bakes log-mel preprocessing
//! and the cross-attention projection into a single graph: feed raw PCM,
//! get cross-K/V tensors back ready to plug into the decoder.
//!
//! Decoder (`cohere-decoder.int8.onnx`):
//! ```text
//! inputs:
//!   tokens                   : I64 [1, n_tokens]
//!   in_n_layer_self_k_cache  : F32 [8, 1, 8, 1024, 128]   # rolling self-K cache
//!   in_n_layer_self_v_cache  : F32 [8, 1, 8, 1024, 128]
//!   n_layer_cross_k          : F32 [8, 1, T_enc, 1024]    # from encoder
//!   n_layer_cross_v          : F32 [8, 1, T_enc, 1024]
//!   offset                   : I64 []                     # write position
//! outputs:
//!   logits                   : F32 [1, n_tokens, 16384]
//!   out_n_layer_self_k_cache : F32 [8, 1, 8, 1024, 128]
//!   out_n_layer_self_v_cache : F32 [8, 1, 8, 1024, 128]
//! ```
//!
//! Architecture constants (all fixed for this export):
//! - 8 layers, 8 heads, head dim 128, d_model 1024
//! - Self-attention rolling cache: 1024 token capacity
//! - Vocab: 16384 (matches `tokens.txt` line count)
//!
//! Cross-attention K/V are computed once by the encoder per utterance and
//! reused at every decoder step. The self-attention cache is a fixed-size
//! ring with the `offset` scalar tracking where the next K/V slice goes.
//!
//! ## Decoder prefix
//!
//! Cohere Transcribe uses a Whisper-style multi-token decoder prefix. For
//! English transcription with punctuation/capitalization on, no timestamps,
//! no diarization, the prefix is:
//!
//! ```text
//! [<|startoftranscript|>=4, <|en|>=62, <|pnc|>=5, <|itn|>=8,
//!  <|notimestamp|>=11, <|nodiarize|>=13]
//! ```
//!
//! Generation continues until `<|endoftext|>=3`.
//!
//! ## Downloading the int8 model for the PoC test
//!
//! The original `CohereLabs/cohere-transcribe-03-2026` weights are gated on
//! HuggingFace (Apache 2.0 licensed but require accepting the model card).
//! The community ONNX export at `cstr/cohere-transcribe-onnx-int8` is not gated:
//!
//! ```bash
//! mkdir -p ~/.cache/voxtype-models/cohere-transcribe-int8
//! cd ~/.cache/voxtype-models/cohere-transcribe-int8
//! BASE=https://huggingface.co/cstr/cohere-transcribe-onnx-int8/resolve/main
//! for f in cohere-encoder.int8.onnx cohere-encoder.int8.onnx.data \
//!          cohere-decoder.int8.onnx cohere-decoder.int8.onnx.data \
//!          tokens.txt; do
//!     curl -L "$BASE/$f" -o "$f"
//! done
//! ```
//!
//! ## Running the integration test
//!
//! ```bash
//! VOXTYPE_COHERE_MODEL_DIR=~/.cache/voxtype-models/cohere-transcribe-int8 \
//!     cargo test --features cohere transcribe::cohere::tests::cohere_poc \
//!                -- --ignored --nocapture
//! ```
//!
//! ## Configuration
//!
//! ```toml
//! engine = "cohere"
//!
//! [cohere]
//! model = "cohere-transcribe-int8"   # subdir in voxtype's models dir
//! language = "en"                     # one of the 14 supported langs
//! threads = 4                         # optional; defaults to num_cpus.min(4)
//! on_demand_loading = false
//! ```

use super::Transcriber;
use crate::config::CohereConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Architecture constants (fixed for the cstr/cohere-transcribe-onnx-int8 export)
// ---------------------------------------------------------------------------

const N_LAYERS: usize = 8;
const N_HEADS: usize = 8;
const HEAD_DIM: usize = 128;
const D_MODEL: usize = N_HEADS * HEAD_DIM; // 1024
const SELF_KV_CACHE_LEN: usize = 1024;
const VOCAB_SIZE: usize = 16384;
const SAMPLE_RATE: usize = 16_000;

/// `<|endoftext|>` token ID. Verified against the cstr export's tokens.txt
/// in `build_prefix_against_real_tokens_txt`. The decoder prefix and other
/// task tokens are looked up by name at runtime in `build_prefix`, so they
/// don't need const declarations here.
const TOK_EOS: i64 = 3;

/// Cohere Transcribe officially supports 14 languages. The language tokens
/// live in `tokens.txt` as `<|<iso>|>` entries; we look them up by name at
/// `new()` time so future model versions that change the IDs still work.
const SUPPORTED_LANGUAGES: &[&str] = &[
    "ar", "de", "en", "es", "fr", "hi", "it", "ja", "ko", "nl", "pt", "ru", "tr", "zh",
];

/// Generation safety limits.
const MAX_TOKENS_PER_SECOND: f32 = 8.0;
const ABSOLUTE_MAX_TOKENS: usize = 1024;

// ---------------------------------------------------------------------------
// Transcriber
// ---------------------------------------------------------------------------

/// Cohere Transcribe transcriber using ONNX Runtime.
pub struct CohereTranscriber {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    /// SentencePiece tokens loaded from `tokens.txt` (id -> piece string).
    tokens: HashMap<u32, String>,
    /// Resolved decoder prefix (`<|sot|>` + language + task tokens). Fed in
    /// the first decoder call to populate the self-attention KV cache.
    prefix: Vec<i64>,
}

impl CohereTranscriber {
    /// Construct from `[cohere]` config: resolves model name → path, applies
    /// thread count, builds the language-specific decoder prefix.
    pub fn new(config: &CohereConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;
        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));
        Self::with_threads_and_lang(&model_dir, threads, &config.language)
    }

    /// Load the Cohere encoder + decoder + tokens from a model directory.
    ///
    /// Expects the cstr/cohere-transcribe-onnx-int8 layout:
    /// - `cohere-encoder.int8.onnx` (+ `.onnx.data` sidecar)
    /// - `cohere-decoder.int8.onnx` (+ `.onnx.data` sidecar)
    /// - `tokens.txt`
    pub fn from_dir(model_dir: &Path) -> Result<Self, TranscribeError> {
        Self::with_threads_and_lang(model_dir, num_cpus::get().min(4), "en")
    }

    /// Load with an explicit thread count for ONNX intra-op parallelism.
    pub fn with_threads(model_dir: &Path, threads: usize) -> Result<Self, TranscribeError> {
        Self::with_threads_and_lang(model_dir, threads, "en")
    }

    /// Full constructor with thread count and language code.
    pub fn with_threads_and_lang(
        model_dir: &Path,
        threads: usize,
        language: &str,
    ) -> Result<Self, TranscribeError> {
        tracing::info!("Loading Cohere Transcribe model from {:?}", model_dir);
        let start = std::time::Instant::now();

        let encoder_file = model_dir.join("cohere-encoder.int8.onnx");
        let decoder_file = model_dir.join("cohere-decoder.int8.onnx");
        let tokens_file = model_dir.join("tokens.txt");

        for (label, path) in [
            ("encoder", &encoder_file),
            ("decoder", &decoder_file),
            ("tokens.txt", &tokens_file),
        ] {
            if !path.exists() {
                return Err(TranscribeError::ModelNotFound(format!(
                    "Cohere {label} not found: {}\n  \
                     Download from https://huggingface.co/cstr/cohere-transcribe-onnx-int8",
                    path.display(),
                )));
            }
        }

        let tokens = load_tokens(&tokens_file)?;
        if tokens.len() != VOCAB_SIZE {
            tracing::warn!(
                "tokens.txt has {} entries; expected {}. Decoder logits dim ({}) \
                 may not align with this tokens file.",
                tokens.len(),
                VOCAB_SIZE,
                VOCAB_SIZE,
            );
        }

        let encoder = build_session(&encoder_file, threads, "encoder")?;
        let decoder = build_session(&decoder_file, threads, "decoder")?;

        let prefix = build_prefix(&tokens, language)?;

        tracing::info!(
            "Cohere model loaded in {:.2}s ({} tokens, language='{}', prefix={:?})",
            start.elapsed().as_secs_f32(),
            tokens.len(),
            language,
            prefix,
        );

        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            tokens,
            prefix,
        })
    }

    /// Run encoder + autoregressive decoder, return generated token ids
    /// (excluding the prefix and EOS).
    fn run_inference(&self, samples: &[f32]) -> Result<Vec<u32>, TranscribeError> {
        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;

        // ---- Encoder ----
        let encoder_start = std::time::Instant::now();
        let n_samples = samples.len();
        let audio_tensor = Tensor::<f32>::from_array(([1usize, n_samples], samples.to_vec()))
            .map_err(|e| TranscribeError::InferenceFailed(format!("audio tensor: {e}")))?;

        let (cross_k_shape, cross_k_data, cross_v_shape, cross_v_data) = {
            let mut encoder = self
                .encoder
                .lock()
                .map_err(|e| TranscribeError::InferenceFailed(format!("encoder lock: {e}")))?;
            let mut outputs = encoder
                .run(ort::inputs!["audio" => audio_tensor])
                .map_err(|e| TranscribeError::InferenceFailed(format!("encoder run: {e}")))?;

            let cross_k_val = outputs.remove("n_layer_cross_k").ok_or_else(|| {
                TranscribeError::InferenceFailed("encoder missing n_layer_cross_k".into())
            })?;
            let cross_v_val = outputs.remove("n_layer_cross_v").ok_or_else(|| {
                TranscribeError::InferenceFailed("encoder missing n_layer_cross_v".into())
            })?;
            let (k_shape, k_data) = cross_k_val
                .try_extract_tensor::<f32>()
                .map_err(|e| TranscribeError::InferenceFailed(format!("extract cross_k: {e}")))?;
            let (v_shape, v_data) = cross_v_val
                .try_extract_tensor::<f32>()
                .map_err(|e| TranscribeError::InferenceFailed(format!("extract cross_v: {e}")))?;
            (
                k_shape.to_vec(),
                k_data.to_vec(),
                v_shape.to_vec(),
                v_data.to_vec(),
            )
        };
        tracing::debug!(
            "Cohere encoder ran in {:.2}s, T_enc={:?}",
            encoder_start.elapsed().as_secs_f32(),
            cross_k_shape,
        );

        // ---- Decoder ----
        let decoder_start = std::time::Instant::now();

        // Self-attention cache: zero-initialized rolling buffer.
        // Cache shape is [N_LAYERS, batch=1, N_HEADS, SELF_KV_CACHE_LEN, HEAD_DIM].
        let cache_elems = N_LAYERS * N_HEADS * SELF_KV_CACHE_LEN * HEAD_DIM;
        let mut self_k_data: Vec<f32> = vec![0.0; cache_elems];
        let mut self_v_data: Vec<f32> = vec![0.0; cache_elems];
        let cache_shape: [usize; 5] = [N_LAYERS, 1, N_HEADS, SELF_KV_CACHE_LEN, HEAD_DIM];

        // Step 1: feed the prefix tokens together so the cache populates in
        // a single call. After this, offset = prefix.len().
        let mut offset: i64 = 0;
        let next_after_prefix = self.decoder_step(
            &self.prefix,
            offset,
            &mut self_k_data,
            &mut self_v_data,
            cache_shape,
            &cross_k_shape,
            &cross_k_data,
            &cross_v_shape,
            &cross_v_data,
        )?;
        offset += self.prefix.len() as i64;

        let mut generated: Vec<i64> = Vec::new();
        if next_after_prefix == TOK_EOS {
            return Ok(Vec::new());
        }
        generated.push(next_after_prefix);

        // Steps 2..N: feed one token per step.
        let max_tokens =
            ((duration_secs * MAX_TOKENS_PER_SECOND) as usize).clamp(16, ABSOLUTE_MAX_TOKENS);
        for _ in 0..max_tokens {
            let last = *generated.last().unwrap();
            let next = self.decoder_step(
                &[last],
                offset,
                &mut self_k_data,
                &mut self_v_data,
                cache_shape,
                &cross_k_shape,
                &cross_k_data,
                &cross_v_shape,
                &cross_v_data,
            )?;
            offset += 1;

            if next == TOK_EOS {
                break;
            }
            if offset as usize >= SELF_KV_CACHE_LEN {
                tracing::warn!("Cohere: self-attention cache full ({}); truncating", offset);
                break;
            }
            generated.push(next);
        }

        tracing::debug!(
            "Cohere decoder produced {} tokens in {:.2}s",
            generated.len(),
            decoder_start.elapsed().as_secs_f32(),
        );

        Ok(generated.into_iter().map(|t| t as u32).collect())
    }

    /// Single decoder forward pass.
    ///
    /// Updates `self_k_data` and `self_v_data` in place from the decoder's
    /// output cache, and returns the predicted next-token id (greedy argmax
    /// over the LAST timestep's logits).
    #[allow(clippy::too_many_arguments)]
    fn decoder_step(
        &self,
        new_tokens: &[i64],
        offset: i64,
        self_k_data: &mut Vec<f32>,
        self_v_data: &mut Vec<f32>,
        cache_shape: [usize; 5],
        cross_k_shape: &[i64],
        cross_k_data: &[f32],
        cross_v_shape: &[i64],
        cross_v_data: &[f32],
    ) -> Result<i64, TranscribeError> {
        let cross_k_shape_us: Vec<usize> = cross_k_shape.iter().map(|&d| d as usize).collect();
        let cross_v_shape_us: Vec<usize> = cross_v_shape.iter().map(|&d| d as usize).collect();
        let n = new_tokens.len();

        let tokens_tensor = Tensor::<i64>::from_array(([1usize, n], new_tokens.to_vec()))
            .map_err(|e| TranscribeError::InferenceFailed(format!("tokens tensor: {e}")))?;
        let self_k_tensor =
            Tensor::<f32>::from_array((cache_shape, std::mem::take(self_k_data)))
                .map_err(|e| TranscribeError::InferenceFailed(format!("self_k tensor: {e}")))?;
        let self_v_tensor =
            Tensor::<f32>::from_array((cache_shape, std::mem::take(self_v_data)))
                .map_err(|e| TranscribeError::InferenceFailed(format!("self_v tensor: {e}")))?;
        let cross_k_tensor =
            Tensor::<f32>::from_array((cross_k_shape_us.clone(), cross_k_data.to_vec()))
                .map_err(|e| TranscribeError::InferenceFailed(format!("cross_k tensor: {e}")))?;
        let cross_v_tensor =
            Tensor::<f32>::from_array((cross_v_shape_us, cross_v_data.to_vec()))
                .map_err(|e| TranscribeError::InferenceFailed(format!("cross_v tensor: {e}")))?;
        let offset_tensor = Tensor::<i64>::from_array(([] as [usize; 0], vec![offset]))
            .map_err(|e| TranscribeError::InferenceFailed(format!("offset tensor: {e}")))?;

        let mut decoder = self
            .decoder
            .lock()
            .map_err(|e| TranscribeError::InferenceFailed(format!("decoder lock: {e}")))?;

        let mut outputs = decoder
            .run(ort::inputs![
                "tokens" => tokens_tensor,
                "in_n_layer_self_k_cache" => self_k_tensor,
                "in_n_layer_self_v_cache" => self_v_tensor,
                "n_layer_cross_k" => cross_k_tensor,
                "n_layer_cross_v" => cross_v_tensor,
                "offset" => offset_tensor,
            ])
            .map_err(|e| TranscribeError::InferenceFailed(format!("decoder run: {e}")))?;

        // Logits: pick the last timestep's argmax.
        let logits_val = outputs
            .remove("logits")
            .ok_or_else(|| TranscribeError::InferenceFailed("decoder missing logits".into()))?;
        let (logits_shape, logits_data) = logits_val
            .try_extract_tensor::<f32>()
            .map_err(|e| TranscribeError::InferenceFailed(format!("extract logits: {e}")))?;
        if logits_shape.len() != 3 || logits_shape[2] as usize != VOCAB_SIZE {
            return Err(TranscribeError::InferenceFailed(format!(
                "unexpected logits shape: {logits_shape:?}, expected [B, T, {VOCAB_SIZE}]"
            )));
        }
        let n_steps = logits_shape[1] as usize;
        let last_offset = (n_steps - 1) * VOCAB_SIZE;
        let last_logits = &logits_data[last_offset..last_offset + VOCAB_SIZE];
        let next_id = argmax(last_logits) as i64;

        // Pull updated cache out and refill our owned buffers.
        let new_k = outputs.remove("out_n_layer_self_k_cache").ok_or_else(|| {
            TranscribeError::InferenceFailed("decoder missing out_n_layer_self_k_cache".into())
        })?;
        let new_v = outputs.remove("out_n_layer_self_v_cache").ok_or_else(|| {
            TranscribeError::InferenceFailed("decoder missing out_n_layer_self_v_cache".into())
        })?;
        let (_, k_data) = new_k
            .try_extract_tensor::<f32>()
            .map_err(|e| TranscribeError::InferenceFailed(format!("extract self_k: {e}")))?;
        let (_, v_data) = new_v
            .try_extract_tensor::<f32>()
            .map_err(|e| TranscribeError::InferenceFailed(format!("extract self_v: {e}")))?;
        *self_k_data = k_data.to_vec();
        *self_v_data = v_data.to_vec();

        Ok(next_id)
    }

    /// Convert generated token ids into text. Filters control / language /
    /// task tokens (anything in the form `<|...|>`, plus `<unk>`/`<pad>`)
    /// and reconstructs SentencePiece word boundaries (U+2581 → space).
    fn decode_tokens(&self, token_ids: &[u32]) -> String {
        let mut out = String::new();
        for &id in token_ids {
            let Some(piece) = self.tokens.get(&id) else {
                continue;
            };
            if is_special_token(piece) {
                continue;
            }
            // Replace the SentencePiece word-boundary marker with a space.
            out.push_str(&piece.replace('\u{2581}', " "));
        }
        out.trim().to_string()
    }
}

impl Transcriber for CohereTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".into()));
        }

        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with Cohere",
            duration_secs,
            samples.len(),
        );

        let start = std::time::Instant::now();
        let token_ids = self.run_inference(samples)?;
        let text = self.decode_tokens(&token_ids).trim().to_string();
        tracing::info!(
            "Cohere transcription completed in {:.2}s: {:?}",
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build an ONNX Runtime session for the encoder or decoder, registering
/// any GPU execution providers that were compiled in via
/// [`super::onnx_ep::register_gpu_eps`].
///
/// Note: there's no `cohere-migraphx` feature today because the int8
/// model uses MatMulNBits(bits=8) which MIGraphX 7.2 can't compile.
/// On the AMD-targeted binary, Cohere runs on the CPU EP only.
fn build_session(
    path: &Path,
    threads: usize,
    label: &str,
) -> Result<Session, TranscribeError> {
    let builder = Session::builder()
        .map_err(|e| TranscribeError::InitFailed(format!("{label} builder: {e}")))?
        .with_intra_threads(threads)
        .map_err(|e| TranscribeError::InitFailed(format!("{label} threads: {e}")))?;

    let mut builder = super::onnx_ep::register_gpu_eps(builder, "Cohere", label)
        .map_err(|e| TranscribeError::InitFailed(format!("{label} EPs: {e}")))?;

    builder.commit_from_file(path).map_err(|e| {
        TranscribeError::InitFailed(format!(
            "Failed to load Cohere {label} from {:?}: {e}",
            path
        ))
    })
}

/// Load `tokens.txt` (one `<piece> <id>\n` line per token). The cstr/Cohere
/// export uses the same NeMo-style layout that other ONNX-engine downloads
/// already use; tolerant of trailing whitespace or CRLF endings.
fn load_tokens(path: &Path) -> Result<HashMap<u32, String>, TranscribeError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        TranscribeError::ModelNotFound(format!("Failed to read {}: {e}", path.display()))
    })?;
    let mut map = HashMap::new();
    for (line_no, raw) in content.lines().enumerate() {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            continue;
        }
        // Format is `<piece><space><id>`. Piece may itself contain spaces
        // for some special tokens, but in this export the last whitespace
        // separator is unambiguous because IDs are integers.
        let last_space = line.rfind(char::is_whitespace).ok_or_else(|| {
            TranscribeError::ModelNotFound(format!(
                "{}:{}: malformed token line: {line:?}",
                path.display(),
                line_no + 1,
            ))
        })?;
        let (piece, id_str) = line.split_at(last_space);
        let id: u32 = id_str.trim().parse().map_err(|_| {
            TranscribeError::ModelNotFound(format!(
                "{}:{}: non-integer token id in {line:?}",
                path.display(),
                line_no + 1,
            ))
        })?;
        map.insert(id, piece.to_string());
    }
    Ok(map)
}

/// Build the decoder prefix sequence for a given language code.
///
/// The prefix is `[<|sot|>, <|<lang>|>, <|pnc|>, <|itn|>, <|notimestamp|>,
/// <|nodiarize|>]`. We resolve the language and task tokens by name from
/// `tokens.txt` rather than hard-coding IDs so the wiring survives a
/// future export that renumbers tokens.
fn build_prefix(
    tokens: &HashMap<u32, String>,
    language: &str,
) -> Result<Vec<i64>, TranscribeError> {
    let lang = language.trim().to_ascii_lowercase();
    if !SUPPORTED_LANGUAGES.contains(&lang.as_str()) {
        return Err(TranscribeError::InitFailed(format!(
            "Cohere does not officially support language '{language}'. \
             Supported languages: {SUPPORTED_LANGUAGES:?}",
        )));
    }
    let lang_tag = format!("<|{lang}|>");
    let lookup = |name: &str| -> Result<i64, TranscribeError> {
        tokens
            .iter()
            .find_map(|(id, piece)| (piece == name).then_some(*id as i64))
            .ok_or_else(|| {
                TranscribeError::InitFailed(format!(
                    "Cohere tokens.txt missing required special token {name:?}"
                ))
            })
    };
    Ok(vec![
        lookup("<|startoftranscript|>")?,
        lookup(&lang_tag)?,
        lookup("<|pnc|>")?,
        lookup("<|itn|>")?,
        lookup("<|notimestamp|>")?,
        lookup("<|nodiarize|>")?,
    ])
}

/// True for Cohere control / language / task tokens. These are stripped
/// from the decoded output so users don't see literal `<|en|>` strings.
fn is_special_token(piece: &str) -> bool {
    if piece.starts_with("<|") && piece.ends_with("|>") {
        return true;
    }
    matches!(piece, "<unk>" | "<pad>" | "<s>" | "</s>")
}

/// Greedy argmax over a 1-D logits slice.
fn argmax(logits: &[f32]) -> usize {
    let mut best = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (i, &v) in logits.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best = i;
        }
    }
    best
}

/// Resolve a model name or path to a directory containing the Cohere ONNX files.
fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    let models_dir = crate::config::Config::models_dir();
    let candidate = models_dir.join(model);
    if candidate.exists() {
        return Ok(candidate);
    }

    let local = PathBuf::from("models").join(model);
    if local.exists() {
        return Ok(local);
    }

    Err(TranscribeError::ModelNotFound(format!(
        "Cohere model '{}' not found. Looked in:\n  - {}\n  - {}\n  - {}",
        model,
        path.display(),
        candidate.display(),
        local.display(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    /// Load a 16 kHz mono WAV into f32 samples in [-1, 1].
    fn load_wav(path: &Path) -> Vec<f32> {
        let reader = hound::WavReader::open(path)
            .unwrap_or_else(|e| panic!("Failed to open {}: {}", path.display(), e));
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, 16_000, "Expected 16 kHz audio");
        assert_eq!(spec.channels, 1, "Expected mono audio");

        let max_val = (1i64 << (spec.bits_per_sample - 1)) as f32;
        reader
            .into_samples::<i32>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / max_val)
            .collect()
    }

    /// End-to-end PoC: load the int8 Cohere model and transcribe a fixture WAV.
    ///
    /// Run with:
    /// ```bash
    /// VOXTYPE_COHERE_MODEL_DIR=~/.cache/voxtype-models/cohere-transcribe-int8 \
    ///     cargo test --features cohere transcribe::cohere::tests::cohere_poc \
    ///                -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn cohere_poc() {
        let model_dir = std::env::var("VOXTYPE_COHERE_MODEL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("models")
                    .join("cohere-transcribe-int8")
            });

        assert!(
            model_dir.exists(),
            "Cohere model dir not found at {}. See module docs for download instructions.",
            model_dir.display()
        );

        let transcriber =
            CohereTranscriber::from_dir(&model_dir).expect("Failed to load Cohere transcriber");

        let wav_path = fixtures_dir().join("vad").join("speech_hello.wav");
        let samples = load_wav(&wav_path);
        assert!(
            !samples.is_empty(),
            "Loaded zero samples from {}",
            wav_path.display()
        );

        let text = transcriber
            .transcribe(&samples)
            .expect("Cohere transcription failed");
        eprintln!("Cohere PoC transcription: {:?}", text);
    }

    #[test]
    fn resolve_model_path_not_found() {
        let result = resolve_model_path("/nonexistent/cohere/path");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TranscribeError::ModelNotFound(_)
        ));
    }

    #[test]
    fn argmax_picks_highest() {
        assert_eq!(argmax(&[0.1, 0.5, 0.3, 0.4]), 1);
        assert_eq!(argmax(&[1.0]), 0);
        assert_eq!(argmax(&[-1.0, -0.5, -0.9]), 1);
    }

    #[test]
    fn build_prefix_rejects_unsupported_language() {
        let tokens = HashMap::new();
        let err = build_prefix(&tokens, "klingon").unwrap_err();
        assert!(matches!(err, TranscribeError::InitFailed(_)));
    }

    #[test]
    fn build_prefix_against_real_tokens_txt() {
        // Sanity check: build_prefix yields the documented IDs when the real
        // tokens.txt is available. Skipped when the model isn't downloaded.
        let Ok(dir) = std::env::var("VOXTYPE_COHERE_MODEL_DIR").map(PathBuf::from) else {
            return;
        };
        let tokens_path = dir.join("tokens.txt");
        if !tokens_path.exists() {
            return;
        }
        let tokens = load_tokens(&tokens_path).expect("tokens.txt should load");
        let prefix = build_prefix(&tokens, "en").expect("build_prefix English");
        assert_eq!(prefix, vec![4, 62, 5, 8, 11, 13]);
        assert_eq!(
            tokens.get(&3).map(String::as_str),
            Some("<|endoftext|>"),
            "EOS token id 3 should map to <|endoftext|>"
        );
    }

    #[test]
    fn build_prefix_lookup_uses_named_tokens() {
        // Synthesize a minimal tokens.txt-equivalent map and check that
        // build_prefix resolves names correctly (no hard-coded IDs).
        let mut tokens = HashMap::new();
        tokens.insert(4, "<|startoftranscript|>".to_string());
        tokens.insert(5, "<|pnc|>".to_string());
        tokens.insert(8, "<|itn|>".to_string());
        tokens.insert(11, "<|notimestamp|>".to_string());
        tokens.insert(13, "<|nodiarize|>".to_string());
        tokens.insert(62, "<|en|>".to_string());
        let prefix = build_prefix(&tokens, "en").unwrap();
        assert_eq!(prefix, vec![4, 62, 5, 8, 11, 13]);

        // If the language token is missing, error rather than panic.
        let mut partial = tokens.clone();
        partial.remove(&62);
        assert!(build_prefix(&partial, "en").is_err());
    }

    #[test]
    fn special_token_filter() {
        assert!(is_special_token("<|en|>"));
        assert!(is_special_token("<|startoftranscript|>"));
        assert!(is_special_token("<|endoftext|>"));
        assert!(is_special_token("<unk>"));
        assert!(is_special_token("<pad>"));
        assert!(!is_special_token("hello"));
        assert!(!is_special_token("\u{2581}world"));
    }
}

// Trip-wire: keep D_MODEL aligned with N_HEADS * HEAD_DIM.
const _: () = {
    if D_MODEL != N_HEADS * HEAD_DIM {
        panic!("D_MODEL must equal N_HEADS * HEAD_DIM");
    }
};
