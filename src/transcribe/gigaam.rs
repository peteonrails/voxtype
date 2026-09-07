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
use super::streaming::{SegmentId, StreamHandle, StreamingEvent, StreamingTranscriber};
use super::Transcriber;
use crate::config::GigaAMConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

/// Sample rate expected by GigaAM
const SAMPLE_RATE: usize = 16000;

/// Max tokens emitted per encoder frame, matching upstream gigaam ONNX decoding.
const MAX_LETTERS_PER_FRAME: usize = 3;

/// Loaded model state, shared with streaming sessions.
///
/// Split out from [`GigaAMTranscriber`] because a streaming session
/// outlives the `&self` borrow `start_stream` is handed: the spawned task
/// needs an owned handle on the sessions.
struct GigaAMInner {
    encoder: std::sync::Mutex<Session>,
    decoder: std::sync::Mutex<Session>,
    joint: std::sync::Mutex<Session>,
    tokens: Vec<String>,
    mel_extractor: GigaAMMelExtractor,
    pred_hidden: usize,
    pred_rnn_layers: usize,
}

/// GigaAM-based transcriber using ONNX Runtime
pub struct GigaAMTranscriber {
    inner: Arc<GigaAMInner>,
    /// Whether `[gigaam] streaming` opted this transcriber into the
    /// streaming pipeline. Gates [`Transcriber::as_streaming`].
    streaming: bool,
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
            inner: Arc::new(GigaAMInner {
                encoder: std::sync::Mutex::new(encoder),
                decoder: std::sync::Mutex::new(decoder),
                joint: std::sync::Mutex::new(joint),
                tokens,
                mel_extractor,
                pred_hidden,
                pred_rnn_layers,
            }),
            streaming: config.streaming,
        })
    }
}

/// Longest clip handed to the encoder. Upstream GigaAM refuses anything
/// past 25 s outright (`gigaam/model.py`: "Too long wav file, use
/// 'transcribe_longform' method."); 20 s leaves room for the padding
/// `split_on_silence` adds around each segment.
/// ponytail: fixed cap rather than a config knob — lift it into
/// GigaAMConfig if a later GigaAM revision moves the limit.
const MAX_SEGMENT_SECS: f32 = 20.0;

/// Pause long enough to call a phrase finished. Shorter chops mid-sentence,
/// longer lets segments drift toward the cap.
const MIN_SILENCE_MS: u32 = 400;

/// How long a streaming segment may grow before it is committed anyway.
/// Much shorter than the batch cap: while streaming, the wait for a cut is
/// the latency the user actually feels. The cut lands on the quietest frame
/// in the window, so it usually falls at a word boundary rather than
/// mid-syllable.
const STREAM_SEGMENT_SECS: f32 = 8.0;

/// How often the uncommitted tail is re-transcribed to preview it on the OSD.
/// Only a floor — the real interval backs off to twice the measured cost, so
/// a slow machine previews less often instead of falling behind the speaker.
const PARTIAL_EVERY: Duration = Duration::from_millis(500);

/// Longest tail re-transcribed for a preview. The whole window is re-encoded
/// every time (the export is full-utterance, there is no cache to extend), so
/// this bounds the cost of a preview no matter how long the phrase runs.
const PARTIAL_WINDOW_SECS: usize = 6;

impl GigaAMInner {
    /// Transcribe one segment: log-mel -> encoder -> greedy RNN-T decode.
    ///
    /// Callers must keep segments under [`MAX_SEGMENT_SECS`]; see
    /// [`Transcriber::transcribe`] for the segmentation that guarantees it.
    fn transcribe_segment(&self, samples: &[f32]) -> Result<String, TranscribeError> {
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

impl GigaAMInner {
    /// Transcribe a recording, splitting it on pauses first.
    ///
    /// GigaAM is a conformer: quality falls off on long audio, and upstream
    /// refuses clips over 25 s rather than degrade quietly — its
    /// `transcribe_longform` is VAD segmentation for exactly this reason.
    /// So the buffer is cut on pauses and transcribed piece by piece, then
    /// joined. Short dictations come back as a single segment with the
    /// leading and trailing silence trimmed.
    fn transcribe_all(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        let floor = crate::vad::noise_floor(&crate::vad::frame_rms(samples));
        self.transcribe_split(samples, floor, MAX_SEGMENT_SECS)
    }

    /// [`Self::transcribe_all`] with the noise floor and cap supplied.
    ///
    /// Streaming passes its session-wide floor here: the leftover tail it
    /// flushes at the end is far too short to estimate one from.
    fn transcribe_split(
        &self,
        samples: &[f32],
        floor: f32,
        max_secs: f32,
    ) -> Result<String, TranscribeError> {
        let segments = crate::vad::split_on_silence_at(samples, floor, max_secs, MIN_SILENCE_MS);

        // Nothing crossed the speech threshold. Hand the buffer over whole
        // rather than silently return nothing — the caller's VAD gate, not
        // this splitter, decides what counts as an empty recording.
        if segments.is_empty() {
            return self.transcribe_segment(samples);
        }

        let mut parts: Vec<String> = Vec::with_capacity(segments.len());
        for range in &segments {
            let text = self.transcribe_segment(&samples[range.clone()])?;
            let text = text.trim();
            if !text.is_empty() {
                parts.push(text.to_string());
            }
        }

        tracing::debug!(
            "GigaAM: {} segment(s) over {:.1}s of audio",
            segments.len(),
            samples.len() as f32 / SAMPLE_RATE as f32,
        );

        Ok(parts.join(" "))
    }
}

impl Transcriber for GigaAMTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        self.inner.transcribe_all(samples)
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        self.streaming.then_some(self as &dyn StreamingTranscriber)
    }
}

impl StreamingTranscriber for GigaAMTranscriber {
    /// Stream by committing whole phrases.
    ///
    /// GigaAM's ONNX export is full-utterance — there is no cache-aware
    /// encoder to feed frame by frame — so instead of faking partials this
    /// leans on the pause splitting the batch path already needs: as soon as
    /// a pause proves a phrase is over, that phrase is transcribed and sent
    /// as `Final`, which is what the output layer types. The tail keeps
    /// growing until the next pause, and the hard cap bounds the wait when
    /// someone talks without pausing at all.
    ///
    /// Only `Final` events are emitted: a full-utterance model has no
    /// meaningful partial to show, and typing a guess would only churn.
    fn start_stream(
        &self,
        mut samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        let inner = Arc::clone(&self.inner);
        let (events_tx, events) = mpsc::channel(32);
        let (cancel, mut cancel_rx) = oneshot::channel();

        let task = tokio::spawn(async move {
            let mut pending: Vec<f32> = Vec::new();
            // Frame energies for the whole session. The floor must be measured
            // over everything heard so far, not over `pending`: after a commit
            // that buffer is a short, speech-dense tail whose 5th percentile
            // sits inside speech, and the threshold derived from it shreds the
            // next phrase into fragments that transcribe to nothing.
            let mut session_rms: Vec<f32> = Vec::new();
            let mut segment_id: SegmentId = 0;
            let mut cancelled = false;
            let mut last_partial = Instant::now();
            let mut partial_cost = Duration::ZERO;
            // Segments are decoded independently, so nothing carries the space
            // that separates one from the last. The output layer types what it
            // is given, verbatim.
            let mut committed_any = false;

            loop {
                tokio::select! {
                    _ = &mut cancel_rx => {
                        cancelled = true;
                        break;
                    }
                    chunk = samples_rx.recv() => {
                        let Some(chunk) = chunk else { break }; // sender dropped: EOF
                        session_rms.extend(crate::vad::frame_rms(&chunk));
                        pending.extend_from_slice(&chunk);
                        let floor = crate::vad::noise_floor(&session_rms);

                        // Every segment but the last has a pause after it, so
                        // it can never be revised by audio still to come.
                        // ponytail: re-splits the whole pending buffer per
                        // chunk. The sort is over ~50 frames per second of
                        // pending audio, so microseconds; revisit only if
                        // the tail is ever allowed to grow unbounded.
                        let segments = crate::vad::split_on_silence_at(
                            &pending,
                            floor,
                            STREAM_SEGMENT_SECS,
                            MIN_SILENCE_MS,
                        );
                        let Some(tail_start) = segments.last().map(|r| r.start) else {
                            // Silence only so far. Keep a short tail in case a
                            // word starts at its edge, and drop the rest: an
                            // idle session must not grow the buffer forever.
                            let cap = STREAM_SEGMENT_SECS as usize * SAMPLE_RATE;
                            if pending.len() > cap {
                                let keep = pending.len() - SAMPLE_RATE;
                                pending.drain(..keep);
                            }
                            continue;
                        };
                        for range in &segments[..segments.len() - 1] {
                            let audio = pending[range.clone()].to_vec();
                            let model = Arc::clone(&inner);
                            // ONNX inference is blocking and long enough to
                            // stall the runtime. Audio keeps arriving on the
                            // channel meanwhile.
                            let done = tokio::task::spawn_blocking(move || {
                                model.transcribe_segment(&audio)
                            })
                            .await;
                            match done {
                                Ok(Ok(text)) if !text.trim().is_empty() => {
                                    let text = separated(text.trim(), committed_any);
                                    committed_any = true;
                                    let event = StreamingEvent::Final { text, segment_id };
                                    segment_id += 1;
                                    if events_tx.send(event).await.is_err() {
                                        return Ok(());
                                    }
                                }
                                Ok(Ok(_)) => {}
                                Ok(Err(e)) => {
                                    let _ = events_tx.send(StreamingEvent::Error(e)).await;
                                }
                                Err(e) => {
                                    let _ = events_tx
                                        .send(StreamingEvent::Error(
                                            TranscribeError::InferenceFailed(format!(
                                                "GigaAM streaming task failed: {}",
                                                e
                                            )),
                                        ))
                                        .await;
                                }
                            }
                        }
                        if !segments[..segments.len() - 1].is_empty() {
                            // Those words are at the cursor now; the preview
                            // must not keep showing them.
                            crate::osd::partial::clear();
                        }
                        pending.drain(..tail_start);

                        // Sliding-window preview. The tail is re-transcribed
                        // whole — a full-utterance export has no cache to
                        // extend — so this is deliberately rate-limited and
                        // capped, and it never leaves the OSD: the daemon
                        // types Partial events, and these are cumulative
                        // revisions, not the deltas that contract expects.
                        if last_partial.elapsed() >= PARTIAL_EVERY.max(partial_cost * 2)
                            && pending.len() >= SAMPLE_RATE / 2
                        {
                            let from = pending
                                .len()
                                .saturating_sub(PARTIAL_WINDOW_SECS * SAMPLE_RATE);
                            let audio = pending[from..].to_vec();
                            let model = Arc::clone(&inner);
                            let started = Instant::now();
                            if let Ok(Ok(text)) =
                                tokio::task::spawn_blocking(move || model.transcribe_segment(&audio))
                                    .await
                            {
                                crate::osd::partial::publish(text.trim());
                            }
                            partial_cost = started.elapsed();
                            last_partial = Instant::now();
                        }
                    }
                }
            }

            // Flush what is left: on a clean end that is the last phrase, and
            // it is the only thing standing between the user and their words.
            // A cancel discards it — the daemon rewinds typed text on cancel.
            if !cancelled && !pending.is_empty() {
                let model = Arc::clone(&inner);
                let floor = crate::vad::noise_floor(&session_rms);
                if let Ok(Ok(text)) = tokio::task::spawn_blocking(move || {
                    model.transcribe_split(&pending, floor, STREAM_SEGMENT_SECS)
                })
                .await
                {
                    if !text.trim().is_empty() {
                        let _ = events_tx
                            .send(StreamingEvent::Final {
                                text: separated(text.trim(), committed_any),
                                segment_id,
                            })
                            .await;
                    }
                }
            }

            crate::osd::partial::clear();
            let _ = events_tx.send(StreamingEvent::Ended).await;
            Ok(())
        });

        Ok(StreamHandle {
            events,
            cancel,
            task,
        })
    }
}

impl GigaAMInner {
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

/// Prefix the separator the output layer will not add for us.
fn separated(text: &str, after_previous: bool) -> String {
    if after_previous {
        format!(" {}", text)
    } else {
        text.to_string()
    }
}
