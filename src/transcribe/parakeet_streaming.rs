//! Parakeet streaming transcriber via `parakeet-rs::ParakeetUnified`
//!
//! Wraps the cache-aware streaming pipeline so voxtype can emit live partial
//! transcripts during recording and a final transcript on hotkey release.
//!
//! Selected when `[parakeet] streaming = true`. Otherwise the batch
//! `ParakeetTranscriber` is used.
//!
//! # Architecture
//!
//! - The transcriber holds a [`ParakeetUnifiedHandle`] (shared model weights)
//!   and a `Mutex<ParakeetUnified>` for batch-mode fallback. The same handle
//!   spawns fresh streaming instances per [`start_stream`] call.
//! - For batch [`Transcriber::transcribe`] the `Mutex<ParakeetUnified>` is
//!   reset between calls so no streaming cache leaks between recordings.
//! - For [`StreamingTranscriber::start_stream`] a fresh `ParakeetUnified` is
//!   constructed from the handle and moved into the spawned drive task.
//!
//! # Partial / Final policy
//!
//! `parakeet-rs` does not surface end-of-utterance signals from the unified
//! pipeline (that lives in the multitalker / EOU model). Until we add VAD-
//! based segmentation, this implementation:
//!
//! - emits one [`StreamingEvent::Partial`] per audio chunk with the
//!   *cumulative* transcript so far;
//! - emits exactly one [`StreamingEvent::Final`] with the flushed text on
//!   stream close (samples sender dropped or cancel signal);
//! - emits exactly one [`StreamingEvent::Ended`] last.
//!
//! This gives us live OSD partials throughout the recording, with the final
//! commit happening at hotkey release. Mid-recording incremental typing
//! (commit-on-pause) is a follow-up once VAD-segmentation lands.

use super::parakeet::{build_execution_config, resolve_model_path};
use super::streaming::{StreamHandle, StreamingEvent, StreamingTranscriber};
use super::{TimedSegment, Transcriber};
use crate::config::ParakeetConfig;
use crate::error::TranscribeError;
use parakeet_rs::{ParakeetUnified, ParakeetUnifiedHandle, UnifiedStreamingConfig};
use std::sync::Mutex;
use tokio::sync::{mpsc, oneshot};

/// Streaming-capable Parakeet transcriber backed by `ParakeetUnified`.
pub struct ParakeetStreamingTranscriber {
    /// Shared model weights. Cloned cheaply for each streaming session.
    handle: ParakeetUnifiedHandle,
    /// Streaming-config snapshot used both for batch (so the handle's
    /// `from_shared_with_streaming_config` succeeds) and for spawning each
    /// streaming task.
    streaming_config: UnifiedStreamingConfig,
    /// Reusable batch instance for `Transcriber::transcribe`. Reset between
    /// calls so no inter-call state leaks.
    batch: Mutex<ParakeetUnified>,
}

impl ParakeetStreamingTranscriber {
    pub fn new(config: &ParakeetConfig) -> Result<Self, TranscribeError> {
        // Streaming-capability gate. The cache-aware streaming pipeline
        // needs both `tokenizer.model` at load time and a decoder graph
        // sized for the streaming inference loop. Models in the registry
        // that don't meet both (e.g. istupakov's TDT-v3) hit either a
        // tokenizer-not-found at load or an ONNX Gather shape error at
        // first chunk — neither is a useful diagnostic. Fail fast here
        // with a message that names the model and the known-good
        // alternative. Unknown model names (custom user directories)
        // fall through with a warning since we can't validate them.
        if crate::setup::model::is_known_parakeet_model(&config.model)
            && !crate::setup::model::is_streaming_compatible_parakeet(&config.model)
        {
            return Err(TranscribeError::InitFailed(format!(
                "Parakeet streaming is enabled but model `{}` does not support \
                 cache-aware streaming.\n\n\
                 Fix one of:\n\n  \
                 - Disable streaming in config.toml under [parakeet]:\n\n      \
                 streaming = false\n\n  \
                 - Or switch to the streaming-compatible model:\n\n      \
                 voxtype setup model {streaming_model}\n\n    \
                 and set [parakeet] model = \"{streaming_model}\" in config.toml.",
                config.model,
                streaming_model = crate::setup::model::DEFAULT_PARAKEET_STREAMING_MODEL,
            )));
        }
        if !crate::setup::model::is_known_parakeet_model(&config.model) {
            tracing::warn!(
                model = %config.model,
                "Parakeet streaming requested with a model not in the registry; \
                 cannot validate streaming compatibility ahead of time. If load \
                 fails below, switch [parakeet] model to {} or set streaming = false.",
                crate::setup::model::DEFAULT_PARAKEET_STREAMING_MODEL
            );
        }

        let model_path = resolve_model_path(&config.model)?;

        tracing::info!(
            "Loading Parakeet streaming (ParakeetUnified) model from {:?}",
            model_path
        );
        let start = std::time::Instant::now();

        let exec_config = build_execution_config();
        let handle = ParakeetUnifiedHandle::load(&model_path, exec_config).map_err(|e| {
            TranscribeError::InitFailed(format!(
                "Parakeet streaming (ParakeetUnified) init failed: {}\n\n\
                Streaming requires a TDT v3 model directory containing tokenizer.model.\n\
                If you're using a TDT v2 directory, switch to TDT v3 or set\n\
                [parakeet] streaming = false to use the batch pipeline.",
                e
            ))
        })?;

        let streaming_config = UnifiedStreamingConfig {
            chunk_secs: config.streaming_chunk_secs,
            left_context_secs: config.streaming_left_context_secs,
            right_context_secs: config.streaming_right_context_secs,
        }
        .validate()
        .map_err(|e| {
            TranscribeError::InitFailed(format!(
                "Invalid Parakeet streaming config: {}. \
                Check streaming_chunk_secs, streaming_left_context_secs, \
                streaming_right_context_secs in [parakeet].",
                e
            ))
        })?;

        let batch = ParakeetUnified::from_shared_with_streaming_config(&handle, streaming_config)
            .map_err(|e| {
            TranscribeError::InitFailed(format!(
                "Failed to spawn batch ParakeetUnified instance: {}",
                e
            ))
        })?;

        tracing::info!(
            "Parakeet streaming model loaded in {:.2}s (chunk={:.2}s, \
            left={:.2}s, right={:.2}s)",
            start.elapsed().as_secs_f32(),
            streaming_config.chunk_secs,
            streaming_config.left_context_secs,
            streaming_config.right_context_secs,
        );

        Ok(Self {
            handle,
            streaming_config,
            batch: Mutex::new(batch),
        })
    }
}

impl Transcriber for ParakeetStreamingTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let mut batch = self.batch.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock ParakeetUnified mutex: {}", e))
        })?;

        // Reset between batch calls so accumulated state from a prior
        // streaming or batch invocation doesn't bleed in.
        batch.reset();

        let result = batch
            .transcribe_audio(samples.to_vec(), 16000, 1)
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("ParakeetUnified inference failed: {}", e))
            })?;

        Ok(result.trim().to_string())
    }

    fn transcribe_timed(&self, _samples: &[f32]) -> Result<Vec<TimedSegment>, TranscribeError> {
        // ParakeetUnified does not expose timed segments in the same shape as
        // the batch ParakeetTranscriber. Users who need timed segments should
        // run with [parakeet] streaming = false.
        Err(TranscribeError::InferenceFailed(
            "transcribe_timed is not supported in streaming mode. \
            Set [parakeet] streaming = false for timed segments."
                .to_string(),
        ))
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        Some(self)
    }
}

impl StreamingTranscriber for ParakeetStreamingTranscriber {
    fn start_stream(
        &self,
        mut samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        // Spawn a fresh ParakeetUnified instance for this session.
        let mut unified =
            ParakeetUnified::from_shared_with_streaming_config(&self.handle, self.streaming_config)
                .map_err(|e| {
                    TranscribeError::InitFailed(format!(
                        "Failed to spawn streaming ParakeetUnified instance: {}",
                        e
                    ))
                })?;

        let (events_tx, events_rx) = mpsc::channel::<StreamingEvent>(64);
        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();

        let task = tokio::task::spawn_blocking(move || -> Result<(), TranscribeError> {
            let mut last_text = String::new();
            let segment_id: u64 = 0;
            let runtime = tokio::runtime::Handle::current();

            loop {
                // Check cancel without blocking.
                match cancel_rx.try_recv() {
                    Ok(()) => {
                        tracing::debug!("Parakeet streaming session cancelled");
                        break;
                    }
                    Err(oneshot::error::TryRecvError::Closed) => {
                        // Sender dropped without sending; treat as cancel.
                        break;
                    }
                    Err(oneshot::error::TryRecvError::Empty) => {}
                }

                let chunk = match runtime.block_on(samples_rx.recv()) {
                    Some(c) => c,
                    None => break, // graceful EOF
                };

                if chunk.is_empty() {
                    continue;
                }

                let text = match unified.transcribe_chunk(&chunk) {
                    Ok(t) => t,
                    Err(e) => {
                        let err = TranscribeError::InferenceFailed(format!(
                            "ParakeetUnified::transcribe_chunk failed: {}",
                            e
                        ));
                        let _ = runtime.block_on(events_tx.send(StreamingEvent::Error(err)));
                        let _ = runtime.block_on(events_tx.send(StreamingEvent::Ended));
                        return Ok(());
                    }
                };

                if text != last_text {
                    last_text = text.clone();
                    let _ = runtime
                        .block_on(events_tx.send(StreamingEvent::Partial { text, segment_id }));
                }
            }

            // Drain any buffered audio with flush() on close.
            let final_text = match unified.flush() {
                Ok(t) => t.trim().to_string(),
                Err(e) => {
                    tracing::warn!("ParakeetUnified::flush failed: {}", e);
                    last_text.trim().to_string()
                }
            };

            if !final_text.is_empty() {
                let _ = runtime.block_on(events_tx.send(StreamingEvent::Final {
                    text: final_text,
                    segment_id,
                }));
            }
            let _ = runtime.block_on(events_tx.send(StreamingEvent::Ended));
            Ok(())
        });

        // Map the spawn_blocking JoinHandle to the trait's expected
        // JoinHandle<Result<(), TranscribeError>> shape.
        let task = tokio::spawn(async move {
            match task.await {
                Ok(r) => r,
                Err(join_err) => Err(TranscribeError::InferenceFailed(format!(
                    "Parakeet streaming task panicked: {}",
                    join_err
                ))),
            }
        });

        Ok(StreamHandle {
            events: events_rx,
            cancel: cancel_tx,
            task,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test that the type and trait signatures compile. Real model-
    /// driven tests require ONNX runtime + a downloaded model and live in
    /// the integration test suite gated behind `--features parakeet`.
    #[test]
    fn streaming_config_validation_rejects_zero_chunk() {
        let cfg = UnifiedStreamingConfig {
            chunk_secs: 0.0,
            left_context_secs: 1.0,
            right_context_secs: 0.5,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn streaming_config_validation_accepts_defaults() {
        let cfg = UnifiedStreamingConfig::default();
        assert!(cfg.validate().is_ok());
    }

    /// Regression for #442: enabling streaming on a known non-streaming model
    /// (e.g. `parakeet-tdt-0.6b-v3`) must fail early in `new()` with a
    /// message that names the model and points the user at the
    /// streaming-compatible alternative, instead of falling through to an
    /// opaque ONNX Gather shape error at the first audio chunk.
    #[test]
    fn new_rejects_streaming_on_known_incompatible_model() {
        let cfg = ParakeetConfig {
            model: "parakeet-tdt-0.6b-v3".to_string(),
            streaming: true,
            ..ParakeetConfig::default()
        };
        // Can't use `.expect_err`: ParakeetStreamingTranscriber wraps
        // parakeet-rs types that don't derive Debug.
        let err = match ParakeetStreamingTranscriber::new(&cfg) {
            Ok(_) => panic!("streaming on a non-streaming model should error before model load"),
            Err(e) => e,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("parakeet-tdt-0.6b-v3"),
            "error must name the configured model: {msg}"
        );
        assert!(
            msg.contains(crate::setup::model::DEFAULT_PARAKEET_STREAMING_MODEL),
            "error must name the streaming-compatible alternative: {msg}"
        );
        assert!(
            msg.contains("streaming = false") || msg.contains("voxtype setup model"),
            "error must offer at least one concrete fix: {msg}"
        );
    }
}
