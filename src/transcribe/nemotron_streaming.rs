//! Nemotron cache-aware streaming transcription via `parakeet-rs`.

use super::parakeet::{build_execution_config, resolve_model_path};
use super::streaming::{StreamHandle, StreamingEvent, StreamingTranscriber};
use super::{TimedSegment, Transcriber};
use crate::config::ParakeetConfig;
use crate::error::TranscribeError;
use parakeet_rs::{Nemotron, NemotronHandle, NemotronMode};
use std::sync::Mutex;
use tokio::sync::{mpsc, oneshot};

const NEMOTRON_FLUSH_CHUNKS: usize = 3;

/// Streaming-capable Nemotron transcriber with shared model weights.
pub struct NemotronStreamingTranscriber {
    handle: NemotronHandle,
    language: String,
    streaming: bool,
    batch: Mutex<Nemotron>,
}

impl NemotronStreamingTranscriber {
    pub fn new(config: &ParakeetConfig) -> Result<Self, TranscribeError> {
        if config.streaming && config.on_demand_loading {
            return Err(TranscribeError::InitFailed(
                "Nemotron streaming requires [parakeet] on_demand_loading = false so the model is ready when recording starts."
                    .to_string(),
            ));
        }
        let model_path = resolve_model_path(&config.model)?;
        let start = std::time::Instant::now();
        tracing::info!(
            model = %config.model,
            language = %config.language,
            "Loading Nemotron streaming model from {:?}",
            model_path
        );

        let handle = NemotronHandle::load(&model_path, build_execution_config()).map_err(|e| {
            TranscribeError::InitFailed(format!(
                "Nemotron init failed: {e}\n\n\
                 Expected encoder.onnx, decoder_joint.onnx, and tokenizer.model \
                 in {}.",
                model_path.display()
            ))
        })?;

        let mut batch = Nemotron::from_shared(&handle);
        configure_language(&mut batch, &config.language)?;

        tracing::info!(
            "Nemotron model loaded in {:.2}s ({:?}, language={})",
            start.elapsed().as_secs_f32(),
            handle.mode(),
            config.language
        );

        Ok(Self {
            handle,
            language: config.language.clone(),
            streaming: config.streaming,
            batch: Mutex::new(batch),
        })
    }
}

fn configure_language(model: &mut Nemotron, language: &str) -> Result<(), TranscribeError> {
    if model.mode() == NemotronMode::Multilingual {
        model.set_target_lang(language).map_err(|e| {
            TranscribeError::InitFailed(format!("Invalid Nemotron language '{language}': {e}"))
        })?;
    } else if !matches!(language, "auto" | "en" | "en-US") {
        tracing::warn!(
            language,
            "Ignoring target language for English-only Nemotron model"
        );
    }
    Ok(())
}

impl Transcriber for NemotronStreamingTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let mut model = self.batch.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock Nemotron mutex: {e}"))
        })?;
        model
            .transcribe_audio(samples)
            .map(|text| text.trim().to_string())
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Nemotron inference failed: {e}"))
            })
    }

    fn transcribe_timed(&self, _samples: &[f32]) -> Result<Vec<TimedSegment>, TranscribeError> {
        Err(TranscribeError::InferenceFailed(
            "Timed segments are not supported by Nemotron streaming models.".to_string(),
        ))
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        self.streaming.then_some(self)
    }

    fn last_detected_language(&self) -> Option<String> {
        (self.language != "auto").then(|| {
            self.language
                .split('-')
                .next()
                .unwrap_or(&self.language)
                .to_string()
        })
    }
}

enum StreamInput {
    Audio(Option<Vec<f32>>),
    Cancel,
}

impl StreamingTranscriber for NemotronStreamingTranscriber {
    fn start_stream(
        &self,
        mut samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        let mut model = Nemotron::from_shared(&self.handle);
        configure_language(&mut model, &self.language)?;
        let chunk_samples = self.handle.chunk_samples();

        let (events_tx, events_rx) = mpsc::channel::<StreamingEvent>(64);
        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();

        let task = tokio::task::spawn_blocking(move || -> Result<(), TranscribeError> {
            let runtime = tokio::runtime::Handle::current();
            let segment_id = 0;
            let mut cancelled = false;
            let mut total_samples = 0usize;
            let mut pending_partial = String::new();

            loop {
                let input = runtime.block_on(async {
                    tokio::select! {
                        _ = &mut cancel_rx => StreamInput::Cancel,
                        chunk = samples_rx.recv() => StreamInput::Audio(chunk),
                    }
                });

                let chunk = match input {
                    StreamInput::Audio(Some(chunk)) => chunk,
                    StreamInput::Audio(None) => break,
                    StreamInput::Cancel => {
                        cancelled = true;
                        break;
                    }
                };
                if chunk.is_empty() {
                    continue;
                }
                total_samples += chunk.len();

                match model.transcribe_chunk(&chunk) {
                    Ok(text) if !text.is_empty() => {
                        pending_partial.push_str(&text);
                        match events_tx.try_send(StreamingEvent::Partial {
                            text: pending_partial.clone(),
                            segment_id,
                        }) {
                            Ok(()) => pending_partial.clear(),
                            Err(mpsc::error::TrySendError::Full(_)) => {}
                            Err(mpsc::error::TrySendError::Closed(_)) => return Ok(()),
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let err = TranscribeError::InferenceFailed(format!(
                            "Nemotron::transcribe_chunk failed: {e}"
                        ));
                        let _ = runtime.block_on(events_tx.send(StreamingEvent::Error(err)));
                        let _ = runtime.block_on(events_tx.send(StreamingEvent::Ended));
                        return Ok(());
                    }
                }
            }

            if cancelled {
                return Ok(());
            }

            if !pending_partial.is_empty()
                && runtime
                    .block_on(events_tx.send(StreamingEvent::Partial {
                        text: std::mem::take(&mut pending_partial),
                        segment_id,
                    }))
                    .is_err()
            {
                return Ok(());
            }

            {
                let mut final_text = String::new();
                let remainder = total_samples % chunk_samples;
                if remainder != 0 {
                    match model.transcribe_chunk(&vec![0.0; chunk_samples - remainder]) {
                        Ok(text) => final_text.push_str(&text),
                        Err(e) => {
                            let err = TranscribeError::InferenceFailed(format!(
                                "Nemotron final-chunk padding failed: {e}"
                            ));
                            let _ = runtime.block_on(events_tx.send(StreamingEvent::Error(err)));
                            let _ = runtime.block_on(events_tx.send(StreamingEvent::Ended));
                            return Ok(());
                        }
                    }
                }

                // Silence drains the RNNT right context, matching the
                // parakeet-rs reference example.
                for _ in 0..NEMOTRON_FLUSH_CHUNKS {
                    match model.transcribe_chunk(&vec![0.0; chunk_samples]) {
                        Ok(text) => final_text.push_str(&text),
                        Err(e) => {
                            let err = TranscribeError::InferenceFailed(format!(
                                "Nemotron flush failed: {e}"
                            ));
                            let _ = runtime.block_on(events_tx.send(StreamingEvent::Error(err)));
                            let _ = runtime.block_on(events_tx.send(StreamingEvent::Ended));
                            return Ok(());
                        }
                    }
                }
                let _ = runtime.block_on(events_tx.send(StreamingEvent::Final {
                    text: final_text,
                    segment_id,
                }));
            }

            let _ = runtime.block_on(events_tx.send(StreamingEvent::Ended));
            Ok(())
        });

        let task = tokio::spawn(async move {
            match task.await {
                Ok(result) => result,
                Err(e) => Err(TranscribeError::InferenceFailed(format!(
                    "Nemotron streaming task panicked: {e}"
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
    use crate::config::ParakeetModelType;
    use crate::transcribe::StreamingEvent;

    #[test]
    fn streaming_rejects_on_demand_loading_before_model_lookup() {
        let config = ParakeetConfig {
            model: "/does/not/exist".to_string(),
            model_type: Some(ParakeetModelType::Nemotron),
            streaming: true,
            on_demand_loading: true,
            ..ParakeetConfig::default()
        };
        let error = match NemotronStreamingTranscriber::new(&config) {
            Ok(_) => panic!("streaming and on-demand loading must be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("on_demand_loading = false"));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires VOXTYPE_NEMOTRON_MODEL and downloaded model weights"]
    async fn real_model_streams_and_flushes() {
        let model = std::env::var("VOXTYPE_NEMOTRON_MODEL")
            .expect("set VOXTYPE_NEMOTRON_MODEL to a compatible model directory");
        let config = ParakeetConfig {
            model,
            model_type: Some(ParakeetModelType::Nemotron),
            language: "en-US".to_string(),
            streaming: true,
            ..ParakeetConfig::default()
        };
        let transcriber = NemotronStreamingTranscriber::new(&config).unwrap();
        let (samples_tx, samples_rx) = mpsc::channel(16);
        let mut handle = transcriber.start_stream(samples_rx).unwrap();

        let mut reader = hound::WavReader::open("tests/fixtures/vad/speech_long.wav").unwrap();
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|sample| sample.unwrap() as f32 / i16::MAX as f32)
            .collect();
        for chunk in samples.chunks(1_600) {
            samples_tx.send(chunk.to_vec()).await.unwrap();
        }
        drop(samples_tx);

        let mut transcript = String::new();
        let mut final_events = 0;
        while let Some(event) = handle.events.recv().await {
            match event {
                StreamingEvent::Partial { text, .. } => transcript.push_str(&text),
                StreamingEvent::Final { text, .. } => {
                    final_events += 1;
                    transcript.push_str(&text);
                }
                StreamingEvent::Ended => break,
                StreamingEvent::Error(error) => panic!("streaming failed: {error}"),
                StreamingEvent::Replace { .. } => panic!("Nemotron must not revise deltas"),
            }
        }
        handle.task.await.unwrap().unwrap();

        assert_eq!(final_events, 1);
        assert!(transcript
            .to_lowercase()
            .contains("voice activity detection"));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires VOXTYPE_NEMOTRON_MODEL and downloaded model weights"]
    async fn real_model_cancels_with_saturated_event_channel() {
        let model = std::env::var("VOXTYPE_NEMOTRON_MODEL")
            .expect("set VOXTYPE_NEMOTRON_MODEL to a compatible model directory");
        let config = ParakeetConfig {
            model,
            model_type: Some(ParakeetModelType::Nemotron),
            language: "en-US".to_string(),
            streaming: true,
            ..ParakeetConfig::default()
        };
        let transcriber = NemotronStreamingTranscriber::new(&config).unwrap();
        let chunk_samples = transcriber.handle.chunk_samples();
        let (samples_tx, samples_rx) = mpsc::channel(128);
        let mut handle = transcriber.start_stream(samples_rx).unwrap();

        for _ in 0..96 {
            samples_tx.send(vec![0.25; chunk_samples]).await.unwrap();
        }
        let _ = handle.cancel.send(());
        handle.events.close();

        tokio::time::timeout(std::time::Duration::from_secs(5), handle.task)
            .await
            .expect("cancelled stream must not block on a full event channel")
            .unwrap()
            .unwrap();
    }
}
