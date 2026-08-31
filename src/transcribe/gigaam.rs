//! GigaAM speech-to-text transcription
//!
//! Uses SberDevices' GigaAM-v3 e2e RNN-T model via ONNX Runtime for local
//! Russian transcription with built-in punctuation and text normalization
//! (the `e2e` variant emits punctuated, normalized text directly).
//!
//! The model exports as three ONNX graphs:
//! - `encoder.onnx`:  log-mel (64-dim, torchaudio conventions) -> [B, 768, T']
//! - `decoder.onnx`:  prediction network (embedding + 1-layer LSTM), stateful
//! - `joint.onnx`:    combines encoder frame + prediction -> vocab logits
//!
//! Preprocessing is NOT part of the ONNX export (torch.stft does not
//! export), so this module implements the log-mel extraction in Rust and
//! pins it against a Python-generated reference in a unit test.
//!
//! Decoding is greedy RNN-T, mirroring gigaam/onnx_utils.py
//! (`_decode_rnnt_batch`): per encoder frame, up to MAX_LETTERS_PER_FRAME
//! emissions, blank = vocab_size (the last joint logit column).
//!
//! Model files: encoder.onnx, decoder.onnx, joint.onnx, tokens.txt
//! (one SentencePiece piece per line; `▁` marks word boundaries).
//!
//! Language: ru (Russian). Export the ONNX files with
//! scripts/export_gigaam_onnx.py from the GigaAM repository.

use super::gigaam_mel::GigaAMMelExtractor;
use super::Transcriber;
use crate::config::GigaAMConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::path::PathBuf;

/// Sample rate expected by GigaAM
const SAMPLE_RATE: usize = 16000;

/// Max tokens emitted per encoder frame, matching upstream gigaam ONNX decoding.
const MAX_LETTERS_PER_FRAME: usize = 3;

/// GigaAM-based transcriber using ONNX Runtime
pub struct GigaAMTranscriber {
    encoder: std::sync::Mutex<Session>,
    decoder: std::sync::Mutex<Session>,
    joint: std::sync::Mutex<Session>,
    tokens: Vec<String>,
    mel_extractor: GigaAMMelExtractor,
    pred_hidden: usize,
    pred_rnn_layers: usize,
}

impl GigaAMTranscriber {
    pub fn new(config: &GigaAMConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;

        tracing::info!("Loading GigaAM model from {:?}", model_dir);
        let start = std::time::Instant::now();

        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));

        let encoder_file = {
            let plain = model_dir.join("encoder.onnx");
            let versioned = model_dir.join("v3_e2e_rnnt_encoder.onnx");
            if plain.exists() {
                plain
            } else if versioned.exists() {
                versioned
            } else {
                return Err(TranscribeError::ModelNotFound(format!(
                    "GigaAM encoder not found in {:?}\n  \
                     Expected encoder.onnx (see scripts/export_gigaam_onnx.py)",
                    model_dir
                )));
            }
        };
        let decoder_file = {
            let plain = model_dir.join("decoder.onnx");
            let versioned = model_dir.join("v3_e2e_rnnt_decoder.onnx");
            if plain.exists() {
                plain
            } else {
                versioned
            }
        };
        let joint_file = {
            let plain = model_dir.join("joint.onnx");
            let versioned = model_dir.join("v3_e2e_rnnt_joint.onnx");
            if plain.exists() {
                plain
            } else {
                versioned
            }
        };

        let build_session = |path: &PathBuf, name: &str| -> Result<Session, TranscribeError> {
            Session::builder()
                .map_err(|e| {
                    TranscribeError::InitFailed(format!("ONNX session builder failed: {}", e))
                })?
                .with_intra_threads(threads)
                .map_err(|e| TranscribeError::InitFailed(format!("Failed to set threads: {}", e)))?
                .commit_from_file(path)
                .map_err(|e| {
                    TranscribeError::InitFailed(format!(
                        "Failed to load GigaAM {} from {:?}: {}",
                        name, path, e
                    ))
                })
        };

        let encoder = build_session(&encoder_file, "encoder")?;
        let decoder = build_session(&decoder_file, "decoder")?;
        let joint = build_session(&joint_file, "joint")?;

        // Load tokens.txt (one SentencePiece piece per line)
        let tokens_path = model_dir.join("tokens.txt");
        if !tokens_path.exists() {
            return Err(TranscribeError::ModelNotFound(format!(
                "GigaAM tokens.txt not found: {}",
                tokens_path.display()
            )));
        }
        let tokens = std::fs::read_to_string(&tokens_path)
            .map_err(|e| {
                TranscribeError::ModelNotFound(format!("Failed to read tokens.txt: {}", e))
            })?
            .lines()
            .map(|l| l.to_string())
            .collect::<Vec<_>>();
        tracing::debug!("Loaded {} tokens", tokens.len());

        // Prediction-network geometry for v3_e2e_rnnt (from the exported
        // model config): 1-layer LSTM, hidden 320, encoder output dim 768.
        // ponytail: constants instead of shape introspection; revisit when
        // a second GigaAM revision lands.
        let pred_hidden = 320usize;
        let pred_rnn_layers = 1usize;

        let mel_extractor = GigaAMMelExtractor::new_default();

        tracing::info!(
            "GigaAM model loaded in {:.2}s (pred_hidden={}, pred_layers={})",
            start.elapsed().as_secs_f32(),
            pred_hidden,
            pred_rnn_layers,
        );

        Ok(Self {
            encoder: std::sync::Mutex::new(encoder),
            decoder: std::sync::Mutex::new(decoder),
            joint: std::sync::Mutex::new(joint),
            tokens,
            mel_extractor,
            pred_hidden,
            pred_rnn_layers,
        })
    }
}

impl Transcriber for GigaAMTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with GigaAM",
            duration_secs,
            samples.len(),
        );

        let start = std::time::Instant::now();

        // 1. Log-mel features (torchaudio-compatible, 64-dim)
        let mel_start = std::time::Instant::now();
        let features = self.mel_extractor.extract(samples);
        tracing::debug!(
            "Mel extraction: {:.2}s ({} frames x {})",
            mel_start.elapsed().as_secs_f32(),
            features.ncols(),
            features.nrows(),
        );
        if features.ncols() == 0 {
            return Err(TranscribeError::AudioFormat(
                "Audio too short for feature extraction".to_string(),
            ));
        }

        // 2. Encoder: audio_signal [1, 64, T] (f32), length [1] (i64)
        let num_frames = features.ncols();
        let feat_dim = features.nrows();
        let (x_data, _offset) = features.into_raw_vec_and_offset();
        let signal_tensor = Tensor::<f32>::from_array(([1usize, feat_dim, num_frames], x_data))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to create audio tensor: {}", e))
            })?;
        let length_tensor = Tensor::<i64>::from_array(([1usize], vec![num_frames as i64]))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to create length tensor: {}", e))
            })?;

        let inference_start = std::time::Instant::now();
        let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
            (
                std::borrow::Cow::Borrowed("audio_signal"),
                signal_tensor.into(),
            ),
            (std::borrow::Cow::Borrowed("length"), length_tensor.into()),
        ];
        let mut encoder = self.encoder.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock encoder: {}", e))
        })?;
        let outputs = encoder.run(inputs).map_err(|e| {
            TranscribeError::InferenceFailed(format!("GigaAM encoder failed: {}", e))
        })?;
        tracing::debug!(
            "Encoder inference: {:.2}s",
            inference_start.elapsed().as_secs_f32(),
        );

        // encoded: [1, enc_dim, T']; encoded_len: [1]
        let (enc_shape, enc_data) =
            outputs["encoded"]
                .try_extract_tensor::<f32>()
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!(
                        "Failed to extract encoder output: {}",
                        e
                    ))
                })?;
        let (_, enc_len_data) =
            outputs["encoded_len"]
                .try_extract_tensor::<i32>()
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!(
                        "Failed to extract encoded_len: {}",
                        e
                    ))
                })?;
        let enc_t = enc_shape[2] as usize;
        let enc_dim = enc_shape[1] as usize;
        let enc_len = enc_len_data.first().copied().unwrap_or(0).max(0) as usize;

        // 3. Greedy RNN-T decoding (single stream, mirrors gigaam ONNX utils)
        let decode_start = std::time::Instant::now();
        let text = self.greedy_decode(enc_data, enc_t, enc_dim, enc_len)?;
        tracing::debug!("RNNT decode: {:.2}s", decode_start.elapsed().as_secs_f32(),);

        tracing::info!(
            "GigaAM transcription completed in {:.2}s: {:?}",
            start.elapsed().as_secs_f32(),
            if text.chars().count() > 50 {
                format!("{}...", text.chars().take(50).collect::<String>())
            } else {
                text.clone()
            },
        );

        Ok(text)
    }
}

impl GigaAMTranscriber {
    fn greedy_decode(
        &self,
        enc: &[f32],
        enc_t: usize,
        enc_dim: usize,
        enc_len: usize,
    ) -> Result<String, TranscribeError> {
        let blank = self.tokens.len() as i64; // blank = vocab_size (last column)
        let mut hyp: Vec<i64> = Vec::new();

        // Prediction-network state: labels [1,1] i64, h/c [layers, 1, pred_hidden]
        let mut labels = vec![blank];
        let mut h: Vec<f32> = vec![0.0; self.pred_rnn_layers * self.pred_hidden];
        let mut c: Vec<f32> = vec![0.0; self.pred_rnn_layers * self.pred_hidden];

        for t in 0..enc_len.min(enc_t) {
            for _ in 0..MAX_LETTERS_PER_FRAME {
                let labels_tensor = Tensor::<i64>::from_array(([1usize, 1usize], labels.clone()))
                    .map_err(|e| {
                    TranscribeError::InferenceFailed(format!("labels tensor: {}", e))
                })?;
                let h_tensor = Tensor::<f32>::from_array((
                    [self.pred_rnn_layers, 1usize, self.pred_hidden],
                    h.clone(),
                ))
                .map_err(|e| TranscribeError::InferenceFailed(format!("h tensor: {}", e)))?;
                let c_tensor = Tensor::<f32>::from_array((
                    [self.pred_rnn_layers, 1usize, self.pred_hidden],
                    c.clone(),
                ))
                .map_err(|e| TranscribeError::InferenceFailed(format!("c tensor: {}", e)))?;

                let dec_inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
                    (std::borrow::Cow::Borrowed("x"), labels_tensor.into()),
                    (std::borrow::Cow::Borrowed("hi"), h_tensor.into()),
                    (std::borrow::Cow::Borrowed("ci"), c_tensor.into()),
                ];
                let mut decoder = self.decoder.lock().map_err(|e| {
                    TranscribeError::InferenceFailed(format!("Failed to lock decoder: {}", e))
                })?;
                let dec_out = decoder.run(dec_inputs).map_err(|e| {
                    TranscribeError::InferenceFailed(format!("GigaAM decoder failed: {}", e))
                })?;

                // dec: [1, 1, pred_hidden] -> joint wants [1, pred_hidden, 1]
                let (dec_shape, dec_data) = dec_out["dec"]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| TranscribeError::InferenceFailed(format!("decoder out: {}", e)))?;
                let pred_hidden = dec_shape.last().copied().unwrap_or(0) as usize;
                let mut dec_t = vec![0.0f32; pred_hidden];
                dec_t.copy_from_slice(&dec_data[..pred_hidden]);
                let dec_tensor = Tensor::<f32>::from_array(([1usize, pred_hidden, 1usize], dec_t))
                    .map_err(|e| TranscribeError::InferenceFailed(format!("dec tensor: {}", e)))?;

                // enc frame slice [1, enc_dim, 1]: encoded is [B, D, T']
                // row-major, so frame t's values sit at d*T'+t (strided)
                let mut frame = Vec::with_capacity(enc_dim);
                for d in 0..enc_dim {
                    frame.push(enc[d * enc_t + t]);
                }
                let enc_tensor = Tensor::<f32>::from_array(([1usize, enc_dim, 1usize], frame))
                    .map_err(|e| {
                        TranscribeError::InferenceFailed(format!("enc frame tensor: {}", e))
                    })?;

                let joint_inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
                    (std::borrow::Cow::Borrowed("enc"), enc_tensor.into()),
                    (std::borrow::Cow::Borrowed("dec"), dec_tensor.into()),
                ];
                let mut joint = self.joint.lock().map_err(|e| {
                    TranscribeError::InferenceFailed(format!("Failed to lock joint: {}", e))
                })?;
                let joint_out = joint.run(joint_inputs).map_err(|e| {
                    TranscribeError::InferenceFailed(format!("GigaAM joint failed: {}", e))
                })?;

                let (_, joint_data) = joint_out["joint"]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| TranscribeError::InferenceFailed(format!("joint out: {}", e)))?;
                let k = argmax(joint_data);

                if k as i64 == blank {
                    // blank: advance to the next encoder frame, prediction
                    // state and last label persist across frames
                    // (mirrors upstream gigaam decoding)
                    break;
                }

                hyp.push(k as i64);
                labels = vec![k as i64];

                // refresh LSTM state from decoder outputs (ho/co)
                let (_, ho) = dec_out["ho"]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| TranscribeError::InferenceFailed(format!("ho out: {}", e)))?;
                let (_, co) = dec_out["co"]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| TranscribeError::InferenceFailed(format!("co out: {}", e)))?;
                h = ho.to_vec();
                c = co.to_vec();
            }
        }

        Ok(pieces_to_text(&hyp, &self.tokens))
    }
}

fn argmax(data: &[f32]) -> usize {
    let mut best = 0usize;
    let mut best_val = f32::NEG_INFINITY;
    for (i, &v) in data.iter().enumerate() {
        if v > best_val {
            best_val = v;
            best = i;
        }
    }
    best
}

/// Join SentencePiece pieces into text: `▁` marks word boundaries.
fn pieces_to_text(ids: &[i64], tokens: &[String]) -> String {
    let mut out = String::new();
    for &id in ids {
        if let Some(piece) = tokens.get(id as usize) {
            if piece == "<unk>" {
                continue;
            }
            out.push_str(&piece.replace('\u{2581}', " "));
        }
    }
    out.trim().to_string()
}

/// Resolve model name to directory path
fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    let model_dir_name = if model.starts_with("gigaam-") {
        model.to_string()
    } else {
        format!("gigaam-{}", model)
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

    Err(TranscribeError::ModelNotFound(format!(
        "GigaAM model not found: {}\n  \
         Looked in: {:?} and {:?}\n  \
         Export the ONNX files with scripts/export_gigaam_onnx.py",
        model, model_path, alt_path
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pieces_to_text() {
        let tokens = vec![
            "\u{2581}привет".to_string(),
            ",".to_string(),
            "\u{2581}мир".to_string(),
        ];
        assert_eq!(pieces_to_text(&[0, 1, 2], &tokens), "привет, мир");
    }

    #[test]
    fn test_pieces_unk_skipped() {
        let tokens = vec!["<unk>".to_string(), "\u{2581}а".to_string()];
        assert_eq!(pieces_to_text(&[0, 1, 1], &tokens), "а а");
    }
}
