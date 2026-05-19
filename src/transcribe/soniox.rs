//! Soniox cloud streaming WebSocket STT backend.
//!
//! Connects to `wss://stt-rt.soniox.com/transcribe-websocket` and pipes
//! 16 kHz mono `pcm_s16le` audio frames over WebSocket, receiving JSON
//! token messages with per-token `is_final` flags. Implements both:
//!
//! - [`Transcriber::transcribe`] — batch path used when
//!   `[soniox] streaming = false` (push-to-talk compatible). Buffers the
//!   audio, opens a one-shot WS session, sends the buffer + empty
//!   binary end-of-audio marker, drains finals until `finished: true`,
//!   and returns the concatenated text.
//!
//! - [`StreamingTranscriber::start_stream`] — live streaming session.
//!   Exposed only when `[soniox] streaming = true` (the default). The
//!   daemon's `streaming_active()` gate auto-promotes push-to-talk to
//!   toggle when this is the active engine, matching the constraint
//!   documented in the v0.7.2 release notes.
//!
//! ## Token reconciliation
//!
//! Soniox emits cumulative finals (a token marked `is_final: true` is
//! committed forever) and revisable non-finals (`is_final: false` may
//! be changed by later messages). Voxtype's `StreamingSession` types
//! events as deltas at the cursor — there is no rewind primitive for
//! partial revision. So this backend keeps a small state machine:
//!
//! - `typed_partial: String` tracks what's been emitted as `Partial`
//!   (and thus typed at the cursor as a tentative tail).
//! - When server finals arrive, the delta against `typed_partial` is
//!   emitted as `StreamingEvent::Final` so the daemon's
//!   `commit_segment` reconciliation works out. If the server's finals
//!   start with `typed_partial`, only the new tail is typed (zero
//!   churn). If they diverge (rare, would be a non-final revision),
//!   the full final is typed and the cursor shows a transient artifact
//!   that the user can fix.
//! - For partials, only stable extensions of `typed_partial` are
//!   emitted. Revisions are silently dropped — better to wait for the
//!   final to resolve than to backspace mid-utterance.
//!
//! ## Errors
//!
//! Connect timeouts, WS errors, and Soniox server `error_message`
//! responses surface as `StreamingEvent::Error` followed by `Ended`.
//! The daemon disowns the session on `Error`/`Ended` so post-stop
//! emissions are dropped (matches the v0.7.2 disown-on-stop fix).

use super::streaming::{SegmentId, StreamHandle, StreamingEvent, StreamingTranscriber};
use super::Transcriber;
use crate::config::SonioxConfig;
use crate::error::TranscribeError;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

const SAMPLE_RATE: u32 = 16_000;
const AUDIO_FORMAT: &str = "pcm_s16le";

/// Production WebSocket endpoint. Soniox doesn't offer regional alternates;
/// the only realistic override would be a local mock during testing, which
/// the tests construct directly without touching this constant.
const SONIOX_WS_ENDPOINT: &str = "wss://stt-rt.soniox.com/transcribe-websocket";

/// Base URL for the Soniox async transcription REST API.
const SONIOX_ASYNC_BASE: &str = "https://api.soniox.com/v1";

/// WS connect / batch-request timeout. Fixed because the only failure
/// mode it covers (connect or one-shot request stalls) is a flat network
/// fault — 30s is comfortable for any working uplink, longer just makes
/// stalls feel worse.
const SONIOX_TIMEOUT: Duration = Duration::from_secs(30);

/// Async REST job polling cadence. Pure implementation detail of the
/// poll loop; 500 ms is fast enough that completion feels instant on the
/// daemon side without burning API call quota.
const ASYNC_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Manual-finalization control frame per
/// https://soniox.com/docs/stt/rt/manual-finalization. Server promotes
/// any in-flight non-final tokens to final and emits a `<fin>` marker.
const FINALIZE_FRAME: &str = r#"{"type":"finalize"}"#;

#[derive(Debug)]
pub struct SonioxTranscriber {
    config: SonioxConfig,
    api_key: String,
    /// Merged vocabulary-boost terms from `config.terms` + `config.terms_file`.
    /// Loaded once at construction so we don't hit disk per session.
    context_terms: Vec<String>,
    /// Lazy reqwest client for the async REST path. Built on first use
    /// via `OnceLock` so we don't pay TLS/DNS-pool init per dictation
    /// and don't allocate one for users who only use the WS realtime path.
    async_client: std::sync::OnceLock<reqwest::Client>,
}

/// Read `config.terms_file` (a JSON array of strings) and merge with
/// `config.terms`, deduplicating while preserving first-seen order. Empty
/// strings are skipped. Returns an error if the file path is set but the
/// file is missing or malformed.
fn load_context_terms(config: &SonioxConfig) -> Result<Vec<String>, TranscribeError> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut push = |t: String| {
        let trimmed = t.trim();
        if trimmed.is_empty() {
            return;
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    };

    if let Some(inline) = &config.terms {
        for t in inline {
            push(t.clone());
        }
    }
    if let Some(path) = &config.terms_file {
        let bytes = std::fs::read(path).map_err(|e| {
            TranscribeError::ConfigError(format!(
                "Soniox terms_file unreadable ({}): {}",
                path.display(),
                e
            ))
        })?;
        let parsed: Vec<String> = serde_json::from_slice(&bytes).map_err(|e| {
            TranscribeError::ConfigError(format!(
                "Soniox terms_file must be a JSON array of strings ({}): {}",
                path.display(),
                e
            ))
        })?;
        for t in parsed {
            push(t);
        }
    }
    Ok(out)
}

impl SonioxTranscriber {
    pub fn new(mut config: SonioxConfig) -> Result<Self, TranscribeError> {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("SONIOX_API_KEY").ok())
            .ok_or_else(|| {
                TranscribeError::ConfigError(
                    "Soniox API key required: set [soniox] api_key or SONIOX_API_KEY".into(),
                )
            })?;

        // User left the default realtime model with async_api enabled —
        // swap to the async-default model.
        if config.async_api && config.model == "stt-rt-v4" {
            config.model = "stt-async-v4".to_string();
        }

        let context_terms = load_context_terms(&config)?;

        if config.async_api {
            tracing::info!(
                "Soniox backend configured: mode=async, model={}, language_hints={:?} (strict={}), context_terms={}",
                config.model,
                config.language_hints,
                config.language_hints_strict,
                context_terms.len(),
            );
        } else {
            tracing::info!(
                "Soniox backend configured: mode=realtime, model={}, streaming={}, language_hints={:?} (strict={}), context_terms={}",
                config.model,
                config.streaming,
                config.language_hints,
                config.language_hints_strict,
                context_terms.len(),
            );
        }
        Ok(Self {
            config,
            api_key,
            context_terms,
            async_client: std::sync::OnceLock::new(),
        })
    }

    fn async_client(&self) -> Result<&reqwest::Client, TranscribeError> {
        if let Some(c) = self.async_client.get() {
            return Ok(c);
        }
        let client = reqwest::Client::builder()
            .timeout(SONIOX_TIMEOUT)
            .build()
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("reqwest client build failed: {}", e))
            })?;
        // get_or_init can't return Result, so first build then OnceLock::set;
        // race with another thread is fine — we'd just drop one extra client.
        let _ = self.async_client.set(client);
        Ok(self.async_client.get().expect("just initialized"))
    }

    fn init_frame(&self) -> String {
        // `enable_endpoint_detection: true` is hardcoded. Turning it off
        // is strictly worse — mid-stream segment finals don't fire on
        // natural pauses, so partials churn more. The stop pipeline still
        // works either way because we drive `{"type":"finalize"}` explicitly.
        let mut obj = serde_json::json!({
            "api_key": self.api_key,
            "model": self.config.model,
            "audio_format": AUDIO_FORMAT,
            "sample_rate": SAMPLE_RATE,
            "num_channels": 1,
            "language_hints": self.config.language_hints,
            "enable_endpoint_detection": true,
        });
        // Only meaningful when hints are present; Soniox ignores otherwise
        // but keep the frame clean by gating on non-empty hints.
        if !self.config.language_hints.is_empty() {
            obj["language_hints_strict"] =
                serde_json::Value::Bool(self.config.language_hints_strict);
        }
        let mut ctx_obj = serde_json::Map::new();
        if let Some(text) = self
            .config
            .context
            .as_ref()
            .filter(|s| !s.trim().is_empty())
        {
            ctx_obj.insert("text".to_string(), serde_json::Value::String(text.clone()));
        }
        if !self.context_terms.is_empty() {
            ctx_obj.insert(
                "terms".to_string(),
                serde_json::Value::Array(
                    self.context_terms
                        .iter()
                        .map(|t| serde_json::Value::String(t.clone()))
                        .collect(),
                ),
            );
        }
        if !ctx_obj.is_empty() {
            obj["context"] = serde_json::Value::Object(ctx_obj);
        }
        obj.to_string()
    }
}

// === Soniox WebSocket protocol types ===

#[derive(Deserialize, Debug, Default)]
struct ServerMessage {
    #[serde(default)]
    tokens: Vec<Token>,
    #[serde(default)]
    finished: bool,
    #[serde(default)]
    error_code: Option<i64>,
    #[serde(default)]
    error_message: Option<String>,
}

#[derive(Deserialize, Debug)]
struct Token {
    text: String,
    /// Realtime tokens always include `is_final`. Async transcripts
    /// omit it (everything in the final transcript is implicitly final).
    /// Default to `true` so the async fetch path works without a separate
    /// Token type.
    #[serde(default = "default_is_final")]
    is_final: bool,
    // Unused: speaker, language, translation_status, start_ms, end_ms,
    // confidence, etc.
}

fn default_is_final() -> bool {
    true
}

/// Convert 16 kHz f32 mono samples in [-1.0, 1.0] to little-endian s16 bytes.
/// The returned Vec is consumed by `Message::Binary` via zero-copy
/// `Vec → Bytes` ownership transfer, so this is the per-chunk allocation
/// for the WS streaming path.
fn f32_to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn f32_to_s16le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        out.extend_from_slice(&f32_to_i16(s).to_le_bytes());
    }
    out
}

/// Count the number of Unicode scalars shared as a prefix between
/// `a` and `b`. Used by the reconciler to compute backspace counts
/// for tail revisions.
fn common_prefix_char_count(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// Replace the `api_key` value in a Soniox init-frame JSON string with
/// `***` so the wire-trace log doesn't leak credentials. Parses + re-emits
/// via serde_json so the result handles any formatting variation.
/// Returns the original frame unchanged if it doesn't parse or has no
/// api_key field (logging shouldn't crash on edge cases).
fn redact_api_key(frame: &str) -> String {
    let mut value: serde_json::Value = match serde_json::from_str(frame) {
        Ok(v) => v,
        Err(_) => return frame.to_string(),
    };
    if let Some(obj) = value.as_object_mut() {
        if obj.contains_key("api_key") {
            obj.insert("api_key".into(), serde_json::Value::String("***".into()));
        }
    }
    value.to_string()
}

/// Soniox emits special control tokens such as `<end>` (endpoint
/// detection), `<fin>`, language tags like `<lang:en>`, etc. These are
/// metadata, not user-visible text — filter them so they don't land at
/// the cursor.
fn is_special_token(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with('<') && trimmed.ends_with('>') && trimmed.len() > 2
}

/// Split a token list into (concatenated final text, concatenated non-final
/// text) preserving order. Skips Soniox special tokens (see
/// [`is_special_token`]).
fn split_tokens(tokens: &[Token]) -> (String, String) {
    let mut final_text = String::new();
    let mut partial_text = String::new();
    for tok in tokens {
        if is_special_token(&tok.text) {
            continue;
        }
        if tok.is_final {
            final_text.push_str(&tok.text);
        } else {
            partial_text.push_str(&tok.text);
        }
    }
    (final_text, partial_text)
}

/// What `Reconciler::process` decided about a server message.
#[derive(Debug, Default)]
struct ReconcilerOutput {
    events: Vec<StreamingEvent>,
    /// The server signalled `finished: true` or surfaced an error; the
    /// caller should stop the session after dispatching `events`.
    terminate: bool,
}

/// State machine that converts a stream of Soniox `ServerMessage`s into
/// voxtype `StreamingEvent` deltas.
///
/// Soniox emits cumulative finals (committed forever) and revisable
/// non-finals. Voxtype's `StreamingSession` types events as deltas at the
/// cursor with no rewind primitive, so the reconciler:
///
/// - Tracks `typed_partial` (the non-final tail already typed via Partial
///   events).
/// - Strips that prefix from incoming finals so we emit only the new tail.
/// - For partials, only emits stable extensions of `typed_partial`. If
///   the server revises the tail (a non-finalstarts-with check fails), we
///   silently drop the partial and wait for finalization to resolve.
#[derive(Debug, Default)]
struct Reconciler {
    typed_partial: String,
}

/// Soniox sessions are single-segment from voxtype's perspective: the WS
/// closes on `finished:true` and any subsequent dictation opens a fresh
/// session. Other backends use segment_id to demarcate sequential
/// utterances within one session; we just emit zero.
const SEGMENT_ID: SegmentId = 0;

impl Reconciler {
    fn process(&mut self, parsed: &ServerMessage, type_partials: bool) -> ReconcilerOutput {
        let mut out = ReconcilerOutput::default();

        if let Some(err) = &parsed.error_message {
            out.events
                .push(StreamingEvent::Error(TranscribeError::InferenceFailed(
                    format!(
                        "Soniox server error{}: {}",
                        parsed
                            .error_code
                            .map(|c| format!(" ({})", c))
                            .unwrap_or_default(),
                        err
                    ),
                )));
            out.terminate = true;
            return out;
        }

        let (final_text, partial_text) = split_tokens(&parsed.tokens);

        if !final_text.is_empty() {
            if final_text.starts_with(&self.typed_partial) {
                // Common case: final extends what we typed as partial.
                // Type just the tail and consume the partial.
                let delta = final_text[self.typed_partial.len()..].to_string();
                if !delta.is_empty() {
                    out.events.push(StreamingEvent::Final {
                        text: delta,
                        segment_id: SEGMENT_ID,
                    });
                }
                self.typed_partial.clear();
            } else if self.typed_partial.starts_with(&final_text) {
                // Soniox finalized only the leading portion of what we've
                // typed as partial; the rest stays in-flight. Nothing new
                // to type — just shift our local view of the still-pending
                // partial. The cursor already shows the correct text.
                self.typed_partial = self.typed_partial[final_text.len()..].to_string();
            } else {
                // Tail revision: typed_partial and final_text share a
                // common prefix but diverge afterward. Backspace the
                // mismatched tail of typed_partial and type the final's
                // tail. Counts are in Unicode scalars to match how
                // typed_chars is tracked.
                let lcp_chars = common_prefix_char_count(&self.typed_partial, &final_text);
                let backspace = self.typed_partial.chars().count() - lcp_chars;
                let final_tail: String = final_text.chars().skip(lcp_chars).collect();
                tracing::debug!(
                    "Soniox tail revision: backspace {} chars, type {:?} (lcp={})",
                    backspace,
                    final_tail,
                    lcp_chars,
                );
                out.events.push(StreamingEvent::Replace {
                    backspace,
                    text: final_tail,
                    segment_id: SEGMENT_ID,
                });
                self.typed_partial.clear();
            }
        }

        // When type_partials is off, no partial is empty, AND the server
        // hasn't revised what we typed, emit only the new tail. The
        // implicit "else" (server revised our typed partial) is silently
        // dropped — the next finalization round resolves it.
        if type_partials
            && !partial_text.is_empty()
            && partial_text.starts_with(&self.typed_partial)
        {
            let delta = partial_text[self.typed_partial.len()..].to_string();
            if !delta.is_empty() {
                out.events.push(StreamingEvent::Partial {
                    text: delta,
                    segment_id: SEGMENT_ID,
                });
                self.typed_partial = partial_text;
            }
        }

        if parsed.finished {
            out.terminate = true;
        }

        out
    }
}

impl Transcriber for SonioxTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".into()));
        }
        // Bridge sync trait method to async backend. Two callers:
        //
        // - Meeting daemon's chunk processor: already inside a tokio
        //   runtime (`#[tokio::main]` multi-thread). Building a fresh
        //   runtime would panic with "Cannot start a runtime from within
        //   a runtime." Use block_in_place + the existing handle.
        //
        // - One-shot CLI (`voxtype transcribe file.wav`): no ambient
        //   runtime. Spin up a private current-thread one.
        let run = async {
            if self.config.async_api {
                self.async_transcribe(samples).await
            } else {
                self.batch_transcribe(samples).await
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| handle.block_on(run)),
            Err(_) => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| {
                        TranscribeError::InferenceFailed(format!("Failed to create runtime: {}", e))
                    })?;
                rt.block_on(run)
            }
        }
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        // Async API is batch-only; streaming=false also keeps PTT compatible.
        (!self.config.async_api && self.config.streaming).then_some(self as _)
    }
}

impl SonioxTranscriber {
    async fn batch_transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        let (ws_stream, _) = tokio::time::timeout(
            SONIOX_TIMEOUT,
            tokio_tungstenite::connect_async(SONIOX_WS_ENDPOINT),
        )
        .await
        .map_err(|_| TranscribeError::InferenceFailed("Soniox: connect timeout".into()))?
        .map_err(|e| {
            TranscribeError::InferenceFailed(format!("Soniox: WS connect failed: {}", e))
        })?;

        let (mut write, mut read) = ws_stream.split();

        let init = self.init_frame();
        tracing::debug!(target: "voxtype::soniox::wire", "-> init {}", redact_api_key(&init));
        write.send(Message::Text(init)).await.map_err(|e| {
            TranscribeError::InferenceFailed(format!("Soniox: send init failed: {}", e))
        })?;

        // Send audio in chunks. 32 KiB = ~1s of pcm_s16le at 16 kHz.
        // Soniox examples use ~120ms frames but larger frames work fine
        // for batch.
        let bytes = f32_to_s16le_bytes(samples);
        const FRAME_BYTES: usize = 32 * 1024;
        for chunk in bytes.chunks(FRAME_BYTES) {
            write
                .send(Message::Binary(chunk.to_vec()))
                .await
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!("Soniox: send audio failed: {}", e))
                })?;
        }

        // Protocol-pure stop: manual finalize forces any pending non-final
        // tokens to final without waiting for endpoint detection (which
        // needs 2s of trailing silence by default and won't fire on a
        // tight batch of pre-recorded audio). Then the empty text frame
        // ends the stream. Server flushes finals, sends `finished:true`,
        // closes. Without finalize, short batches stall server-side until
        // the 408 "Request timeout" fires ~20s later.
        write
            .send(Message::Text(FINALIZE_FRAME.into()))
            .await
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Soniox: send finalize failed: {}", e))
            })?;
        write
            .send(Message::Text(String::new()))
            .await
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Soniox: send EOA failed: {}", e))
            })?;

        let mut transcript = String::new();
        let deadline = tokio::time::Instant::now() + SONIOX_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(TranscribeError::InferenceFailed(
                    "Soniox: batch timeout".into(),
                ));
            }
            let msg = match tokio::time::timeout(remaining, read.next()).await {
                Ok(Some(Ok(m))) => m,
                Ok(Some(Err(e))) => {
                    return Err(TranscribeError::InferenceFailed(format!(
                        "Soniox: WS error: {}",
                        e
                    )))
                }
                Ok(None) => break,
                Err(_) => {
                    return Err(TranscribeError::InferenceFailed(
                        "Soniox: batch timeout".into(),
                    ))
                }
            };
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Close(_) => break,
                _ => continue,
            };
            let parsed: ServerMessage = match serde_json::from_str(&text) {
                Ok(p) => p,
                Err(e) => {
                    tracing::debug!("Soniox: unparseable message ({}): {}", e, text);
                    continue;
                }
            };
            if let Some(err) = parsed.error_message {
                return Err(TranscribeError::InferenceFailed(format!(
                    "Soniox server error{}: {}",
                    parsed
                        .error_code
                        .map(|c| format!(" ({})", c))
                        .unwrap_or_default(),
                    err
                )));
            }
            for tok in &parsed.tokens {
                if tok.is_final && !is_special_token(&tok.text) {
                    transcript.push_str(&tok.text);
                }
            }
            if parsed.finished {
                break;
            }
        }

        let _ = write.send(Message::Close(None)).await;
        Ok(transcript.trim().to_string())
    }
}

/// Encode 16 kHz f32 mono samples as a WAV byte buffer (PCM 16-bit LE)
/// for upload to the Soniox async API. Mirrors the helper in
/// `transcribe/remote.rs` so both cloud backends share one WAV format.
fn encode_wav_s16le(samples: &[f32]) -> Result<Vec<u8>, TranscribeError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut buffer, spec)
        .map_err(|e| TranscribeError::AudioFormat(format!("WAV writer init: {}", e)))?;
    for &s in samples {
        writer
            .write_sample(f32_to_i16(s))
            .map_err(|e| TranscribeError::AudioFormat(format!("WAV sample write: {}", e)))?;
    }
    writer
        .finalize()
        .map_err(|e| TranscribeError::AudioFormat(format!("WAV finalize: {}", e)))?;
    Ok(buffer.into_inner())
}

#[derive(Deserialize, Debug)]
struct FileUploadResponse {
    id: String,
}

#[derive(Deserialize, Debug)]
struct TranscriptionCreateResponse {
    id: String,
}

#[derive(Deserialize, Debug)]
struct TranscriptionStatusResponse {
    status: String,
    #[serde(default)]
    error_message: Option<String>,
}

#[derive(Deserialize, Debug)]
struct TranscriptResponse {
    tokens: Vec<Token>,
}

impl SonioxTranscriber {
    async fn async_transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        let wav = encode_wav_s16le(samples)?;
        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::info!(
            "Soniox async: uploading {:.1}s audio ({} KiB) as WAV",
            duration_secs,
            wav.len() / 1024
        );

        let client = self.async_client()?;

        let auth = format!("Bearer {}", self.api_key);

        // 1. Upload file (multipart).
        let upload_start = std::time::Instant::now();
        let file_part = reqwest::multipart::Part::bytes(wav)
            .file_name("voxtype.wav")
            .mime_str("audio/wav")
            .map_err(|e| TranscribeError::InferenceFailed(format!("mime build failed: {}", e)))?;
        let form = reqwest::multipart::Form::new().part("file", file_part);
        let resp = client
            .post(format!("{}/files", SONIOX_ASYNC_BASE))
            .header("Authorization", &auth)
            .multipart(form)
            .send()
            .await
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Soniox async: upload failed: {}", e))
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(TranscribeError::InferenceFailed(format!(
                "Soniox async: upload returned {}: {}",
                status, body
            )));
        }
        let file_resp: FileUploadResponse = resp.json().await.map_err(|e| {
            TranscribeError::InferenceFailed(format!("Soniox async: parse upload response: {}", e))
        })?;
        let file_id = file_resp.id;
        tracing::debug!(
            "Soniox async: uploaded in {:.2}s, file_id={}",
            upload_start.elapsed().as_secs_f32(),
            file_id
        );

        // 2. Create transcription job.
        let mut body = serde_json::json!({
            "model": self.config.model,
            "file_id": file_id,
        });
        if !self.config.language_hints.is_empty() {
            body["language_hints"] = serde_json::json!(self.config.language_hints);
            body["language_hints_strict"] = serde_json::json!(self.config.language_hints_strict);
        }
        // Mirror the init_frame context shape: text + terms together.
        // Critical for meeting mode (which forces async), where users
        // configuring `terms_file` would otherwise silently lose
        // vocabulary boosts that work fine for realtime dictation.
        let mut ctx_obj = serde_json::Map::new();
        if let Some(text) = self
            .config
            .context
            .as_ref()
            .filter(|s| !s.trim().is_empty())
        {
            ctx_obj.insert("text".to_string(), serde_json::Value::String(text.clone()));
        }
        if !self.context_terms.is_empty() {
            ctx_obj.insert(
                "terms".to_string(),
                serde_json::Value::Array(
                    self.context_terms
                        .iter()
                        .map(|t| serde_json::Value::String(t.clone()))
                        .collect(),
                ),
            );
        }
        if !ctx_obj.is_empty() {
            body["context"] = serde_json::Value::Object(ctx_obj);
        }
        let resp = client
            .post(format!("{}/transcriptions", SONIOX_ASYNC_BASE))
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Soniox async: create transcription failed: {}",
                    e
                ))
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            // Best effort: delete the orphaned file.
            self.async_delete_file(client, &auth, &file_id).await;
            return Err(TranscribeError::InferenceFailed(format!(
                "Soniox async: create transcription returned {}: {}",
                status, text
            )));
        }
        let job: TranscriptionCreateResponse = resp.json().await.map_err(|e| {
            TranscribeError::InferenceFailed(format!("Soniox async: parse job response: {}", e))
        })?;
        let job_id = job.id;
        tracing::debug!("Soniox async: created job_id={}", job_id);

        // 3. Poll until completed or error.
        let poll_interval = ASYNC_POLL_INTERVAL;
        let max_wait = Duration::from_secs(self.config.async_max_wait_secs.max(5));
        let poll_start = std::time::Instant::now();
        loop {
            if poll_start.elapsed() > max_wait {
                self.async_cleanup(client, &auth, &job_id).await;
                return Err(TranscribeError::InferenceFailed(format!(
                    "Soniox async: job {} did not complete within {}s",
                    job_id,
                    max_wait.as_secs()
                )));
            }
            tokio::time::sleep(poll_interval).await;
            let resp = client
                .get(format!("{}/transcriptions/{}", SONIOX_ASYNC_BASE, job_id))
                .header("Authorization", &auth)
                .send()
                .await
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!("Soniox async: poll failed: {}", e))
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                self.async_cleanup(client, &auth, &job_id).await;
                return Err(TranscribeError::InferenceFailed(format!(
                    "Soniox async: poll returned {}: {}",
                    status, text
                )));
            }
            let status_resp: TranscriptionStatusResponse = resp.json().await.map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Soniox async: parse status response: {}",
                    e
                ))
            })?;
            match status_resp.status.as_str() {
                "completed" => break,
                "error" => {
                    let err = status_resp
                        .error_message
                        .unwrap_or_else(|| "unspecified error".to_string());
                    self.async_cleanup(client, &auth, &job_id).await;
                    return Err(TranscribeError::InferenceFailed(format!(
                        "Soniox async: server error: {}",
                        err
                    )));
                }
                "processing" | "queued" | "running" => {
                    tracing::trace!("Soniox async: job {} status={}", job_id, status_resp.status);
                }
                other => {
                    tracing::warn!(
                        "Soniox async: unknown status '{}', continuing to poll",
                        other
                    );
                }
            }
        }
        tracing::info!(
            "Soniox async: transcription completed in {:.2}s",
            poll_start.elapsed().as_secs_f32()
        );

        // 4. Fetch transcript.
        let resp = client
            .get(format!(
                "{}/transcriptions/{}/transcript",
                SONIOX_ASYNC_BASE, job_id
            ))
            .header("Authorization", &auth)
            .send()
            .await
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Soniox async: fetch transcript: {}", e))
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            self.async_cleanup(client, &auth, &job_id).await;
            return Err(TranscribeError::InferenceFailed(format!(
                "Soniox async: fetch transcript returned {}: {}",
                status, text
            )));
        }
        let body = resp.text().await.map_err(|e| {
            TranscribeError::InferenceFailed(format!("Soniox async: read transcript body: {}", e))
        })?;
        tracing::debug!(target: "voxtype::soniox::wire", "<- transcript: {}", body);
        let transcript: TranscriptResponse = serde_json::from_str(&body).map_err(|e| {
            TranscribeError::InferenceFailed(format!(
                "Soniox async: parse transcript: {} (body: {})",
                e, body
            ))
        })?;

        // 5. Cleanup server-side state (best effort, don't fail user-visible).
        self.async_cleanup(client, &auth, &job_id).await;

        // 6. Concatenate tokens (skip special markers).
        let mut out = String::new();
        for tok in &transcript.tokens {
            if is_special_token(&tok.text) {
                continue;
            }
            out.push_str(&tok.text);
        }
        Ok(out.trim().to_string())
    }

    async fn async_delete_file(&self, client: &reqwest::Client, auth: &str, file_id: &str) {
        let _ = client
            .delete(format!("{}/files/{}", SONIOX_ASYNC_BASE, file_id))
            .header("Authorization", auth)
            .send()
            .await;
    }

    async fn async_cleanup(&self, client: &reqwest::Client, auth: &str, job_id: &str) {
        // DELETE /v1/transcriptions/{id} cascades to its associated file
        // per Soniox docs, so we don't issue a separate /files/{id} DELETE.
        let _ = client
            .delete(format!("{}/transcriptions/{}", SONIOX_ASYNC_BASE, job_id))
            .header("Authorization", auth)
            .send()
            .await;
    }
}

impl StreamingTranscriber for SonioxTranscriber {
    fn start_stream(
        &self,
        samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        let (events_tx, events_rx) = mpsc::channel::<StreamingEvent>(64);
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

        let init = self.init_frame();
        let type_partials = self.config.type_partials;

        let task = tokio::spawn(async move {
            run_streaming_session(init, type_partials, samples_rx, events_tx, cancel_rx).await
        });

        Ok(StreamHandle {
            events: events_rx,
            cancel: cancel_tx,
            task,
        })
    }
}

/// Emit an `Error` followed by `Ended` so the daemon surfaces a notification
/// and cleanly resets to idle. The pattern repeats at every fatal site in
/// `run_streaming_session`.
async fn send_fatal(events_tx: &mpsc::Sender<StreamingEvent>, msg: String) {
    let _ = events_tx
        .send(StreamingEvent::Error(TranscribeError::InferenceFailed(msg)))
        .await;
    let _ = events_tx.send(StreamingEvent::Ended).await;
}

async fn run_streaming_session(
    init_frame: String,
    type_partials: bool,
    mut samples_rx: mpsc::Receiver<Vec<f32>>,
    events_tx: mpsc::Sender<StreamingEvent>,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<(), TranscribeError> {
    // Connect.
    let ws_result = tokio::time::timeout(
        SONIOX_TIMEOUT,
        tokio_tungstenite::connect_async(SONIOX_WS_ENDPOINT),
    )
    .await;
    let ws_stream = match ws_result {
        Ok(Ok((s, _))) => s,
        Ok(Err(e)) => {
            send_fatal(&events_tx, format!("Soniox: WS connect failed: {}", e)).await;
            return Ok(());
        }
        Err(_) => {
            send_fatal(&events_tx, "Soniox: connect timeout".into()).await;
            return Ok(());
        }
    };
    let (mut write, mut read) = ws_stream.split();

    // Send init.
    tracing::debug!(target: "voxtype::soniox::wire", "-> init {}", redact_api_key(&init_frame));
    if let Err(e) = write.send(Message::Text(init_frame)).await {
        send_fatal(&events_tx, format!("Soniox: send init failed: {}", e)).await;
        return Ok(());
    }

    let mut reconciler = Reconciler::default();
    let mut samples_closed = false;
    let mut sent_eoa = false;
    // Safety net for the server failing to close after our protocol-pure
    // shutdown (see the EOF arm below). The 5 s budget is comfortable for
    // a clean Soniox close (~200–300 ms in practice).
    let mut drain_deadline: Option<tokio::time::Instant> = None;
    const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

    loop {
        let drain_timer = async {
            match drain_deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            biased;

            // Highest priority: cancel signal from daemon.
            _ = &mut cancel_rx => {
                tracing::debug!("Soniox streaming session cancelled");
                break;
            }

            // Drain timeout fired without server-initiated close.
            _ = drain_timer, if drain_deadline.is_some() => {
                tracing::info!("Soniox drain timeout reached after end-of-audio; closing session");
                break;
            }

            // Outgoing audio frames.
            chunk = samples_rx.recv(), if !samples_closed => {
                match chunk {
                    Some(c) if !c.is_empty() => {
                        let bytes = f32_to_s16le_bytes(&c);
                        if let Err(e) = write.send(Message::Binary(bytes)).await {
                            let _ = events_tx.send(StreamingEvent::Error(
                                TranscribeError::InferenceFailed(format!(
                                    "Soniox: send audio failed: {}", e
                                ))
                            )).await;
                            break;
                        }
                    }
                    Some(_) => { /* empty chunk, skip */ }
                    None => {
                        // EOF from daemon. Protocol-pure stop sequence per
                        // https://soniox.com/docs/stt/rt/manual-finalization
                        // and /docs/api-reference/stt/websocket-api:
                        //   1. {"type":"finalize"}  — force-finalize any
                        //      in-flight non-final tokens (the server
                        //      emits them as is_final:true plus a `<fin>`
                        //      marker we filter out).
                        //   2. ""                    — empty text frame,
                        //      documented end-of-audio signal.
                        // Server then sends remaining finals followed by
                        // `finished:true` and closes the socket.
                        samples_closed = true;
                        if !sent_eoa {
                            if let Err(e) = write
                                .send(Message::Text(FINALIZE_FRAME.into()))
                                .await
                            {
                                tracing::warn!("Soniox: finalize send failed: {}", e);
                            }
                            if let Err(e) = write.send(Message::Text(String::new())).await {
                                tracing::warn!("Soniox: EOA send failed: {}", e);
                            }
                            sent_eoa = true;
                            drain_deadline =
                                Some(tokio::time::Instant::now() + DRAIN_TIMEOUT);
                            tracing::debug!(
                                "Soniox: sent finalize + empty-text EOA; draining (timeout {}s)",
                                DRAIN_TIMEOUT.as_secs(),
                            );
                        }
                    }
                }
            }

            // Incoming server messages.
            msg = read.next() => {
                let msg = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        let _ = events_tx.send(StreamingEvent::Error(
                            TranscribeError::InferenceFailed(format!("Soniox: WS error: {}", e))
                        )).await;
                        break;
                    }
                    None => break,
                };
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => break,
                    _ => continue,
                };
                tracing::debug!(target: "voxtype::soniox::wire", "<- {}", text);
                let parsed: ServerMessage = match serde_json::from_str(&text) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("Soniox: unparseable message ({}): {}", e, text);
                        continue;
                    }
                };

                // Suppress 408 (Request timeout) errors that arrive after
                // we've signalled end-of-audio. These come from the server's
                // own idle timer after the user pressed stop — surfacing them
                // as a "Streaming Error" notification is misleading UX.
                if sent_eoa && parsed.error_code == Some(408) {
                    tracing::debug!("Soniox: ignoring post-EOA 408 timeout");
                    break;
                }

                let out = reconciler.process(&parsed, type_partials);
                let mut send_failed = false;
                for ev in out.events {
                    if events_tx.send(ev).await.is_err() {
                        send_failed = true;
                        break;
                    }
                }
                // `finished:true` → reconciler sets terminate. That's the
                // documented end-of-session signal. The server closes the
                // socket right after; this break gets us out before the
                // close fires.
                if send_failed || out.terminate {
                    break;
                }
            }
        }
    }

    // Send a close frame on the way out. On the finished:true happy path the
    // server already initiated the close (so this may be a no-op against an
    // already-closing socket), but on cancel/drain-timeout/error paths an
    // explicit close is cleaner than letting the TCP teardown carry it.
    let _ = write.send(Message::Close(None)).await;

    let _ = events_tx.send(StreamingEvent::Ended).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_key(key: Option<&str>) -> SonioxConfig {
        SonioxConfig {
            api_key: key.map(|s| s.to_string()),
            model: "stt-rt-v4".into(),
            language_hints: vec!["hu".into(), "en".into()],
            language_hints_strict: true,
            streaming: true,
            type_partials: true,
            context: None,
            terms: None,
            terms_file: None,
            async_api: false,
            async_max_wait_secs: 120,
        }
    }

    #[test]
    fn async_api_disables_streaming_surface() {
        let mut cfg = cfg_with_key(Some("k"));
        cfg.async_api = true;
        let t = SonioxTranscriber::new(cfg).unwrap();
        assert!(t.as_streaming().is_none());
        assert_eq!(t.config.model, "stt-async-v4");
    }

    #[test]
    fn async_api_keeps_explicit_model_override() {
        let mut cfg = cfg_with_key(Some("k"));
        cfg.async_api = true;
        cfg.model = "stt-async-experimental".into();
        let t = SonioxTranscriber::new(cfg).unwrap();
        assert_eq!(t.config.model, "stt-async-experimental");
    }

    #[test]
    fn wav_encoding_has_correct_header() {
        let samples = vec![0.0_f32; 16000]; // 1s of silence
        let wav = encode_wav_s16le(&samples).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        let data_len = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_len, 32000);
        let sr = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
        assert_eq!(sr, 16000);
        assert_eq!(wav.len(), 44 + 32000);
    }

    #[test]
    fn requires_api_key_from_config_or_env() {
        std::env::remove_var("SONIOX_API_KEY");
        let err = SonioxTranscriber::new(cfg_with_key(None)).unwrap_err();
        assert!(matches!(err, TranscribeError::ConfigError(_)));
    }

    #[test]
    fn accepts_config_api_key() {
        let t = SonioxTranscriber::new(cfg_with_key(Some("test-key"))).unwrap();
        assert_eq!(t.api_key, "test-key");
    }

    #[test]
    fn init_frame_contains_required_fields() {
        let t = SonioxTranscriber::new(cfg_with_key(Some("my-key"))).unwrap();
        let frame: serde_json::Value = serde_json::from_str(&t.init_frame()).unwrap();
        assert_eq!(frame["api_key"], "my-key");
        assert_eq!(frame["model"], "stt-rt-v4");
        assert_eq!(frame["audio_format"], "pcm_s16le");
        assert_eq!(frame["sample_rate"], 16000);
        assert_eq!(frame["num_channels"], 1);
        assert_eq!(frame["language_hints"][0], "hu");
        assert_eq!(frame["language_hints"][1], "en");
        assert_eq!(frame["language_hints_strict"], true);
        assert_eq!(frame["enable_endpoint_detection"], true);
    }

    #[test]
    fn init_frame_honors_language_hints_strict_false() {
        let mut cfg = cfg_with_key(Some("k"));
        cfg.language_hints_strict = false;
        let t = SonioxTranscriber::new(cfg).unwrap();
        let frame: serde_json::Value = serde_json::from_str(&t.init_frame()).unwrap();
        assert_eq!(frame["language_hints_strict"], false);
    }

    #[test]
    fn init_frame_omits_strict_when_hints_empty() {
        let mut cfg = cfg_with_key(Some("k"));
        cfg.language_hints = vec![];
        let t = SonioxTranscriber::new(cfg).unwrap();
        let frame: serde_json::Value = serde_json::from_str(&t.init_frame()).unwrap();
        assert!(frame["language_hints_strict"].is_null());
    }

    #[test]
    fn init_frame_includes_context_text_when_set() {
        let mut cfg = cfg_with_key(Some("k"));
        cfg.context = Some("medical terminology".into());
        let t = SonioxTranscriber::new(cfg).unwrap();
        let frame: serde_json::Value = serde_json::from_str(&t.init_frame()).unwrap();
        assert_eq!(frame["context"]["text"], "medical terminology");
        assert!(frame["context"]["terms"].is_null());
    }

    #[test]
    fn init_frame_includes_context_terms_when_inline() {
        let mut cfg = cfg_with_key(Some("k"));
        cfg.terms = Some(vec!["Claude".into(), "voxtype".into()]);
        let t = SonioxTranscriber::new(cfg).unwrap();
        let frame: serde_json::Value = serde_json::from_str(&t.init_frame()).unwrap();
        let terms = frame["context"]["terms"].as_array().expect("terms array");
        assert_eq!(terms.len(), 2);
        assert_eq!(terms[0], "Claude");
        assert_eq!(terms[1], "voxtype");
    }

    #[test]
    fn init_frame_omits_context_when_no_text_or_terms() {
        let cfg = cfg_with_key(Some("k"));
        let t = SonioxTranscriber::new(cfg).unwrap();
        let frame: serde_json::Value = serde_json::from_str(&t.init_frame()).unwrap();
        assert!(
            frame.get("context").is_none(),
            "no context field expected when empty"
        );
    }

    #[test]
    fn load_context_terms_dedupes_and_trims() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("terms.json");
        std::fs::write(
            &path,
            r#"["Claude", "  voxtype  ", "Claude", "", "Hyprland"]"#,
        )
        .unwrap();
        let mut cfg = cfg_with_key(Some("k"));
        cfg.terms = Some(vec!["voxtype".into(), "Codecool".into()]);
        cfg.terms_file = Some(path);
        let t = SonioxTranscriber::new(cfg).unwrap();
        assert_eq!(
            t.context_terms,
            vec!["voxtype", "Codecool", "Claude", "Hyprland"],
        );
    }

    #[test]
    fn load_context_terms_errors_on_bad_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, r#"{"terms":["x"]}"#).unwrap();
        let mut cfg = cfg_with_key(Some("k"));
        cfg.terms_file = Some(path);
        let err = SonioxTranscriber::new(cfg).unwrap_err();
        assert!(matches!(err, TranscribeError::ConfigError(_)));
    }

    #[test]
    fn as_streaming_respects_streaming_flag() {
        let t = SonioxTranscriber::new(cfg_with_key(Some("k"))).unwrap();
        assert!(
            t.as_streaming().is_some(),
            "streaming=true should expose StreamingTranscriber"
        );

        let mut cfg = cfg_with_key(Some("k"));
        cfg.streaming = false;
        let t = SonioxTranscriber::new(cfg).unwrap();
        assert!(
            t.as_streaming().is_none(),
            "streaming=false should hide StreamingTranscriber"
        );
    }

    #[test]
    fn f32_to_s16le_round_trip_endpoints() {
        let samples = vec![-1.0_f32, 0.0, 1.0];
        let bytes = f32_to_s16le_bytes(&samples);
        assert_eq!(bytes.len(), 6);
        // -1.0 → -32767 (or -32768; clamp + round depends), 0 → 0, 1.0 → 32767
        let s0 = i16::from_le_bytes([bytes[0], bytes[1]]);
        let s1 = i16::from_le_bytes([bytes[2], bytes[3]]);
        let s2 = i16::from_le_bytes([bytes[4], bytes[5]]);
        assert!(s0 <= -32700, "-1.0 should map near i16::MIN, got {}", s0);
        assert_eq!(s1, 0);
        assert!(s2 >= 32700, "1.0 should map near i16::MAX, got {}", s2);
    }

    #[test]
    fn f32_to_s16le_clamps_out_of_range() {
        let samples = vec![-2.0_f32, 2.0];
        let bytes = f32_to_s16le_bytes(&samples);
        let s0 = i16::from_le_bytes([bytes[0], bytes[1]]);
        let s1 = i16::from_le_bytes([bytes[2], bytes[3]]);
        // Both clamped: -2.0 → -1.0 → near i16::MIN; 2.0 → 1.0 → near i16::MAX
        assert!(s0 <= -32700);
        assert!(s1 >= 32700);
    }

    #[test]
    fn split_tokens_preserves_order_and_separates_finality() {
        let toks = vec![
            Token {
                text: "Hello".into(),
                is_final: true,
            },
            Token {
                text: " ".into(),
                is_final: true,
            },
            Token {
                text: "world".into(),
                is_final: false,
            },
        ];
        let (finals, partials) = split_tokens(&toks);
        assert_eq!(finals, "Hello ");
        assert_eq!(partials, "world");
    }

    #[test]
    fn split_tokens_handles_all_final() {
        let toks = vec![
            Token {
                text: "abc".into(),
                is_final: true,
            },
            Token {
                text: "def".into(),
                is_final: true,
            },
        ];
        let (finals, partials) = split_tokens(&toks);
        assert_eq!(finals, "abcdef");
        assert_eq!(partials, "");
    }

    #[test]
    fn split_tokens_handles_empty() {
        let (finals, partials) = split_tokens(&[]);
        assert_eq!(finals, "");
        assert_eq!(partials, "");
    }

    // === Reconciler tests ===

    fn msg(tokens: Vec<(&str, bool)>, finished: bool) -> ServerMessage {
        ServerMessage {
            tokens: tokens
                .into_iter()
                .map(|(t, f)| Token {
                    text: t.to_string(),
                    is_final: f,
                })
                .collect(),
            finished,
            error_code: None,
            error_message: None,
        }
    }

    fn err_msg(code: i64, text: &str) -> ServerMessage {
        ServerMessage {
            tokens: vec![],
            finished: false,
            error_code: Some(code),
            error_message: Some(text.to_string()),
        }
    }

    fn extract(events: &[StreamingEvent]) -> Vec<(&'static str, String)> {
        events
            .iter()
            .map(|e| match e {
                StreamingEvent::Partial { text, .. } => ("Partial", text.clone()),
                StreamingEvent::Final { text, .. } => ("Final", text.clone()),
                StreamingEvent::Replace {
                    backspace, text, ..
                } => ("Replace", format!("-{}+{}", backspace, text)),
                StreamingEvent::Ended => ("Ended", String::new()),
                StreamingEvent::Error(e) => ("Error", e.to_string()),
            })
            .collect()
    }

    #[test]
    fn reconciler_initial_partial_emits_full_delta() {
        let mut r = Reconciler::default();
        let out = r.process(&msg(vec![("hel", false)], false), true);
        assert_eq!(extract(&out.events), vec![("Partial", "hel".to_string())]);
        assert!(!out.terminate);
        assert_eq!(r.typed_partial, "hel");
    }

    #[test]
    fn reconciler_partial_extension_emits_only_delta() {
        let mut r = Reconciler::default();
        r.process(&msg(vec![("hel", false)], false), true);
        let out = r.process(&msg(vec![("hello", false)], false), true);
        assert_eq!(extract(&out.events), vec![("Partial", "lo".to_string())]);
        assert_eq!(r.typed_partial, "hello");
    }

    #[test]
    fn reconciler_partial_divergence_drops_event() {
        let mut r = Reconciler::default();
        r.process(&msg(vec![("hello", false)], false), true);
        // Server revised: "hellp" doesn't start with "hello".
        let out = r.process(&msg(vec![("hellp", false)], false), true);
        assert!(
            out.events.is_empty(),
            "divergence should be silently dropped"
        );
        assert_eq!(
            r.typed_partial, "hello",
            "typed_partial unchanged on divergence"
        );
    }

    #[test]
    fn reconciler_final_strips_typed_partial_prefix() {
        let mut r = Reconciler::default();
        r.process(&msg(vec![("hello", false)], false), true);
        // Server now finalizes "hello world": prefix "hello" already typed.
        let out = r.process(&msg(vec![("hello world", true)], false), true);
        assert_eq!(extract(&out.events), vec![("Final", " world".to_string())]);
        assert_eq!(r.typed_partial, "");
    }

    #[test]
    fn reconciler_final_equals_typed_partial_emits_nothing() {
        let mut r = Reconciler::default();
        r.process(&msg(vec![("hello", false)], false), true);
        // Server finalizes exactly what was typed: no delta to type.
        let out = r.process(&msg(vec![("hello", true)], false), true);
        assert!(out.events.is_empty());
        assert_eq!(r.typed_partial, "");
    }

    #[test]
    fn reconciler_final_diverges_from_typed_partial_emits_replace() {
        let mut r = Reconciler::default();
        r.process(&msg(vec![("hello", false)], false), true);
        // Diverging final: emit Replace with full backspace + new text.
        let out = r.process(&msg(vec![("goodbye", true)], false), true);
        // LCP of "hello" and "goodbye" = 0 → backspace 5, type "goodbye"
        assert_eq!(
            extract(&out.events),
            vec![("Replace", "-5+goodbye".to_string())]
        );
        assert_eq!(r.typed_partial, "");
    }

    #[test]
    fn reconciler_punctuation_revision_emits_minimal_replace() {
        // Real-world case: Soniox finalized "tévedések." but the partial
        // had typed "tévedések,". LCP=9, backspace=1 (the comma),
        // type "."
        let mut r = Reconciler::default();
        r.process(&msg(vec![("tévedések,", false)], false), true);
        let out = r.process(&msg(vec![("tévedések.", true)], false), true);
        assert_eq!(extract(&out.events), vec![("Replace", "-1+.".to_string())]);
        assert_eq!(r.typed_partial, "");
    }

    #[test]
    fn reconciler_tail_revision_handles_unicode_scalars() {
        // Hungarian: "fejeztem" → "fejezte" (lose 'm', gain nothing) plus
        // " be a mondat" → " be a mondatot." (gain "ot.")
        let mut r = Reconciler::default();
        r.process(&msg(vec![("fejeztem be a mondat", false)], false), true);
        let out = r.process(&msg(vec![("fejezte be a mondatot.", true)], false), true);
        // LCP = "fejezte" (7 chars). typed has "m be a mondat" (13 chars after lcp).
        // final has " be a mondatot." (15 chars after lcp).
        // Backspace 13, type " be a mondatot."
        assert_eq!(
            extract(&out.events),
            vec![("Replace", "-13+ be a mondatot.".to_string())]
        );
    }

    #[test]
    fn common_prefix_counts_unicode_scalars_not_bytes() {
        // Hungarian "á" is 2 bytes in UTF-8 but 1 scalar.
        assert_eq!(common_prefix_char_count("áb", "ác"), 1);
        assert_eq!(common_prefix_char_count("hello", "hellp"), 4);
        assert_eq!(common_prefix_char_count("abc", "xyz"), 0);
        assert_eq!(common_prefix_char_count("", "anything"), 0);
        assert_eq!(common_prefix_char_count("same", "same"), 4);
    }

    #[test]
    fn reconciler_finished_terminates() {
        let mut r = Reconciler::default();
        let out = r.process(&msg(vec![("done", true)], true), true);
        assert!(out.terminate);
        assert_eq!(extract(&out.events), vec![("Final", "done".to_string())]);
    }

    #[test]
    fn reconciler_error_terminates_with_error_event() {
        let mut r = Reconciler::default();
        let out = r.process(&err_msg(401, "invalid api key"), true);
        assert!(out.terminate);
        assert_eq!(out.events.len(), 1);
        assert!(matches!(out.events[0], StreamingEvent::Error(_)));
    }

    #[test]
    fn reconciler_type_partials_false_skips_partial_emission() {
        let mut r = Reconciler::default();
        let out = r.process(&msg(vec![("hello", false)], false), false);
        assert!(
            out.events.is_empty(),
            "type_partials=false should emit no Partial"
        );
        assert_eq!(
            r.typed_partial, "",
            "typed_partial should not advance when not typing"
        );
    }

    #[test]
    fn reconciler_type_partials_false_still_emits_finals() {
        let mut r = Reconciler::default();
        let out = r.process(&msg(vec![("hello", true)], false), false);
        assert_eq!(extract(&out.events), vec![("Final", "hello".to_string())]);
    }

    #[test]
    fn reconciler_mixed_final_then_partial_in_one_message() {
        let mut r = Reconciler::default();
        let out = r.process(&msg(vec![("Hello ", true), ("world", false)], false), true);
        assert_eq!(
            extract(&out.events),
            vec![
                ("Final", "Hello ".to_string()),
                ("Partial", "world".to_string()),
            ]
        );
        assert_eq!(r.typed_partial, "world");
    }

    #[test]
    fn reconciler_final_is_prefix_of_typed_partial_emits_nothing() {
        // Soniox finalizes only the leading portion of what was typed.
        // The remainder stays as a pending partial. Cursor already shows
        // the correct text — no new typing needed.
        let mut r = Reconciler::default();
        r.process(&msg(vec![("Hello world foo", false)], false), true);
        let out = r.process(&msg(vec![("Hello world", true)], false), true);
        assert!(out.events.is_empty(), "final-is-prefix should emit nothing");
        assert_eq!(r.typed_partial, " foo");
    }

    #[test]
    fn reconciler_skips_end_token() {
        let mut r = Reconciler::default();
        let out = r.process(
            &msg(
                vec![("hello", true), ("<end>", true), (" world", false)],
                false,
            ),
            true,
        );
        // <end> filtered; only "hello" final + " world" partial.
        assert_eq!(
            extract(&out.events),
            vec![
                ("Final", "hello".to_string()),
                ("Partial", " world".to_string())
            ]
        );
    }

    #[test]
    fn reconciler_skips_lang_token() {
        let mut r = Reconciler::default();
        let out = r.process(&msg(vec![("<lang:en>", true), ("text", true)], false), true);
        assert_eq!(extract(&out.events), vec![("Final", "text".to_string())]);
    }

    #[test]
    fn is_special_token_recognizes_various_brackets() {
        assert!(is_special_token("<end>"));
        assert!(is_special_token("<fin>"));
        assert!(is_special_token("<lang:en>"));
        assert!(is_special_token("<speaker:1>"));
        assert!(!is_special_token("hello"));
        assert!(!is_special_token("<incomplete"));
        assert!(!is_special_token("incomplete>"));
        assert!(!is_special_token(""));
        // Punctuation tokens with brackets like "<3" should not match.
        assert!(!is_special_token("<3"));
    }

    #[test]
    fn reconciler_partial_after_final_extends_cleanly() {
        let mut r = Reconciler::default();
        r.process(&msg(vec![("Hello", true)], false), true);
        let out = r.process(&msg(vec![("world", false)], false), true);
        assert_eq!(extract(&out.events), vec![("Partial", "world".to_string())]);
        assert_eq!(r.typed_partial, "world");
    }
}
