//! Deepgram cloud streaming WebSocket STT backend.
//!
//! Implements [`StreamingTranscriber`] for live dictation: audio frames
//! stream to Deepgram over a WebSocket and finalized transcript segments
//! stream back. Also implements the one-shot [`Transcriber`] trait (used by
//! `voxtype transcribe file.wav` and meeting-mode chunking) by running a
//! single self-contained streaming round.
//!
//! ## Why finals-only
//!
//! Deepgram emits interim results that are *cumulative per segment* (each
//! interim restates the whole in-progress segment), not deltas. The daemon's
//! [`StreamingSession::type_partial_delta`](crate::output::streaming::StreamingSession)
//! treats partials as deltas to append. Emitting Deepgram interims as
//! `Partial` would therefore duplicate text at the cursor. Rather than carry
//! a prefix-reconciler (as the Soniox backend does), this backend emits only
//! `Final` segments. Deepgram finalizes at endpointing boundaries (roughly
//! per utterance), so finals still stream in throughout a recording.
//!
//! Errors during the session surface as `StreamingEvent::Error` followed by
//! `StreamingEvent::Ended`, matching the trait contract.

use crate::config::DeepgramConfig;
use crate::error::TranscribeError;
use crate::transcribe::streaming::{SegmentId, StreamHandle, StreamingEvent, StreamingTranscriber};
use crate::transcribe::Transcriber;
use deepgram::common::options::{Encoding, Endpointing, Language, Model, Options};
use deepgram::common::stream_response::{Channel, StreamResponse};
use deepgram::listen::websocket::WebsocketHandle;
use deepgram::{Deepgram, DeepgramError};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// 16 kHz mono — matches [`crate::audio::AudioCapture`]'s output.
const SAMPLE_RATE: u32 = 16000;

/// Deepgram drops audio in the first ~200-300ms after the WebSocket opens.
/// A brief silence primer warms the connection without adding latency.
const SILENCE_PRIMER_MS: u64 = 300;

/// Max time to wait for the WebSocket handshake before giving up, so a
/// stalled connect can't pin the daemon in `State::Streaming` forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Deepgram's hard limit: 500 tokens total across all keyterms.
/// https://developers.deepgram.com/docs/keyterm
const KEYTERM_TOKEN_BUDGET: usize = 500;

/// nova-3 and flux models take `keyterm` params; older models
/// (nova-2/nova-1/enhanced/base) take `keywords`. nova-3 rejects
/// `keywords` with a 400 — never send both.
fn uses_keyterms(model: &str) -> bool {
    model.starts_with("nova-3") || model.starts_with("flux")
}

/// Legacy `keywords` param: hard limit 100 keywords per request.
/// https://developers.deepgram.com/docs/keywords
const KEYWORDS_TERM_LIMIT: usize = 100;

/// Enforce the model-appropriate vocabulary limit: keyterm mode uses the
/// 500-token budget (whitespace-token approximation); keywords mode caps
/// at 100 terms. Drops trailing terms and warns with their names.
fn budget_terms(terms: &[String], keyterm_mode: bool) -> Vec<String> {
    let mut used = 0usize;
    let mut kept: Vec<String> = Vec::new();
    let mut dropped: Vec<&str> = Vec::new();
    for term in terms {
        let fits = if keyterm_mode {
            let cost = term.split_whitespace().count().max(1);
            if used + cost <= KEYTERM_TOKEN_BUDGET {
                used += cost;
                true
            } else {
                false
            }
        } else {
            kept.len() < KEYWORDS_TERM_LIMIT
        };
        if fits {
            kept.push(term.clone());
        } else {
            dropped.push(term.as_str());
        }
    }
    if !dropped.is_empty() {
        tracing::warn!(
            "Deepgram vocabulary exceeds the {} limit; dropped: {}",
            if keyterm_mode {
                format!("{KEYTERM_TOKEN_BUDGET}-token keyterm")
            } else {
                format!("{KEYWORDS_TERM_LIMIT}-keyword")
            },
            dropped.join(", ")
        );
    }
    kept
}

/// Install a default rustls `CryptoProvider` for this process.
///
/// The `deepgram` crate's dependency tree enables both the `aws-lc-rs`
/// (via reqwest) and `ring` (via tokio-tungstenite) rustls providers, so
/// rustls cannot select one automatically and panics on the first TLS
/// handshake. We install aws-lc-rs explicitly (matching reqwest's choice).
/// Idempotent: the `Once` guard plus the ignored result make repeated
/// calls and an already-installed provider both safe.
fn ensure_crypto_provider() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// A Deepgram streaming transcriber.
pub struct DeepgramTranscriber {
    config: DeepgramConfig,
    /// Unified vocabulary terms, already resolved by the factory.
    vocabulary: Vec<String>,
}

impl DeepgramTranscriber {
    pub fn new(config: DeepgramConfig, vocabulary: Vec<String>) -> Result<Self, TranscribeError> {
        let key_present = config
            .api_key
            .as_deref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false);
        if !key_present {
            return Err(TranscribeError::ConfigError(
                "Deepgram API key is required. Set it in [deepgram] api_key \
                 or the DEEPGRAM_API_KEY environment variable."
                    .to_string(),
            ));
        }
        // deepgram's deps make rustls' provider ambiguous; pin it before
        // any TLS handshake happens.
        ensure_crypto_provider();
        Ok(Self { config, vocabulary })
    }

    /// Build a Deepgram client from the configured endpoint + key.
    fn build_client(&self) -> Result<Deepgram, TranscribeError> {
        let api_key = self.config.api_key.as_deref().unwrap_or_default();
        if self.config.endpoint == crate::config::DEFAULT_DEEPGRAM_ENDPOINT {
            return Deepgram::new(api_key)
                .map_err(|e| map_client_error(e, "Failed to initialize Deepgram client"));
        }
        let base_url = endpoint_to_base_url(&self.config.endpoint)?;
        Deepgram::with_base_url_and_api_key(base_url.as_str(), api_key).map_err(|e| {
            map_client_error(
                e,
                "Failed to initialize Deepgram client with custom endpoint",
            )
        })
    }

    fn options(&self) -> Options {
        let mut builder = Options::builder()
            .model(Model::from(self.config.model.clone()))
            .language(Language::from(self.config.language.clone()))
            .smart_format(self.config.smart_format);
        if !self.vocabulary.is_empty() {
            let keyterm_mode = uses_keyterms(&self.config.model);
            let kept = budget_terms(&self.vocabulary, keyterm_mode);
            tracing::debug!(
                terms = kept.len(),
                model = %self.config.model,
                "Applying vocabulary to Deepgram request"
            );
            if keyterm_mode {
                builder = builder.keyterms(kept.iter().map(String::as_str));
            } else {
                builder = builder.keywords(kept.iter().map(String::as_str));
            }
        }
        builder.build()
    }

    /// One-shot batch transcription: open a stream, send all samples, close,
    /// and join the finalized segments. Reuses the streaming connection so
    /// `voxtype transcribe file.wav` works without a separate REST path.
    async fn batch_transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        let timeout = Duration::from_secs(self.config.finish_timeout_secs);
        let client = self.build_client()?;

        // Bound every network step so a stalled connect/send/close/receive
        // can't hang the synchronous Transcriber::transcribe() bridge.
        let mut handle = match tokio::time::timeout(
            timeout,
            open_handle(&client, self.options(), self.config.endpointing_ms),
        )
        .await
        {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(TranscribeError::RemoteError(
                    "Deepgram connect timed out".to_string(),
                ))
            }
        };

        // Warm-up primer, then all audio, then close.
        let _ = tokio::time::timeout(timeout, handle.send_data(silence_primer())).await;
        match tokio::time::timeout(timeout, handle.send_data(f32_to_pcm_bytes(samples))).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(TranscribeError::RemoteError(format!(
                    "Deepgram send failed: {e}"
                )))
            }
            Err(_) => {
                return Err(TranscribeError::RemoteError(
                    "Deepgram send timed out".to_string(),
                ))
            }
        }
        match tokio::time::timeout(timeout, handle.close_stream()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("Deepgram close_stream failed: {e}"),
            Err(_) => tracing::warn!("Deepgram close_stream timed out"),
        }

        // Idle timeout per message: a long file keeps producing finals, so
        // the budget resets on each one; a real stall returns an error
        // rather than silently truncating the transcript.
        let mut parts: Vec<String> = Vec::new();
        loop {
            match tokio::time::timeout(timeout, handle.receive()).await {
                Ok(Some(Ok(resp))) => {
                    if let Some(t) = extract_final_transcript(&resp) {
                        if !t.is_empty() {
                            parts.push(t);
                        }
                    }
                }
                Ok(Some(Err(e))) => {
                    return Err(TranscribeError::RemoteError(format!(
                        "Deepgram stream error: {e}"
                    )));
                }
                Ok(None) => break,
                Err(_) => {
                    return Err(TranscribeError::RemoteError(format!(
                        "Deepgram batch transcription stalled (no data for {}s)",
                        timeout.as_secs()
                    )));
                }
            }
        }
        Ok(parts.join(" "))
    }
}

impl Transcriber for DeepgramTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".into()));
        }
        // Bridge sync trait method to the async backend. When called from
        // within voxtype's multi-threaded runtime, use block_in_place; from a
        // bare CLI context with no runtime, spin up a private one.
        let run = async { self.batch_transcribe(samples).await };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| handle.block_on(run)),
            Err(_) => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| {
                        TranscribeError::InferenceFailed(format!("Failed to create runtime: {e}"))
                    })?;
                rt.block_on(run)
            }
        }
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        self.config.streaming.then_some(self as _)
    }
}

impl StreamingTranscriber for DeepgramTranscriber {
    fn start_stream(
        &self,
        samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        // Build the client up front so auth/URL errors surface synchronously.
        let client = self.build_client()?;
        let options = self.options();
        let endpointing_ms = self.config.endpointing_ms;
        let finish_timeout_secs = self.config.finish_timeout_secs;

        let (events_tx, events_rx) = mpsc::channel::<StreamingEvent>(64);
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

        let task = tokio::spawn(async move {
            run_streaming_session(
                client,
                options,
                endpointing_ms,
                finish_timeout_secs,
                samples_rx,
                events_tx,
                cancel_rx,
            )
            .await
        });

        Ok(StreamHandle {
            events: events_rx,
            cancel: cancel_tx,
            task,
        })
    }
}

/// Emit `Error` then `Ended` so the daemon surfaces a notification and resets
/// to idle cleanly. Mirrors the Soniox backend's fatal-path pattern.
async fn send_fatal(events_tx: &mpsc::Sender<StreamingEvent>, msg: String) {
    let _ = events_tx
        .send(StreamingEvent::Error(TranscribeError::RemoteError(msg)))
        .await;
    let _ = events_tx.send(StreamingEvent::Ended).await;
}

async fn run_streaming_session(
    client: Deepgram,
    options: Options,
    endpointing_ms: Option<u32>,
    finish_timeout_secs: u64,
    mut samples_rx: mpsc::Receiver<Vec<f32>>,
    events_tx: mpsc::Sender<StreamingEvent>,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<(), TranscribeError> {
    // Connect, racing against cancel and a hard timeout so a stalled
    // handshake can't pin the daemon in State::Streaming forever.
    let mut handle = {
        tokio::select! {
            biased;
            _ = &mut cancel_rx => {
                tracing::debug!("Deepgram streaming cancelled during connect");
                let _ = events_tx.send(StreamingEvent::Ended).await;
                return Ok(());
            }
            result = tokio::time::timeout(
                CONNECT_TIMEOUT,
                open_handle(&client, options, endpointing_ms),
            ) => {
                match result {
                    Ok(Ok(h)) => h,
                    Ok(Err(e)) => {
                        send_fatal(&events_tx, format!("Deepgram connect failed: {e}")).await;
                        return Ok(());
                    }
                    Err(_) => {
                        send_fatal(
                            &events_tx,
                            format!("Deepgram connect timed out after {}s", CONNECT_TIMEOUT.as_secs()),
                        )
                        .await;
                        return Ok(());
                    }
                }
            }
        }
    };

    // Warm-up primer so the first ~300ms of speech isn't dropped.
    if let Err(e) = handle.send_data(silence_primer()).await {
        tracing::warn!("Deepgram silence primer send failed: {e}");
    }

    let mut next_segment: SegmentId = 0;
    let mut samples_closed = false;
    let mut eof_at: Option<tokio::time::Instant> = None;
    let mut finals_after_stop: u32 = 0;
    // After end-of-audio we wait for Deepgram to flush trailing finals and
    // close. If it never closes cleanly, this deadline stops us from waiting
    // (and, in buffer mode, never flushing) forever.
    let mut drain_deadline: Option<tokio::time::Instant> = None;
    let drain_timeout = Duration::from_secs(finish_timeout_secs);

    loop {
        let drain_timer = async {
            match drain_deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            biased;

            // Cancel from the daemon: stop immediately, no flush.
            _ = &mut cancel_rx => {
                tracing::debug!("Deepgram streaming session cancelled");
                break;
            }

            // Drain deadline hit after end-of-audio without a clean close.
            _ = drain_timer, if drain_deadline.is_some() => {
                tracing::warn!(
                    "Deepgram drain timeout ({}s) after end-of-audio; trailing finals may be lost",
                    drain_timeout.as_secs()
                );
                break;
            }

            // Outgoing audio frames.
            chunk = samples_rx.recv(), if !samples_closed => {
                match chunk {
                    Some(c) if !c.is_empty() => {
                        if let Err(e) = handle.send_data(f32_to_pcm_bytes(&c)).await {
                            send_fatal(&events_tx, format!("Deepgram send audio failed: {e}")).await;
                            return Ok(());
                        }
                    }
                    Some(_) => { /* empty chunk, skip */ }
                    None => {
                        // EOF from daemon: close the send side so Deepgram
                        // flushes remaining finals, then keep reading until
                        // the server closes the socket or the drain deadline.
                        samples_closed = true;
                        eof_at = Some(tokio::time::Instant::now());
                        if let Err(e) = handle.close_stream().await {
                            tracing::warn!("Deepgram close_stream failed: {e}");
                        }
                        drain_deadline = Some(tokio::time::Instant::now() + drain_timeout);
                    }
                }
            }

            // Incoming transcripts.
            response = handle.receive() => {
                match response {
                    Some(Ok(resp)) => {
                        if samples_closed {
                            if let StreamResponse::TerminalResponse { .. } = &resp {
                                tracing::debug!(
                                    "Deepgram terminal metadata received; ending drain early"
                                );
                                break;
                            }
                        }
                        if let Some(text) = extract_final_transcript(&resp) {
                            if !text.is_empty() {
                                let segment_id = next_segment;
                                // Deepgram finals are individually trimmed,
                                // complete segments ("Hello world." then
                                // "How are you?"). Join them with a space so
                                // concatenation doesn't run segments together
                                // ("Hello world.How are you?").
                                let text = if segment_id > 0 {
                                    format!(" {text}")
                                } else {
                                    text
                                };
                                next_segment += 1;
                                if samples_closed {
                                    finals_after_stop += 1;
                                }
                                if events_tx
                                    .send(StreamingEvent::Final { text, segment_id })
                                    .await
                                    .is_err()
                                {
                                    // Daemon dropped the receiver; nothing
                                    // more to do.
                                    break;
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        send_fatal(&events_tx, format!("Deepgram stream error: {e}")).await;
                        return Ok(());
                    }
                    None => {
                        // Server closed the socket cleanly.
                        break;
                    }
                }
            }
        }
    }

    if let Some(t) = eof_at {
        tracing::info!(
            drain_ms = t.elapsed().as_millis() as u64,
            finals_after_stop,
            "Deepgram drain complete"
        );
    }
    let _ = events_tx.send(StreamingEvent::Ended).await;
    Ok(())
}

/// Open a Deepgram realtime WebSocket handle with the given options.
async fn open_handle(
    client: &Deepgram,
    options: Options,
    endpointing_ms: Option<u32>,
) -> Result<WebsocketHandle, TranscribeError> {
    client
        .transcription()
        .stream_request_with_options(options)
        .encoding(Encoding::Linear16)
        .sample_rate(SAMPLE_RATE)
        .channels(1)
        .interim_results(true)
        .endpointing(match endpointing_ms {
            Some(ms) => Endpointing::CustomDurationMs(ms),
            None => Endpointing::Enabled,
        })
        .handle()
        .await
        .map_err(|e| TranscribeError::RemoteError(format!("Failed to open Deepgram stream: {e}")))
}

/// A buffer of silence (`SILENCE_PRIMER_MS` of 16 kHz s16le mono).
fn silence_primer() -> Vec<u8> {
    // 16 kHz × 2 bytes/sample × ms/1000.
    vec![0u8; (SILENCE_PRIMER_MS * SAMPLE_RATE as u64 * 2 / 1000) as usize]
}

/// Convert f32 audio samples (-1.0..1.0) to PCM i16 little-endian bytes.
fn f32_to_pcm_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let i16_val = (clamped * 32767.0) as i16;
        bytes.extend_from_slice(&i16_val.to_le_bytes());
    }
    bytes
}

fn extract_final_transcript(response: &StreamResponse) -> Option<String> {
    match response {
        StreamResponse::TranscriptResponse {
            is_final, channel, ..
        } if *is_final => extract_transcript(channel),
        _ => None,
    }
}

fn extract_transcript(channel: &Channel) -> Option<String> {
    Some(channel.alternatives.first()?.transcript.trim().to_string())
}

fn map_client_error(err: DeepgramError, context: &str) -> TranscribeError {
    match err {
        DeepgramError::InvalidUrl => {
            TranscribeError::ConfigError("Invalid Deepgram endpoint URL".to_string())
        }
        other => TranscribeError::RemoteError(format!("{context}: {other}")),
    }
}

/// Derive a base URL (scheme + host) from a full `/v1/listen` endpoint, for
/// pointing the Deepgram client at a self-hosted instance.
fn endpoint_to_base_url(endpoint: &str) -> Result<String, TranscribeError> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err(TranscribeError::ConfigError(
            "Deepgram endpoint URL cannot be empty".to_string(),
        ));
    }

    let without_query = endpoint
        .split('?')
        .next()
        .unwrap_or(endpoint)
        .trim_end_matches('/');

    if let Some(base) = without_query.strip_suffix("/v1/listen") {
        if !base.is_empty() {
            return Ok(base.to_string());
        }
    }

    let scheme_sep = without_query
        .find("://")
        .ok_or_else(|| TranscribeError::ConfigError("Invalid Deepgram endpoint URL".to_string()))?;
    let host_start = scheme_sep + 3;
    let host_and_path = &without_query[host_start..];
    if host_and_path.is_empty() {
        return Err(TranscribeError::ConfigError(
            "Invalid Deepgram endpoint URL".to_string(),
        ));
    }

    let host_end = host_and_path
        .find('/')
        .map(|idx| host_start + idx)
        .unwrap_or(without_query.len());
    let base = &without_query[..host_end];
    if base.ends_with("://") {
        return Err(TranscribeError::ConfigError(
            "Invalid Deepgram endpoint URL".to_string(),
        ));
    }

    Ok(base.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use deepgram::common::stream_response::{Alternatives, Metadata, ModelInfo};

    fn make_transcript_response(is_final: bool, transcript: &str) -> StreamResponse {
        StreamResponse::TranscriptResponse {
            type_field: "Results".to_string(),
            start: 0.0,
            duration: 1.0,
            is_final,
            speech_final: false,
            from_finalize: false,
            channel: Channel {
                alternatives: vec![Alternatives {
                    transcript: transcript.to_string(),
                    words: Vec::new(),
                    confidence: 0.99,
                    languages: vec!["en".to_string()],
                }],
            },
            metadata: Metadata {
                request_id: "req-123".to_string(),
                model_info: ModelInfo {
                    name: "nova-3".to_string(),
                    version: "latest".to_string(),
                    arch: "nova".to_string(),
                },
                model_uuid: "model-123".to_string(),
            },
            channel_index: vec![0],
        }
    }

    #[test]
    fn keyterm_models_selected_by_prefix() {
        assert!(uses_keyterms("nova-3"));
        assert!(uses_keyterms("nova-3-medical"));
        assert!(uses_keyterms("flux-general-en"));
        assert!(!uses_keyterms("nova-2"));
        assert!(!uses_keyterms("enhanced"));
        assert!(!uses_keyterms("base"));
    }

    #[test]
    fn budget_keeps_keyterms_within_500_tokens() {
        // 260 two-word terms = 520 tokens; only 250 fit in keyterm mode.
        let terms: Vec<String> = (0..260).map(|i| format!("term number{i}")).collect();
        let kept = budget_terms(&terms, true);
        assert_eq!(kept.len(), 250);
        assert_eq!(kept[0], "term number0");
    }

    #[test]
    fn budget_caps_keywords_at_100_terms() {
        // Legacy keywords param: hard limit 100 keywords per request.
        let terms: Vec<String> = (0..150).map(|i| format!("term{i}")).collect();
        let kept = budget_terms(&terms, false);
        assert_eq!(kept.len(), 100);
        assert_eq!(kept[0], "term0");
    }

    #[test]
    fn budget_passes_small_lists_through() {
        let terms = vec!["voxtype".to_string(), "Hyprland".to_string()];
        assert_eq!(budget_terms(&terms, true), terms);
        assert_eq!(budget_terms(&terms, false), terms);
    }

    #[test]
    fn pcm_silence_is_zero() {
        let bytes = f32_to_pcm_bytes(&[0.0; 4]);
        assert_eq!(bytes.len(), 8);
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn pcm_clamps_out_of_range() {
        let bytes = f32_to_pcm_bytes(&[2.0, -2.0]);
        assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), 32767);
        assert_eq!(i16::from_le_bytes([bytes[2], bytes[3]]), -32767);
    }

    #[test]
    fn final_transcript_extracted_only_when_final() {
        let f = make_transcript_response(true, "hello world");
        assert_eq!(
            extract_final_transcript(&f),
            Some("hello world".to_string())
        );
        let interim = make_transcript_response(false, "hello");
        assert_eq!(extract_final_transcript(&interim), None);
    }

    #[test]
    fn silence_primer_sizing() {
        // 300ms @ 16kHz, 2 bytes/sample = 9600 bytes.
        assert_eq!(silence_primer().len(), 9600);
    }

    #[test]
    fn base_url_from_full_endpoint() {
        assert_eq!(
            endpoint_to_base_url("wss://api.deepgram.com/v1/listen").unwrap(),
            "wss://api.deepgram.com"
        );
        assert_eq!(
            endpoint_to_base_url("wss://api.deepgram.com/v1/listen?model=nova-3").unwrap(),
            "wss://api.deepgram.com"
        );
    }

    #[test]
    fn new_rejects_missing_key() {
        let cfg = DeepgramConfig::default();
        assert!(DeepgramTranscriber::new(cfg, Vec::new()).is_err());
    }
}
