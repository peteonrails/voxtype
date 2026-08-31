//! GigaAM v3 RNN-T speech-to-text (Russian specialist).
//!
//! In-tree ONNX Runtime engine: 64-mel HTK frontend + Conformer encoder +
//! LSTM prediction network + joiner, greedy decode. Model files come from
//! the gigastt INT8 bundle (GitHub Release `models-v3-2026-06-22`).
//!
//! Pipeline: Audio (f32, 16kHz) -> log-mel [64, T] -> encoder -> greedy RNN-T

use super::onnx_ep;
use super::Transcriber;
use crate::config::GigaamConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::f32::consts::PI;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const SAMPLE_RATE: usize = 16000;
const N_MELS: usize = 64;
const N_FFT: usize = 320;
const HOP_LENGTH: usize = 160;
const PRED_HIDDEN: usize = 320;
const ENC_DIM: usize = 768;
const MAX_TOKENS_PER_STEP: usize = 10;
const WORD_BOUNDARY: char = '\u{2581}';

const ENC_IN_AUDIO: &str = "audio_signal";
const ENC_IN_LENGTH: &str = "length";
const ENC_OUT_ENCODED: &str = "encoded";
const ENC_OUT_LEN: &str = "encoded_len";
const DEC_IN_X: &str = "x";
const DEC_IN_H: &str = "h.1";
const DEC_IN_C: &str = "c.1";
const DEC_OUT_DEC: &str = "dec";
const DEC_OUT_H: &str = "h";
const DEC_OUT_C: &str = "c";
const JOIN_IN_ENC: &str = "enc";
const JOIN_IN_DEC: &str = "dec";
const JOIN_OUT: &str = "joint";

const ENCODER_FILE: &str = "v3_rnnt_encoder_int8.onnx";
const DECODER_FILE: &str = "v3_rnnt_decoder.onnx";
const JOINT_FILE: &str = "v3_rnnt_joint.onnx";
const VOCAB_FILE: &str = "v3_vocab.txt";

struct SparseMelBand {
    start: usize,
    weights: Vec<f32>,
}

struct MelSpectrogram {
    n_fft: usize,
    hop_length: usize,
    window: Vec<f32>,
    mel_bands: Vec<SparseMelBand>,
    fft: Arc<dyn Fft<f32>>,
}

impl MelSpectrogram {
    fn new() -> Self {
        let n_fft = N_FFT;
        let n_mels = N_MELS;
        let sample_rate = SAMPLE_RATE as f32;
        let window: Vec<f32> = (0..n_fft)
            .map(|n| 0.5 * (1.0 - (2.0 * PI * n as f32 / (n_fft - 1) as f32).cos()))
            .collect();
        let filterbank = create_htk_filterbank(n_fft, n_mels, sample_rate);
        let n_freqs = n_fft / 2 + 1;
        let mel_bands = sparsify_mel_filterbank(&filterbank, n_mels, n_freqs);
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n_fft);
        Self {
            n_fft,
            hop_length: HOP_LENGTH,
            window,
            mel_bands,
            fft,
        }
    }

    /// Log-mel spectrogram, channels-first [n_mels, num_frames] as a flat Vec.
    fn compute(&self, samples: &[f32]) -> (Vec<f32>, usize) {
        let n_freqs = self.n_fft / 2 + 1;
        let n_mels = self.mel_bands.len();
        if samples.len() < self.n_fft {
            return (vec![0.0; n_mels], 1);
        }
        let num_frames = (samples.len() - self.n_fft) / self.hop_length + 1;
        let mut output = vec![0.0_f32; n_mels * num_frames];
        let mut fft_input = vec![Complex::new(0.0_f32, 0.0); self.n_fft];
        let mut power = vec![0.0_f32; n_freqs];

        for frame_idx in 0..num_frames {
            let start = frame_idx * self.hop_length;
            for (i, (slot, window)) in fft_input.iter_mut().zip(&self.window).enumerate() {
                let sample = samples.get(start + i).copied().unwrap_or(0.0);
                *slot = Complex::new(sample * window, 0.0);
            }
            self.fft.process(&mut fft_input[..self.n_fft]);
            for (slot, bin) in power.iter_mut().zip(fft_input.iter()) {
                *slot = bin.norm_sqr();
            }
            for (m, band) in self.mel_bands.iter().enumerate() {
                let mut energy = 0.0_f32;
                for (i, &w) in band.weights.iter().enumerate() {
                    energy += w * power[band.start + i];
                }
                output[m * num_frames + frame_idx] = energy.max(1e-10).ln();
            }
        }
        (output, num_frames)
    }
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0)
}

fn create_htk_filterbank(n_fft: usize, n_mels: usize, sample_rate: f32) -> Vec<f32> {
    let n_freqs = n_fft / 2 + 1;
    let mel_min = hz_to_mel(0.0);
    let mel_max = hz_to_mel(sample_rate / 2.0);
    let mel_points: Vec<f32> = (0..=(n_mels + 1))
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (n_mels + 1) as f32)
        .collect();
    let bin_points: Vec<f32> = mel_points
        .iter()
        .map(|&m| mel_to_hz(m) * n_fft as f32 / sample_rate)
        .collect();
    let mut filterbank = vec![0.0_f32; n_mels * n_freqs];
    for m in 0..n_mels {
        let f_left = bin_points[m];
        let f_center = bin_points[m + 1];
        let f_right = bin_points[m + 2];
        let row_start = m * n_freqs;
        for k in 0..n_freqs {
            let freq = k as f32;
            filterbank[row_start + k] = if freq >= f_left && freq <= f_center && f_center > f_left {
                (freq - f_left) / (f_center - f_left)
            } else if freq > f_center && freq <= f_right && f_right > f_center {
                (f_right - freq) / (f_right - f_center)
            } else {
                0.0
            };
        }
    }
    filterbank
}

fn sparsify_mel_filterbank(
    filterbank: &[f32],
    n_mels: usize,
    n_freqs: usize,
) -> Vec<SparseMelBand> {
    let mut bands = Vec::with_capacity(n_mels);
    for m in 0..n_mels {
        let row = &filterbank[m * n_freqs..(m + 1) * n_freqs];
        match (
            row.iter().position(|&w| w != 0.0),
            row.iter().rposition(|&w| w != 0.0),
        ) {
            (Some(start), Some(end)) => bands.push(SparseMelBand {
                start,
                weights: row[start..=end].to_vec(),
            }),
            _ => bands.push(SparseMelBand {
                start: 0,
                weights: vec![0.0],
            }),
        }
    }
    bands
}

struct Tokenizer {
    tokens: Vec<String>,
    blank_id: usize,
}

impl Tokenizer {
    fn load(path: &std::path::Path) -> Result<Self, TranscribeError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            TranscribeError::ModelNotFound(format!(
                "failed to read vocab {}: {}",
                path.display(),
                e
            ))
        })?;
        let mut tokens = Vec::new();
        for line in content.lines() {
            if line.is_empty() || line.parse::<usize>().is_ok() {
                continue;
            }
            let token = if let Some(pos) = line.rfind(['\t', ' ']) {
                let after = line[pos + 1..].trim();
                if after.parse::<usize>().is_ok() {
                    line[..pos].to_string()
                } else {
                    line.to_string()
                }
            } else {
                line.to_string()
            };
            tokens.push(token);
        }
        if tokens.is_empty() {
            return Err(TranscribeError::InitFailed(format!(
                "vocabulary file is empty: {}",
                path.display()
            )));
        }
        let blank_id = tokens
            .iter()
            .position(|t| t == "<blk>")
            .unwrap_or_else(|| tokens.len().saturating_sub(1));
        Ok(Self { tokens, blank_id })
    }

    fn decode(&self, ids: &[usize]) -> String {
        let mut text = String::new();
        for &id in ids {
            if id == self.blank_id || id >= self.tokens.len() {
                continue;
            }
            let token = &self.tokens[id];
            if token == "<unk>" {
                continue;
            }
            text.push_str(token);
        }
        text.replace(WORD_BOUNDARY, " ").trim().to_string()
    }
}

struct DecoderState {
    h: Vec<f32>,
    c: Vec<f32>,
    prev_token: i64,
}

impl DecoderState {
    fn new(blank_id: usize) -> Self {
        Self {
            h: vec![0.0; PRED_HIDDEN],
            c: vec![0.0; PRED_HIDDEN],
            prev_token: blank_id as i64,
        }
    }
}

fn extract_encoder_frame(encoded: &[f32], encoded_len: usize, t: usize, enc_frame: &mut [f32]) {
    for (ch, slot) in enc_frame.iter_mut().enumerate() {
        *slot = encoded[ch * encoded_len + t];
    }
}

fn argmax(logits: &[f32], blank_id: usize) -> usize {
    logits
        .iter()
        .enumerate()
        .max_by(|(_i, a), (_j, b)| a.total_cmp(b))
        .map(|(idx, _)| idx)
        .unwrap_or(blank_id)
}

struct Sessions {
    encoder: Session,
    decoder: Session,
    joiner: Session,
}

/// GigaAM v3 RNN-T transcriber.
pub struct GigaamTranscriber {
    sessions: Mutex<Sessions>,
    tokenizer: Tokenizer,
    mel: MelSpectrogram,
}

impl GigaamTranscriber {
    pub fn new(config: &GigaamConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;
        tracing::info!("Loading GigaAM v3 RNN-T from {:?}", model_dir);
        let start = std::time::Instant::now();

        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));
        let encoder_path = model_dir.join(ENCODER_FILE);
        let decoder_path = model_dir.join(DECODER_FILE);
        let joint_path = model_dir.join(JOINT_FILE);
        let vocab_path = model_dir.join(VOCAB_FILE);
        for path in [&encoder_path, &decoder_path, &joint_path, &vocab_path] {
            if !path.exists() {
                return Err(TranscribeError::ModelNotFound(format!(
                    "GigaAM model file missing: {}\n  \
                     Expected {ENCODER_FILE}, {DECODER_FILE}, {JOINT_FILE}, {VOCAB_FILE}\n  \
                     Copy the gigastt INT8 bundle (GitHub Release models-v3-2026-06-22) \
                     into this directory, or wait for models.voxtype.io to host it.",
                    path.display()
                )));
            }
        }

        let tokenizer = Tokenizer::load(&vocab_path)?;
        let encoder = load_session(&encoder_path, threads, "encoder")?;
        let decoder = load_session(&decoder_path, 1, "decoder")?;
        let joiner = load_session(&joint_path, 1, "joiner")?;

        tracing::info!(
            "GigaAM v3 RNN-T loaded in {:.2}s (vocab={}, blank_id={})",
            start.elapsed().as_secs_f32(),
            tokenizer.tokens.len(),
            tokenizer.blank_id,
        );

        Ok(Self {
            sessions: Mutex::new(Sessions {
                encoder,
                decoder,
                joiner,
            }),
            tokenizer,
            mel: MelSpectrogram::new(),
        })
    }

    fn greedy_decode(
        &self,
        sessions: &mut Sessions,
        encoded: &[f32],
        encoded_len: usize,
    ) -> Result<Vec<usize>, TranscribeError> {
        let blank_id = self.tokenizer.blank_id;
        let mut state = DecoderState::new(blank_id);
        let mut tokens = Vec::new();
        let mut enc_frame = vec![0.0_f32; ENC_DIM];
        let mut dec_out = vec![0.0_f32; PRED_HIDDEN];
        let mut new_h = vec![0.0_f32; PRED_HIDDEN];
        let mut new_c = vec![0.0_f32; PRED_HIDDEN];
        let mut cache_valid = false;
        let mut in_blank_run = false;

        if encoded.len() < ENC_DIM * encoded_len {
            return Err(TranscribeError::InferenceFailed(format!(
                "encoder output size mismatch: got {}, expected >= {}",
                encoded.len(),
                ENC_DIM * encoded_len
            )));
        }

        for t in 0..encoded_len {
            let mut tokens_this_step = 0;
            extract_encoder_frame(encoded, encoded_len, t, &mut enc_frame);
            loop {
                if !in_blank_run {
                    run_decoder(sessions, &state, &mut dec_out, &mut new_h, &mut new_c)?;
                    cache_valid = true;
                } else if !cache_valid {
                    return Err(TranscribeError::InferenceFailed(
                        "blank-run decoder cache is stale".to_string(),
                    ));
                }

                let logits = run_joiner(sessions, &enc_frame, &dec_out)?;
                let token = argmax(&logits, blank_id);

                if token == blank_id {
                    in_blank_run = true;
                    break;
                }
                if tokens_this_step >= MAX_TOKENS_PER_STEP {
                    in_blank_run = false;
                    cache_valid = false;
                    break;
                }

                in_blank_run = false;
                state.prev_token = token as i64;
                state.h.copy_from_slice(&new_h);
                state.c.copy_from_slice(&new_c);
                tokens.push(token);
                tokens_this_step += 1;
            }
        }
        Ok(tokens)
    }
}

fn load_session(
    path: &std::path::Path,
    threads: usize,
    label: &str,
) -> Result<Session, TranscribeError> {
    let builder = Session::builder()
        .map_err(|e| TranscribeError::InitFailed(format!("ONNX session builder failed: {e}")))?;
    let builder = onnx_ep::register_gpu_eps(builder, "GigaAM", label)
        .map_err(|e| TranscribeError::InitFailed(format!("Failed to register EPs: {e}")))?;
    builder
        .with_intra_threads(threads.max(1))
        .map_err(|e| TranscribeError::InitFailed(format!("Failed to set threads: {e}")))?
        .commit_from_file(path)
        .map_err(|e| {
            TranscribeError::InitFailed(format!(
                "Failed to load GigaAM {label} from {}: {e}",
                path.display()
            ))
        })
}

fn run_decoder(
    sessions: &mut Sessions,
    state: &DecoderState,
    dec_out: &mut [f32],
    new_h: &mut [f32],
    new_c: &mut [f32],
) -> Result<(), TranscribeError> {
    let x = Tensor::<i64>::from_array(([1usize, 1], vec![state.prev_token]))
        .map_err(|e| TranscribeError::InferenceFailed(format!("decoder token tensor: {e}")))?;
    let h = Tensor::<f32>::from_array(([1usize, 1, PRED_HIDDEN], state.h.clone()))
        .map_err(|e| TranscribeError::InferenceFailed(format!("decoder h tensor: {e}")))?;
    let c = Tensor::<f32>::from_array(([1usize, 1, PRED_HIDDEN], state.c.clone()))
        .map_err(|e| TranscribeError::InferenceFailed(format!("decoder c tensor: {e}")))?;
    let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
        (std::borrow::Cow::Borrowed(DEC_IN_X), x.into()),
        (std::borrow::Cow::Borrowed(DEC_IN_H), h.into()),
        (std::borrow::Cow::Borrowed(DEC_IN_C), c.into()),
    ];
    let outputs = sessions
        .decoder
        .run(inputs)
        .map_err(|e| TranscribeError::InferenceFailed(format!("GigaAM decoder failed: {e}")))?;
    copy_named_f32(&outputs, DEC_OUT_DEC, dec_out)?;
    copy_named_f32(&outputs, DEC_OUT_H, new_h)?;
    copy_named_f32(&outputs, DEC_OUT_C, new_c)?;
    Ok(())
}

fn run_joiner(
    sessions: &mut Sessions,
    enc_frame: &[f32],
    dec_data: &[f32],
) -> Result<Vec<f32>, TranscribeError> {
    let enc = Tensor::<f32>::from_array(([1usize, ENC_DIM, 1], enc_frame.to_vec()))
        .map_err(|e| TranscribeError::InferenceFailed(format!("joiner enc tensor: {e}")))?;
    let dec = Tensor::<f32>::from_array(([1usize, PRED_HIDDEN, 1], dec_data.to_vec()))
        .map_err(|e| TranscribeError::InferenceFailed(format!("joiner dec tensor: {e}")))?;
    let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
        (std::borrow::Cow::Borrowed(JOIN_IN_ENC), enc.into()),
        (std::borrow::Cow::Borrowed(JOIN_IN_DEC), dec.into()),
    ];
    let outputs = sessions
        .joiner
        .run(inputs)
        .map_err(|e| TranscribeError::InferenceFailed(format!("GigaAM joiner failed: {e}")))?;
    let val = &outputs[JOIN_OUT];
    let (_shape, data) = val
        .try_extract_tensor::<f32>()
        .map_err(|e| TranscribeError::InferenceFailed(format!("extract joiner logits: {e}")))?;
    Ok(data.to_vec())
}

fn copy_named_f32(
    outputs: &ort::session::SessionOutputs<'_>,
    name: &str,
    dest: &mut [f32],
) -> Result<(), TranscribeError> {
    let val = &outputs[name];
    let (_shape, data) = val
        .try_extract_tensor::<f32>()
        .map_err(|e| TranscribeError::InferenceFailed(format!("extract {name}: {e}")))?;
    if data.len() != dest.len() {
        return Err(TranscribeError::InferenceFailed(format!(
            "{name} size mismatch: got {}, expected {}",
            data.len(),
            dest.len()
        )));
    }
    dest.copy_from_slice(data);
    Ok(())
}

impl Transcriber for GigaamTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("empty audio buffer".into()));
        }
        let start = std::time::Instant::now();
        let (features, num_frames) = self.mel.compute(samples);
        let mut sessions = self.sessions.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("failed to lock GigaAM sessions: {e}"))
        })?;

        let audio = Tensor::<f32>::from_array(([1usize, N_MELS, num_frames], features))
            .map_err(|e| TranscribeError::InferenceFailed(format!("encoder audio tensor: {e}")))?;
        let length = Tensor::<i64>::from_array(([1usize], vec![num_frames as i64]))
            .map_err(|e| TranscribeError::InferenceFailed(format!("encoder length tensor: {e}")))?;
        let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
            (std::borrow::Cow::Borrowed(ENC_IN_AUDIO), audio.into()),
            (std::borrow::Cow::Borrowed(ENC_IN_LENGTH), length.into()),
        ];
        let outputs = sessions
            .encoder
            .run(inputs)
            .map_err(|e| TranscribeError::InferenceFailed(format!("GigaAM encoder failed: {e}")))?;

        let encoded_val = &outputs[ENC_OUT_ENCODED];
        let (_enc_shape, encoded_view) = encoded_val
            .try_extract_tensor::<f32>()
            .map_err(|e| TranscribeError::InferenceFailed(format!("extract encoded: {e}")))?;
        let len_val = &outputs[ENC_OUT_LEN];
        let encoded_len = if let Ok((_, data)) = len_val.try_extract_tensor::<i32>() {
            data.first().copied().unwrap_or(0).max(0) as usize
        } else {
            let (_, data) = len_val.try_extract_tensor::<i64>().map_err(|e| {
                TranscribeError::InferenceFailed(format!("extract encoded_len: {e}"))
            })?;
            data.first().copied().unwrap_or(0).max(0) as usize
        };
        let encoded = encoded_view.to_vec();
        drop(outputs);

        tracing::debug!(
            encoded_floats = encoded.len(),
            encoded_len,
            "GigaAM encoder output"
        );

        let tokens = self.greedy_decode(&mut sessions, &encoded, encoded_len)?;
        let text = self.tokenizer.decode(&tokens);
        tracing::info!(
            "GigaAM transcription completed in {:.2}s: {:?}",
            start.elapsed().as_secs_f32(),
            if text.chars().count() > 50 {
                format!("{}...", text.chars().take(50).collect::<String>())
            } else {
                text.clone()
            }
        );
        Ok(text)
    }

    fn last_detected_language(&self) -> Option<String> {
        Some("ru".to_string())
    }
}

fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }
    let models_dir = crate::config::Config::models_dir();
    let candidates = [
        models_dir.join(model),
        PathBuf::from(model),
        PathBuf::from("models").join(model),
    ];
    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }
    Err(TranscribeError::ModelNotFound(format!(
        "GigaAM model '{}' not found. Looked in:\n  \
         - {}\n  \
         - {}\n  \
         - {}\n\n\
         Place the INT8 bundle (v3_rnnt_encoder_int8.onnx, v3_rnnt_decoder.onnx, \
         v3_rnnt_joint.onnx, v3_vocab.txt) from \
         https://github.com/ekhodzitsky/gigastt/releases/tag/models-v3-2026-06-22 \
         into {}.",
        model,
        candidates[0].display(),
        candidates[1].display(),
        candidates[2].display(),
        models_dir.join("gigaam-v3-rnnt-int8").display(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argmax_picks_the_largest() {
        assert_eq!(argmax(&[0.1, 4.0, 2.0], 0), 1);
        assert_eq!(argmax(&[], 7), 7);
    }

    #[test]
    fn extract_encoder_frame_is_channels_first() {
        // encoded layout [ENC_DIM, T] with T=2, ENC_DIM=3 for the test.
        let encoded = vec![
            1.0, 2.0, // ch 0
            3.0, 4.0, // ch 1
            5.0, 6.0, // ch 2
        ];
        let mut frame = vec![0.0; 3];
        extract_encoder_frame(&encoded, 2, 1, &mut frame);
        assert_eq!(frame, vec![2.0, 4.0, 6.0]);
    }

    #[test]
    fn tokenizer_decodes_char_vocab() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("v3_vocab.txt");
        std::fs::write(&path, "▁ 0\nа 1\nб 2\n<blk> 3\n").unwrap();
        let tok = Tokenizer::load(&path).unwrap();
        assert_eq!(tok.blank_id, 3);
        assert_eq!(tok.decode(&[0, 1, 2]), "аб");
    }

    #[test]
    fn mel_one_second_of_silence_has_expected_frames() {
        let mel = MelSpectrogram::new();
        let audio = vec![0.0f32; 16000];
        let (feats, frames) = mel.compute(&audio);
        // (16000 - 320) / 160 + 1 = 99
        assert_eq!(frames, 99);
        assert_eq!(feats.len(), N_MELS * 99);
    }

    #[test]
    fn resolve_model_path_reports_missing() {
        let err = resolve_model_path("/nonexistent/gigaam").unwrap_err();
        assert!(matches!(err, TranscribeError::ModelNotFound(_)));
    }
}
