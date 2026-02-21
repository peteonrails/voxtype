//! Paraformer-based speech-to-text transcription
//!
//! Uses Alibaba's Paraformer model via ONNX Runtime for local transcription.
//! Paraformer is a non-autoregressive encoder-predictor-decoder model that
//! generates all output tokens in a single pass (no autoregressive loop).
//!
//! Preprocessing reuses the shared Fbank pipeline (fbank.rs) with identical
//! parameters to SenseVoice: 80-dim Fbank, LFR m=7/n=6, CMVN normalization.
//! The key difference is CMVN stats come from an am.mvn file (Kaldi binary
//! matrix format) rather than ONNX model metadata.
//!
//! Pipeline: Audio (f32, 16kHz) -> Fbank (80-dim) -> LFR (560-dim) -> CMVN -> ONNX -> token decode
//!
//! Languages: zh+en (bilingual), zh+yue+en (trilingual)
//! Model files: model.int8.onnx (or model.onnx), tokens.txt, am.mvn

use super::fbank::{self, FbankExtractor, LfrConfig};
use super::ctc;
use super::Transcriber;
use crate::config::ParaformerConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::PathBuf;

/// Sample rate expected by Paraformer
const SAMPLE_RATE: usize = 16000;

/// Paraformer-based transcriber using ONNX Runtime
pub struct ParaformerTranscriber {
    session: std::sync::Mutex<Session>,
    tokens: HashMap<u32, String>,
    neg_mean: Vec<f32>,
    inv_stddev: Vec<f32>,
    fbank_extractor: FbankExtractor,
}

impl ParaformerTranscriber {
    pub fn new(config: &ParaformerConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;

        tracing::info!("Loading Paraformer model from {:?}", model_dir);
        let start = std::time::Instant::now();

        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));

        // Find model file (prefer int8 quantized)
        let model_file = {
            let int8 = model_dir.join("model.int8.onnx");
            let full = model_dir.join("model.onnx");
            if int8.exists() {
                int8
            } else if full.exists() {
                tracing::info!("Using full-precision model (model.int8.onnx not found)");
                full
            } else {
                return Err(TranscribeError::ModelNotFound(format!(
                    "Paraformer model not found in {:?}\n  \
                     Expected model.int8.onnx or model.onnx\n  \
                     Run: voxtype setup model",
                    model_dir
                )));
            }
        };

        // Load tokens.txt
        let tokens_path = model_dir.join("tokens.txt");
        if !tokens_path.exists() {
            return Err(TranscribeError::ModelNotFound(format!(
                "Paraformer tokens.txt not found: {}\n  \
                 Ensure tokens.txt is in the model directory.",
                tokens_path.display()
            )));
        }
        let tokens = ctc::load_tokens(&tokens_path)?;
        tracing::debug!("Loaded {} tokens", tokens.len());

        // Create ONNX session
        let session = Session::builder()
            .map_err(|e| {
                TranscribeError::InitFailed(format!("ONNX session builder failed: {}", e))
            })?
            .with_intra_threads(threads)
            .map_err(|e| {
                TranscribeError::InitFailed(format!("Failed to set threads: {}", e))
            })?
            .commit_from_file(&model_file)
            .map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load Paraformer model from {:?}: {}",
                    model_file, e
                ))
            })?;

        // Read CMVN stats from am.mvn (Kaldi binary matrix)
        let mvn_path = model_dir.join("am.mvn");
        let (neg_mean, inv_stddev) = if mvn_path.exists() {
            read_cmvn_from_kaldi_mvn(&mvn_path)?
        } else {
            // Fall back to ONNX model metadata (like SenseVoice)
            tracing::info!("am.mvn not found, trying ONNX model metadata for CMVN");
            read_cmvn_from_metadata(&session)?
        };

        let fbank_extractor = FbankExtractor::new_default();

        tracing::info!(
            "Paraformer model loaded in {:.2}s",
            start.elapsed().as_secs_f32(),
        );

        Ok(Self {
            session: std::sync::Mutex::new(session),
            tokens,
            neg_mean,
            inv_stddev,
            fbank_extractor,
        })
    }
}

impl Transcriber for ParaformerTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with Paraformer",
            duration_secs,
            samples.len(),
        );

        let start = std::time::Instant::now();

        // 1. Extract Fbank features (shared pipeline, identical to SenseVoice)
        let fbank_start = std::time::Instant::now();
        let fbank_features = self.fbank_extractor.extract(samples);
        tracing::debug!(
            "Fbank extraction: {:.2}s ({} frames x {})",
            fbank_start.elapsed().as_secs_f32(),
            fbank_features.nrows(),
            fbank_features.ncols(),
        );

        if fbank_features.nrows() == 0 {
            return Err(TranscribeError::AudioFormat(
                "Audio too short for feature extraction".to_string(),
            ));
        }

        // 2. LFR stacking (shared, same m=7/n=6 as SenseVoice)
        let lfr = fbank::apply_lfr(&fbank_features, &LfrConfig::default());
        tracing::debug!("LFR output: {} frames x {}", lfr.nrows(), lfr.ncols());

        // 3. CMVN normalization (shared, stats from am.mvn)
        let mut features = lfr;
        fbank::apply_cmvn(&mut features, &self.neg_mean, &self.inv_stddev);

        // 4. Build ONNX inputs
        let num_frames = features.nrows();
        let feat_dim = features.ncols();

        // speech: shape [1, T, 560]
        let (x_data, _offset) = features.into_raw_vec_and_offset();
        let speech_tensor = Tensor::<f32>::from_array(([1usize, num_frames, feat_dim], x_data))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create speech tensor: {}",
                    e
                ))
            })?;

        // speech_lengths: shape [1]
        let lengths_tensor = Tensor::<i32>::from_array(([1usize], vec![num_frames as i32]))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create lengths tensor: {}",
                    e
                ))
            })?;

        // 5. Run inference
        let inference_start = std::time::Instant::now();
        let mut session = self.session.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock session: {}", e))
        })?;

        let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
            (std::borrow::Cow::Borrowed("speech"), speech_tensor.into()),
            (
                std::borrow::Cow::Borrowed("speech_lengths"),
                lengths_tensor.into(),
            ),
        ];

        let outputs = session.run(inputs).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Paraformer inference failed: {}", e))
        })?;

        tracing::debug!(
            "ONNX inference: {:.2}s",
            inference_start.elapsed().as_secs_f32(),
        );

        // 6. Extract output and decode tokens
        // Paraformer outputs token IDs directly (not CTC logits)
        let result = decode_paraformer_output(&outputs, &self.tokens)?;

        tracing::info!(
            "Paraformer transcription completed in {:.2}s: {:?}",
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

/// Decode Paraformer ONNX output to text
///
/// Paraformer outputs token IDs directly from its CIF+decoder pipeline.
/// The output may be named "logits" and shaped as either:
/// - [batch, seq_len] with i64 token IDs
/// - [batch, seq_len] with f32 token IDs (pre-argmaxed)
/// - [batch, seq_len, vocab_size] with f32 logits (needs argmax)
fn decode_paraformer_output(
    outputs: &ort::session::SessionOutputs,
    tokens: &HashMap<u32, String>,
) -> Result<String, TranscribeError> {
    // Try to find the output tensor - name varies across model exports
    let output_val = outputs
        .get("logits")
        .or_else(|| outputs.get("output"))
        .ok_or_else(|| {
            TranscribeError::InferenceFailed(
                "Paraformer model output not found (expected 'logits' or 'output')".to_string(),
            )
        })?;

    // Try extracting as i64 first (direct token IDs)
    if let Ok((shape, data)) = output_val.try_extract_tensor::<i64>() {
        tracing::debug!("Paraformer output shape (i64): {:?}", &*shape);
        let token_ids: Vec<u32> = data.iter().map(|&id| id as u32).collect();
        return Ok(tokens_to_text(&token_ids, tokens));
    }

    // Try extracting as f32
    let (shape, data) = output_val.try_extract_tensor::<f32>().map_err(|e| {
        TranscribeError::InferenceFailed(format!("Failed to extract Paraformer output: {}", e))
    })?;

    let shape_dims: &[i64] = shape;
    tracing::debug!("Paraformer output shape (f32): {:?}", shape_dims);

    if shape_dims.len() == 3 {
        // [batch, seq_len, vocab_size] - needs argmax then BPE-aware decoding
        let seq_len = shape_dims[1] as usize;
        let vocab_size = shape_dims[2] as usize;
        // Do CTC argmax + dedup + blank removal to get token IDs,
        // then pass through tokens_to_text which handles @@ BPE markers
        let token_ids = ctc_decode_to_ids(data, seq_len, vocab_size);
        Ok(tokens_to_text(&token_ids, tokens))
    } else if shape_dims.len() == 2 {
        // [batch, seq_len] - pre-argmaxed token IDs as f32
        let seq_len = shape_dims[1] as usize;
        let token_ids: Vec<u32> = data[..seq_len]
            .iter()
            .map(|&v| v as u32)
            .collect();
        Ok(tokens_to_text(&token_ids, tokens))
    } else {
        Err(TranscribeError::InferenceFailed(format!(
            "Unexpected Paraformer output shape: {:?}",
            shape_dims
        )))
    }
}

/// CTC greedy decode to token IDs: argmax per frame, collapse duplicates, remove blanks
fn ctc_decode_to_ids(logits: &[f32], time_steps: usize, vocab_size: usize) -> Vec<u32> {
    let blank_id: u32 = 0;
    let mut token_ids: Vec<u32> = Vec::new();
    let mut prev_id: Option<u32> = None;

    for t in 0..time_steps {
        let offset = t * vocab_size;
        let frame = &logits[offset..offset + vocab_size];

        let best_id = frame
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx as u32)
            .unwrap_or(blank_id);

        if best_id != blank_id && Some(best_id) != prev_id {
            token_ids.push(best_id);
        }
        prev_id = Some(best_id);
    }

    token_ids
}

/// Convert token IDs to text, handling BPE continuation markers
///
/// Paraformer uses `@@` suffix for BPE continuation (e.g., "hel@@" + "lo" = "hello").
/// Chinese characters appear as individual tokens without markers.
/// Special tokens (<blank>, <s>, </s>, <OOV>) are filtered out.
fn tokens_to_text(token_ids: &[u32], tokens: &HashMap<u32, String>) -> String {
    let mut result = String::new();

    for &id in token_ids {
        if let Some(token_str) = tokens.get(&id) {
            // Skip special tokens
            if token_str.starts_with('<') && token_str.ends_with('>') {
                continue;
            }

            // Handle BPE continuation marker
            if let Some(base) = token_str.strip_suffix("@@") {
                result.push_str(base);
            } else {
                // SentencePiece marker cleanup (some models use this instead of @@)
                result.push_str(&token_str.replace('\u{2581}', " "));
            }
        }
    }

    result.trim().to_string()
}

/// Read CMVN stats from Kaldi am.mvn binary matrix file
///
/// Format: binary header + 2-row float matrix where:
/// - Row 0: accumulated feature sums, last element = frame count
/// - Row 1: accumulated squared sums, last element = 0
///
/// Returns (neg_mean, inv_stddev) for use with apply_cmvn()
fn read_cmvn_from_kaldi_mvn(
    path: &std::path::Path,
) -> Result<(Vec<f32>, Vec<f32>), TranscribeError> {
    let data = std::fs::read(path).map_err(|e| {
        TranscribeError::InitFailed(format!("Failed to read am.mvn: {}", e))
    })?;

    let mut pos = 0;

    // Skip binary header: "\0B" marker
    if data.len() < 2 || data[0] != 0x00 || data[1] != b'B' {
        return Err(TranscribeError::InitFailed(
            "am.mvn: invalid Kaldi binary marker".to_string(),
        ));
    }
    pos += 2;

    // Skip optional space after header
    if pos < data.len() && data[pos] == b' ' {
        pos += 1;
    }

    // Read matrix type: "FM" for float, "DM" for double
    let is_double = if pos + 2 <= data.len() {
        let tag = &data[pos..pos + 2];
        pos += 2;
        if tag == b"FM" {
            false
        } else if tag == b"DM" {
            true
        } else {
            return Err(TranscribeError::InitFailed(format!(
                "am.mvn: unexpected matrix type tag: {:?}",
                tag
            )));
        }
    } else {
        return Err(TranscribeError::InitFailed(
            "am.mvn: truncated matrix header".to_string(),
        ));
    };

    // Skip optional space
    if pos < data.len() && data[pos] == b' ' {
        pos += 1;
    }

    // Read dimensions: \4<rows:i32> \4<cols:i32>
    if pos >= data.len() || data[pos] != 4 {
        return Err(TranscribeError::InitFailed(
            "am.mvn: expected \\4 before rows".to_string(),
        ));
    }
    pos += 1;
    if pos + 4 > data.len() {
        return Err(TranscribeError::InitFailed(
            "am.mvn: truncated rows value".to_string(),
        ));
    }
    let rows = i32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;

    if pos >= data.len() || data[pos] != 4 {
        return Err(TranscribeError::InitFailed(
            "am.mvn: expected \\4 before cols".to_string(),
        ));
    }
    pos += 1;
    if pos + 4 > data.len() {
        return Err(TranscribeError::InitFailed(
            "am.mvn: truncated cols value".to_string(),
        ));
    }
    let cols = i32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;

    if rows != 2 {
        return Err(TranscribeError::InitFailed(format!(
            "am.mvn: expected 2 rows, got {}",
            rows
        )));
    }

    tracing::debug!("am.mvn: {} rows x {} cols, double={}", rows, cols, is_double);

    // Read matrix data
    let feat_dim = cols - 1; // last column is the count
    let matrix: Vec<Vec<f64>> = if is_double {
        let elem_size = 8;
        let total = rows * cols * elem_size;
        if pos + total > data.len() {
            return Err(TranscribeError::InitFailed(
                "am.mvn: truncated matrix data".to_string(),
            ));
        }
        (0..rows)
            .map(|r| {
                (0..cols)
                    .map(|c| {
                        let offset = pos + (r * cols + c) * elem_size;
                        f64::from_le_bytes(data[offset..offset + elem_size].try_into().unwrap())
                    })
                    .collect()
            })
            .collect()
    } else {
        let elem_size = 4;
        let total = rows * cols * elem_size;
        if pos + total > data.len() {
            return Err(TranscribeError::InitFailed(
                "am.mvn: truncated matrix data".to_string(),
            ));
        }
        (0..rows)
            .map(|r| {
                (0..cols)
                    .map(|c| {
                        let offset = pos + (r * cols + c) * elem_size;
                        f32::from_le_bytes(data[offset..offset + elem_size].try_into().unwrap())
                            as f64
                    })
                    .collect()
            })
            .collect()
    };

    // Extract mean and variance from accumulated stats
    let count = matrix[0][feat_dim]; // frame count is last element of row 0
    if count <= 0.0 {
        return Err(TranscribeError::InitFailed(
            "am.mvn: zero frame count".to_string(),
        ));
    }

    let mut neg_mean = Vec::with_capacity(feat_dim);
    let mut inv_stddev = Vec::with_capacity(feat_dim);

    for i in 0..feat_dim {
        let mean = matrix[0][i] / count;
        let variance = (matrix[1][i] / count) - (mean * mean);
        let stddev = variance.max(1e-20).sqrt();
        neg_mean.push(-mean as f32);
        inv_stddev.push((1.0 / stddev) as f32);
    }

    tracing::debug!(
        "CMVN stats loaded from am.mvn: {} dimensions, {:.0} frames",
        feat_dim,
        count
    );

    Ok((neg_mean, inv_stddev))
}

/// Read CMVN stats from ONNX model metadata (fallback if no am.mvn)
fn read_cmvn_from_metadata(session: &Session) -> Result<(Vec<f32>, Vec<f32>), TranscribeError> {
    let metadata = session.metadata().map_err(|e| {
        TranscribeError::InitFailed(format!("Failed to read model metadata: {}", e))
    })?;

    let neg_mean_str = metadata.custom("neg_mean").ok_or_else(|| {
        TranscribeError::InitFailed(
            "Model has no am.mvn file and no CMVN metadata. \
             Ensure am.mvn is in the model directory."
                .to_string(),
        )
    })?;

    let inv_stddev_str = metadata.custom("inv_stddev").ok_or_else(|| {
        TranscribeError::InitFailed(
            "Model metadata missing 'inv_stddev' key".to_string(),
        )
    })?;

    let neg_mean: Vec<f32> = neg_mean_str
        .split(',')
        .filter_map(|s: &str| s.trim().parse::<f32>().ok())
        .collect();

    let inv_stddev: Vec<f32> = inv_stddev_str
        .split(',')
        .filter_map(|s: &str| s.trim().parse::<f32>().ok())
        .collect();

    if neg_mean.is_empty() || inv_stddev.is_empty() {
        return Err(TranscribeError::InitFailed(format!(
            "CMVN stats malformed (neg_mean: {} values, inv_stddev: {} values)",
            neg_mean.len(),
            inv_stddev.len()
        )));
    }

    Ok((neg_mean, inv_stddev))
}

/// Resolve model name to directory path
fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    // Map short names to directory names
    let model_dir_name = if model.starts_with("paraformer-") {
        model.to_string()
    } else {
        format!("paraformer-{}", model)
    };

    let models_dir = crate::config::Config::models_dir();
    let model_path = models_dir.join(&model_dir_name);

    if model_path.exists() {
        return Ok(model_path);
    }

    // Check without prefix
    let alt_path = models_dir.join(model);
    if alt_path.exists() {
        return Ok(alt_path);
    }

    // Check sherpa-onnx naming convention
    let sherpa_name = format!("sherpa-onnx-{}", model_dir_name);
    let sherpa_path = models_dir.join(&sherpa_name);
    if sherpa_path.exists() {
        return Ok(sherpa_path);
    }

    Err(TranscribeError::ModelNotFound(format!(
        "Paraformer model '{}' not found. Looked in:\n  \
         - {}\n  \
         - {}\n  \
         - {}\n\n\
         Run: voxtype setup model",
        model,
        model_path.display(),
        alt_path.display(),
        sherpa_path.display(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_resolve_model_path_not_found() {
        let result = resolve_model_path("/nonexistent/path/to/model");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TranscribeError::ModelNotFound(_)
        ));
    }

    #[test]
    fn test_resolve_model_path_absolute() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();
        fs::write(model_path.join("model.int8.onnx"), b"dummy").unwrap();

        let resolved = resolve_model_path(model_path.to_str().unwrap());
        assert!(resolved.is_ok());
        assert_eq!(resolved.unwrap(), model_path);
    }

    #[test]
    fn test_tokens_to_text_chinese() {
        let mut tokens = HashMap::new();
        tokens.insert(0, "<blank>".to_string());
        tokens.insert(1, "<s>".to_string());
        tokens.insert(2, "</s>".to_string());
        tokens.insert(10, "你".to_string());
        tokens.insert(11, "好".to_string());
        tokens.insert(12, "世".to_string());
        tokens.insert(13, "界".to_string());

        let ids = vec![1, 10, 11, 12, 13, 2];
        let result = tokens_to_text(&ids, &tokens);
        assert_eq!(result, "你好世界");
    }

    #[test]
    fn test_tokens_to_text_bpe() {
        let mut tokens = HashMap::new();
        tokens.insert(0, "<blank>".to_string());
        tokens.insert(1, "<s>".to_string());
        tokens.insert(2, "</s>".to_string());
        tokens.insert(10, "hel@@".to_string());
        tokens.insert(11, "lo".to_string());
        tokens.insert(12, "wor@@".to_string());
        tokens.insert(13, "ld".to_string());

        let ids = vec![1, 10, 11, 12, 13, 2];
        let result = tokens_to_text(&ids, &tokens);
        assert_eq!(result, "helloworld");
    }

    #[test]
    fn test_tokens_to_text_sentencepiece() {
        let mut tokens = HashMap::new();
        tokens.insert(10, "\u{2581}hello".to_string());
        tokens.insert(11, "\u{2581}world".to_string());

        let ids = vec![10, 11];
        let result = tokens_to_text(&ids, &tokens);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_read_cmvn_kaldi_float() {
        let temp_dir = TempDir::new().unwrap();
        let mvn_path = temp_dir.path().join("am.mvn");

        // Build a minimal Kaldi binary float matrix: 2 rows x 4 cols
        // Row 0: [10.0, 20.0, 30.0, 5.0]  (sums + count=5)
        // Row 1: [30.0, 100.0, 200.0, 0.0] (sum of squares)
        let mut data: Vec<u8> = Vec::new();
        data.push(0x00); // binary marker
        data.push(b'B');
        data.push(b'F'); // float matrix
        data.push(b'M');
        data.push(b' ');
        data.push(4); // \4 before rows
        data.extend_from_slice(&2i32.to_le_bytes());
        data.push(4); // \4 before cols
        data.extend_from_slice(&4i32.to_le_bytes());
        // Row 0: sums
        for v in &[10.0f32, 20.0, 30.0, 5.0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        // Row 1: sum of squares
        for v in &[30.0f32, 100.0, 200.0, 0.0] {
            data.extend_from_slice(&v.to_le_bytes());
        }

        fs::write(&mvn_path, &data).unwrap();

        let (neg_mean, inv_stddev) = read_cmvn_from_kaldi_mvn(&mvn_path).unwrap();
        assert_eq!(neg_mean.len(), 3);
        assert_eq!(inv_stddev.len(), 3);

        // mean[0] = 10/5 = 2.0, neg_mean[0] = -2.0
        assert!((neg_mean[0] - (-2.0)).abs() < 1e-5);
        // mean[1] = 20/5 = 4.0, neg_mean[1] = -4.0
        assert!((neg_mean[1] - (-4.0)).abs() < 1e-5);
        // variance[0] = 30/5 - 4 = 2.0, stddev = sqrt(2), inv = 1/sqrt(2)
        assert!((inv_stddev[0] - (1.0 / 2.0f32.sqrt())).abs() < 1e-5);
    }
}
