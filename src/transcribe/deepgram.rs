//! Deepgram speech-to-text backend.
//!
//! Dictation streams over Deepgram's realtime WebSocket
//! ([`StreamingTranscriber`]) when `[deepgram] streaming` is on. A finished
//! recording (`voxtype transcribe`, meeting chunks, streaming recovery, and
//! dictation with `streaming = false`) goes to the pre-recorded endpoint in
//! one request ([`Transcriber::transcribe`]), which decodes it faster than
//! real time.
//!
//! ## Why finals-only
//!
//! Deepgram's interim results are *cumulative per segment*: each interim
//! restates the whole in-progress segment. The daemon's streaming session
//! treats `Partial` text as a delta to append, so passing interims through
//! would repeat text at the cursor. The session therefore asks for finals
//! only. Deepgram finalizes at endpointing boundaries, roughly once per
//! utterance, so text still arrives throughout a recording.

use super::audio::{encode_wav_s16le, pcm_s16le, SAMPLE_RATE};
use super::streaming::{SegmentId, StreamHandle, StreamingEvent, StreamingTranscriber};
use super::Transcriber;
use crate::config::DeepgramConfig;
use crate::error::TranscribeError;
use futures_util::{SinkExt, StreamExt};
use reqwest::Url;
use serde::Deserialize;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::header::{HeaderValue, AUTHORIZATION};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

/// Deepgram's limits on `keyterm`: up to 100 terms, and a request whose
/// keyterms come to more than 500 tokens is refused.
/// https://developers.deepgram.com/docs/keyterm
const KEYTERM_TOKEN_BUDGET: usize = 500;

/// Deepgram does not publish how it counts keyterm tokens: its staff put the
/// count closer to syllables than words, and 170 one-word terms, mostly
/// codes such as "EZS93AV", went over 500 (deepgram/deepgram-python-sdk#503).
/// Counting a token per two characters stays under the limit for both.
const KEYTERM_CHARS_PER_TOKEN: usize = 2;

/// Deepgram's limit on the number of terms, for `keyterm` and the older
/// `keywords` parameter alike.
/// https://developers.deepgram.com/docs/keywords
const VOCABULARY_TERM_LIMIT: usize = 100;

/// Deepgram drops audio in the first ~200-300 ms after the socket opens. A
/// short silence ahead of the microphone keeps the first word.
const SILENCE_PRIMER_MS: u64 = 300;

/// Control messages of the realtime API.
/// https://developers.deepgram.com/docs/finalize and
/// https://developers.deepgram.com/docs/close-stream
const FINALIZE_FRAME: &str = r#"{"type":"Finalize"}"#;
const CLOSE_STREAM_FRAME: &str = r#"{"type":"CloseStream"}"#;

#[derive(Debug)]
pub struct DeepgramTranscriber {
    config: DeepgramConfig,
    api_key: String,
    /// The pre-recorded endpoint, `https://` (or `http://` on loopback).
    rest_url: Url,
    /// The realtime endpoint: the same URL with a `wss://` (or `ws://`) scheme.
    ws_url: Url,
    /// Vocabulary for the next request, set per recording by the daemon.
    active_vocabulary: Mutex<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct DeepgramResponse {
    results: DeepgramResults,
}

#[derive(Debug, Deserialize)]
struct DeepgramResults {
    channels: Vec<DeepgramChannel>,
}

#[derive(Debug, Deserialize)]
struct DeepgramChannel {
    alternatives: Vec<DeepgramAlternative>,
}

#[derive(Debug, Deserialize)]
struct DeepgramAlternative {
    transcript: String,
}

/// One message from the realtime socket. Only `Results` and `Error` carry
/// anything the session acts on; `Metadata` parses with the defaults and is
/// skipped. (`SpeechStarted` and `UtteranceEnd` carry `channel` as an array
/// and would not parse, but the session never asks for them.)
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StreamMessage {
    #[serde(rename = "type")]
    kind: String,
    is_final: bool,
    /// Set on the result that answers a `Finalize`.
    from_finalize: bool,
    channel: Option<DeepgramChannel>,
    description: Option<String>,
    message: Option<String>,
}

impl StreamMessage {
    fn transcript(&self) -> Option<&str> {
        let text = self
            .channel
            .as_ref()?
            .alternatives
            .first()?
            .transcript
            .trim();
        (!text.is_empty()).then_some(text)
    }
}

/// Check a `[deepgram]` config the way the engine does at startup: the
/// endpoint, the model, language and timeout, and the API key. Logs
/// nothing, so `voxtype doctor` can call it too.
pub fn check_config(config: &DeepgramConfig) -> Result<(), TranscribeError> {
    checked(config).map(|_| ())
}

/// [`check_config`], returning what the engine needs: the key and the
/// pre-recorded and realtime endpoints.
fn checked(config: &DeepgramConfig) -> Result<(String, Url, Url), TranscribeError> {
    let (rest_url, ws_url) = endpoints(&config.endpoint)?;
    validate_config(config)?;
    let api_key = config.resolve_api_key().ok_or_else(|| {
        TranscribeError::ConfigError(
            "Deepgram API key required: set DEEPGRAM_API_KEY or [deepgram] api_key".into(),
        )
    })?;
    // Both APIs send the key in a header. A Deepgram key is visible ASCII:
    // the pre-recorded client (ureq) refuses a line break or a typographic
    // quote in a header, and whitespace inside a key cannot be part of it,
    // so such a key is refused here, at startup, rather than on the first
    // request. Whitespace around the key is already trimmed.
    if !api_key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(TranscribeError::ConfigError(
            "Deepgram API key contains characters that cannot be sent in an HTTP header. \
             Check VOXTYPE_DEEPGRAM_API_KEY, DEEPGRAM_API_KEY or [deepgram] api_key for \
             stray quotes, or spaces or line breaks inside the key."
                .into(),
        ));
    }
    Ok((api_key, rest_url, ws_url))
}

impl DeepgramTranscriber {
    pub fn new(config: DeepgramConfig) -> Result<Self, TranscribeError> {
        let (api_key, rest_url, ws_url) = checked(&config)?;

        tracing::info!(
            "Deepgram backend configured: endpoint={}, model={}, language={}, streaming={}, smart_format={}, mip_opt_out={}, timeout={}s",
            rest_url,
            config.model,
            config.language,
            config.streaming,
            config.smart_format,
            config.mip_opt_out,
            config.timeout_secs,
        );
        if config.streaming && config.language == "auto" {
            tracing::warn!(
                "Deepgram cannot detect the language of a live stream, so streamed dictation \
                 is transcribed as English; finished recordings still detect it. Set \
                 [deepgram] language to the language you speak, or \"multi\"."
            );
        }

        Ok(Self {
            config,
            api_key,
            rest_url,
            ws_url,
            active_vocabulary: Mutex::new(Vec::new()),
        })
    }

    /// Query parameters shared by both APIs, plus the realtime-only ones
    /// when `streaming`.
    fn query(&self, streaming: bool) -> Vec<(&'static str, String)> {
        let mut query = vec![
            ("model", self.config.model.clone()),
            ("smart_format", self.config.smart_format.to_string()),
            ("mip_opt_out", self.config.mip_opt_out.to_string()),
        ];
        match self.config.language.as_str() {
            // Deepgram detects the language of a finished recording only. A
            // live stream without `language` is transcribed as English.
            "auto" if streaming => {}
            "auto" => query.push(("detect_language", "true".into())),
            language => query.push(("language", language.into())),
        }
        if streaming {
            query.push(("encoding", "linear16".into()));
            query.push(("sample_rate", SAMPLE_RATE.to_string()));
            query.push(("channels", "1".into()));
            if let Some(ms) = self.config.endpointing_ms {
                query.push(("endpointing", ms.to_string()));
            }
        }

        let terms = self
            .active_vocabulary
            .lock()
            .map(|terms| terms.clone())
            .unwrap_or_default();
        if !terms.is_empty() {
            let keyterm_mode = uses_keyterms(&self.config.model);
            let kept = budget_terms(terms, keyterm_mode);
            tracing::debug!(
                terms = kept.len(),
                model = %self.config.model,
                "Applying vocabulary to Deepgram request"
            );
            let param = if keyterm_mode { "keyterm" } else { "keywords" };
            query.extend(kept.into_iter().map(|term| (param, term)));
        }
        query
    }

    fn request(&self) -> ureq::Request {
        let query = self.query(false);
        ureq::post(self.rest_url.as_str())
            .timeout(Duration::from_secs(self.config.timeout_secs))
            .set("Authorization", &format!("Token {}", self.api_key))
            .set("Content-Type", "audio/wav")
            .query_pairs(query.iter().map(|(key, value)| (*key, value.as_str())))
    }

    /// The realtime socket's handshake request: the endpoint with every
    /// query parameter, authenticated in the `Authorization` header so the
    /// key never appears in a URL.
    fn ws_request(&self) -> Result<Request, TranscribeError> {
        let mut url = self.ws_url.clone();
        url.query_pairs_mut().extend_pairs(self.query(true));
        let mut request = url.as_str().into_client_request().map_err(|e| {
            TranscribeError::ConfigError(format!("Invalid Deepgram endpoint URL: {}", e))
        })?;
        // `check_config` refuses such a key at startup; this is the fallback.
        let mut auth = HeaderValue::from_str(&format!("Token {}", self.api_key)).map_err(|_| {
            TranscribeError::ConfigError(
                "Deepgram API key contains characters that cannot be sent in an HTTP header".into(),
            )
        })?;
        auth.set_sensitive(true);
        request.headers_mut().insert(AUTHORIZATION, auth);
        Ok(request)
    }

    fn parse_response(body: &str) -> Result<String, TranscribeError> {
        let response: DeepgramResponse = serde_json::from_str(body).map_err(|e| {
            TranscribeError::RemoteError(format!("Deepgram returned malformed JSON: {}", e))
        })?;

        // An empty transcript is a recording with no words, which the
        // daemon and meeting mode handle as such; it is not an error.
        response
            .results
            .channels
            .first()
            .and_then(|channel| channel.alternatives.first())
            .map(|alternative| alternative.transcript.trim().to_string())
            .ok_or_else(|| {
                TranscribeError::RemoteError(
                    "Deepgram response contained no channel alternatives".into(),
                )
            })
    }
}

impl Transcriber for DeepgramTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".into()));
        }

        let wav = encode_wav_s16le(samples)?;
        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!(
            "Sending {:.2}s of audio to Deepgram ({} KiB WAV)",
            duration_secs,
            wav.len() / 1024
        );
        let started = std::time::Instant::now();

        let response = self.request().send_bytes(&wav).map_err(map_request_error)?;
        let body = response.into_string().map_err(|e| {
            TranscribeError::RemoteError(format!("Failed to read Deepgram response: {}", e))
        })?;
        let transcript = Self::parse_response(&body)?;

        tracing::info!(
            "Deepgram transcription completed in {:.2}s",
            started.elapsed().as_secs_f32()
        );
        Ok(transcript)
    }

    fn set_vocabulary(&self, terms: &[String]) {
        if let Ok(mut active) = self.active_vocabulary.lock() {
            active.clear();
            active.extend_from_slice(terms);
        }
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        self.config.streaming.then_some(self as _)
    }

    fn last_detected_language(&self) -> Option<String> {
        // The trait reports a two-letter code; Deepgram takes BCP-47 tags
        // such as "en-US", whose primary subtag is that code.
        match self.config.language.as_str() {
            "auto" | "multi" => None,
            language => language
                .split(['-', '_'])
                .next()
                .map(str::to_ascii_lowercase),
        }
    }
}

impl StreamingTranscriber for DeepgramTranscriber {
    fn start_stream(
        &self,
        samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        let request = self.ws_request()?;
        let timeout = Duration::from_secs(self.config.timeout_secs);
        let (events_tx, events_rx) = mpsc::channel::<StreamingEvent>(64);
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

        let task = tokio::spawn(async move {
            run_streaming_session(request, timeout, samples_rx, events_tx, cancel_rx).await
        });

        Ok(StreamHandle {
            events: events_rx,
            cancel: cancel_tx,
            task,
        })
    }
}

/// Emit `Error` then `Ended`, so the daemon reports the error and returns to
/// idle (or recovers the recording through the pre-recorded endpoint).
async fn send_fatal(events_tx: &mpsc::Sender<StreamingEvent>, error: TranscribeError) {
    let _ = events_tx.send(StreamingEvent::Error(error)).await;
    let _ = events_tx.send(StreamingEvent::Ended).await;
}

/// One dictation over the realtime socket.
///
/// Audio streams as 16-bit PCM while the daemon holds the samples channel
/// open. When it closes the channel, the session sends `Finalize`, so
/// Deepgram transcribes whatever it still holds and answers with one
/// `from_finalize` result, and then `CloseStream`, so Deepgram closes the
/// socket once every result is out. The session ends on whichever comes
/// first: the `from_finalize` result, usually within ~150 ms of the
/// microphone stopping, or the close.
async fn run_streaming_session(
    request: Request,
    timeout: Duration,
    mut samples_rx: mpsc::Receiver<Vec<f32>>,
    events_tx: mpsc::Sender<StreamingEvent>,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<(), TranscribeError> {
    let connect = tokio::time::timeout(timeout, tokio_tungstenite::connect_async(request));
    let ws_stream = tokio::select! {
        biased;
        _ = &mut cancel_rx => {
            let _ = events_tx.send(StreamingEvent::Ended).await;
            return Ok(());
        }
        result = connect => match result {
            Ok(Ok((stream, _))) => stream,
            Ok(Err(e)) => {
                send_fatal(&events_tx, connect_error(e)).await;
                return Ok(());
            }
            Err(_) => {
                send_fatal(
                    &events_tx,
                    TranscribeError::NetworkError(format!(
                        "Deepgram connection timed out after {}s",
                        timeout.as_secs()
                    )),
                )
                .await;
                return Ok(());
            }
        },
    };
    let (mut write, mut read) = ws_stream.split();

    let mut fatal = write
        .send(Message::Binary(silence_primer()))
        .await
        .err()
        .map(|e| TranscribeError::NetworkError(format!("Deepgram: send audio failed: {}", e)));
    let mut next_segment: SegmentId = 0;
    // Set when the daemon closes the samples channel; the drain deadline
    // counts from it.
    let mut stopped_at: Option<tokio::time::Instant> = None;
    let mut finals_after_stop: u32 = 0;

    while fatal.is_none() {
        let samples_closed = stopped_at.is_some();
        let drain_deadline = stopped_at.map(|stopped| stopped + timeout);
        let drain_timer = async move {
            match drain_deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            biased;

            _ = &mut cancel_rx => {
                tracing::debug!("Deepgram streaming session cancelled");
                break;
            }

            _ = drain_timer, if samples_closed => {
                fatal = Some(TranscribeError::RemoteError(format!(
                    "Deepgram did not finish within {}s after recording stopped",
                    timeout.as_secs()
                )));
            }

            message = read.next() => {
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(frame))) => {
                        // After the stop a normal close ends the stream; any
                        // other code (1011 is Deepgram's internal error) is
                        // reported, so recovery can run.
                        let abnormal = frame.as_ref().is_some_and(|f| f.code != CloseCode::Normal);
                        if !samples_closed || abnormal {
                            fatal = Some(closed(frame, samples_closed));
                        }
                        break;
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        fatal = Some(TranscribeError::NetworkError(format!(
                            "Deepgram connection error: {}",
                            e
                        )));
                        break;
                    }
                    None => {
                        if !samples_closed {
                            fatal = Some(closed(None, false));
                        }
                        break;
                    }
                };
                tracing::debug!(target: "voxtype::deepgram::wire", "<- {}", text);
                let parsed: StreamMessage = match serde_json::from_str(&text) {
                    Ok(parsed) => parsed,
                    Err(e) => {
                        // The message is on the wire trace above; it can
                        // carry transcribed words, so it is not logged here.
                        tracing::warn!("Deepgram: unparseable message ({}), {} bytes", e, text.len());
                        continue;
                    }
                };
                match parsed.kind.as_str() {
                    "Results" => {
                        if let Some(text) = parsed.transcript().filter(|_| parsed.is_final) {
                            // Finals are trimmed, complete segments. A leading
                            // space keeps "Hello world." and "How are you?"
                            // from running together.
                            let text = if next_segment == 0 {
                                text.to_string()
                            } else {
                                format!(" {}", text)
                            };
                            let segment_id = next_segment;
                            next_segment += 1;
                            if samples_closed {
                                finals_after_stop += 1;
                            }
                            if events_tx
                                .send(StreamingEvent::Final { text, segment_id })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        if samples_closed && parsed.from_finalize {
                            break;
                        }
                    }
                    "Error" => {
                        fatal = Some(TranscribeError::RemoteError(format!(
                            "Deepgram error: {}",
                            parsed
                                .description
                                .or(parsed.message)
                                .unwrap_or_else(|| "no description".into())
                        )));
                    }
                    _ => {}
                }
            }

            chunk = samples_rx.recv(), if !samples_closed => match chunk {
                Some(samples) if !samples.is_empty() => {
                    if let Err(e) = write.send(Message::Binary(pcm_s16le(&samples))).await {
                        fatal = Some(TranscribeError::NetworkError(format!(
                            "Deepgram: send audio failed: {}",
                            e
                        )));
                    }
                }
                Some(_) => {}
                None => {
                    stopped_at = Some(tokio::time::Instant::now());
                    for frame in [FINALIZE_FRAME, CLOSE_STREAM_FRAME] {
                        if let Err(e) = write.send(Message::Text(frame.into())).await {
                            fatal = Some(TranscribeError::NetworkError(format!(
                                "Deepgram: ending the stream failed: {}",
                                e
                            )));
                            break;
                        }
                    }
                }
            },
        }
    }

    // Release the connection. After a stop `CloseStream` already went out.
    // When the session gets here before the stop (an error, or a cancel the
    // daemon has not aborted yet) it tells Deepgram to end the stream; an
    // aborted task just drops the socket, which ends it too.
    if stopped_at.is_none() {
        let _ = write.send(Message::Text(CLOSE_STREAM_FRAME.into())).await;
    }
    let _ = write.send(Message::Close(None)).await;

    if let Some(error) = fatal {
        send_fatal(&events_tx, error).await;
        return Ok(());
    }
    if let Some(stopped_at) = stopped_at {
        tracing::info!(
            drain_ms = stopped_at.elapsed().as_millis() as u64,
            finals_after_stop,
            "Deepgram stream finished"
        );
    }
    let _ = events_tx.send(StreamingEvent::Ended).await;
    Ok(())
}

/// The error for a socket Deepgram closed while audio was still being sent
/// (an idle timeout, an undecodable stream or a revoked key end it this way),
/// or closed with an error code after the stop.
fn closed(frame: Option<CloseFrame<'_>>, after_stop: bool) -> TranscribeError {
    let when = if after_stop {
        "after recording stopped"
    } else {
        "during recording"
    };
    TranscribeError::RemoteError(match frame {
        Some(frame) if !frame.reason.is_empty() => format!(
            "Deepgram closed the connection {when} ({}: {})",
            u16::from(frame.code),
            frame.reason
        ),
        Some(frame) => format!(
            "Deepgram closed the connection {when} ({})",
            u16::from(frame.code)
        ),
        None => format!("Deepgram closed the connection {when}"),
    })
}

/// The handshake error, with the same categories as a failed pre-recorded
/// request, so a bad key reads the same on both paths.
fn connect_error(error: tokio_tungstenite::tungstenite::Error) -> TranscribeError {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            let detail = response
                .body()
                .as_deref()
                .map(|body| truncate_detail(&String::from_utf8_lossy(body)))
                .filter(|detail| !detail.is_empty());
            TranscribeError::RemoteError(status_message(response.status().as_u16(), detail))
        }
        other => TranscribeError::NetworkError(format!("Deepgram connection failed: {}", other)),
    }
}

/// `SILENCE_PRIMER_MS` of 16 kHz 16-bit mono silence.
fn silence_primer() -> Vec<u8> {
    vec![0u8; (SILENCE_PRIMER_MS * SAMPLE_RATE as u64 / 1000 * 2) as usize]
}

/// nova-3 takes `keyterm`; older models (nova-2, nova, enhanced, base) take
/// `keywords`. nova-3 refuses `keywords` with a 400, so a request carries
/// one or the other, never both. (Flux also takes `keyterm`, but only on
/// `/v2/listen`, which this engine does not use.)
fn uses_keyterms(model: &str) -> bool {
    model.starts_with("nova-3")
}

/// The tokens Deepgram is likely to count for one keyterm, rounded up: at
/// least one per word, and one per `KEYTERM_CHARS_PER_TOKEN` characters.
fn estimated_tokens(term: &str) -> usize {
    let words = term.split_whitespace().count();
    let chars = term.chars().filter(|c| !c.is_whitespace()).count();
    words.max(chars.div_ceil(KEYTERM_CHARS_PER_TOKEN)).max(1)
}

/// Keep the vocabulary within Deepgram's limits: 100 terms, and for
/// `keyterm` an estimated 500 tokens. Terms past a limit are dropped, and
/// named in a warning.
fn budget_terms(terms: Vec<String>, keyterm_mode: bool) -> Vec<String> {
    let mut used = 0usize;
    let mut kept: Vec<String> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for term in terms {
        let cost = if keyterm_mode {
            estimated_tokens(&term)
        } else {
            0
        };
        if kept.len() < VOCABULARY_TERM_LIMIT && used + cost <= KEYTERM_TOKEN_BUDGET {
            used += cost;
            kept.push(term);
        } else {
            dropped.push(term);
        }
    }
    if !dropped.is_empty() {
        tracing::warn!(
            "Deepgram vocabulary exceeds the {} limit; dropped: {}",
            if keyterm_mode {
                format!("{VOCABULARY_TERM_LIMIT}-term, {KEYTERM_TOKEN_BUDGET}-token keyterm")
            } else {
                format!("{VOCABULARY_TERM_LIMIT}-keyword")
            },
            dropped.join(", ")
        );
    }
    kept
}

/// Both endpoints from the configured URL. The documented form is the
/// pre-recorded `https://…/v1/listen`; the realtime API is the same URL over
/// `wss://`, so a `wss://` endpoint is accepted too and selects the same
/// pair.
fn endpoints(endpoint: &str) -> Result<(Url, Url), TranscribeError> {
    let mut rest = Url::parse(endpoint.trim()).map_err(|e| {
        TranscribeError::ConfigError(format!("Invalid Deepgram endpoint URL: {}", e))
    })?;
    if !rest.username().is_empty() || rest.password().is_some() {
        return Err(TranscribeError::ConfigError(
            "Deepgram endpoint must not contain credentials".into(),
        ));
    }
    let is_loopback = rest.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    let secure = match rest.scheme() {
        "https" | "wss" => true,
        "http" | "ws" if is_loopback => false,
        _ => {
            return Err(TranscribeError::ConfigError(
                "Deepgram endpoint must use HTTPS (HTTP is allowed only for localhost tests)"
                    .into(),
            ))
        }
    };
    // ws, wss, http and https are all special schemes, so switching between
    // them cannot fail.
    let _ = rest.set_scheme(if secure { "https" } else { "http" });
    let mut ws = rest.clone();
    let _ = ws.set_scheme(if secure { "wss" } else { "ws" });
    Ok((rest, ws))
}

fn validate_config(config: &DeepgramConfig) -> Result<(), TranscribeError> {
    if config.model.trim().is_empty() {
        return Err(TranscribeError::ConfigError(
            "Deepgram model must not be empty".into(),
        ));
    }
    if config.language.trim().is_empty() {
        return Err(TranscribeError::ConfigError(
            "Deepgram language must not be empty".into(),
        ));
    }
    if config.timeout_secs == 0 {
        return Err(TranscribeError::ConfigError(
            "Deepgram timeout_secs must be greater than zero".into(),
        ));
    }
    Ok(())
}

/// The error text for an HTTP status from either API, so a bad key reads
/// the same whether the pre-recorded request or the socket handshake hit it.
fn status_message(code: u16, detail: Option<String>) -> String {
    let summary = match code {
        401 => "authentication failed",
        // https://developers.deepgram.com/docs/errors
        403 => "access denied: the key's project has no access to this model",
        402 => "account has insufficient credit",
        429 => "rate limit exceeded",
        _ => "request failed",
    };
    match detail {
        Some(detail) => format!("Deepgram {} (HTTP {}): {}", summary, code, detail),
        None => format!("Deepgram {} (HTTP {})", summary, code),
    }
}

fn map_request_error(error: ureq::Error) -> TranscribeError {
    match error {
        ureq::Error::Status(code, response) => {
            let detail = response.into_string().unwrap_or_default();
            TranscribeError::RemoteError(status_message(
                code,
                (!detail.is_empty()).then(|| truncate_detail(&detail)),
            ))
        }
        ureq::Error::Transport(error) => {
            TranscribeError::NetworkError(format!("Deepgram request failed: {}", error))
        }
    }
}

fn truncate_detail(detail: &str) -> String {
    const MAX_CHARS: usize = 500;
    let trimmed = detail.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        trimmed.to_string()
    } else {
        format!("{}…", trimmed.chars().take(MAX_CHARS).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serializes the tests that read or change the API key variables.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A transcriber whose key comes from `config`, whatever the environment
    /// of other tests holds.
    fn new_transcriber(config: DeepgramConfig) -> DeepgramTranscriber {
        let _guard = env_lock();
        clear_key_env();
        DeepgramTranscriber::new(config).unwrap()
    }

    fn clear_key_env() {
        for var in crate::config::DEEPGRAM_API_KEY_ENV_VARS {
            std::env::remove_var(var);
        }
    }

    fn config_with_key() -> DeepgramConfig {
        DeepgramConfig {
            api_key: Some("config-key".into()),
            ..Default::default()
        }
    }

    fn query_value<'a>(query: &'a [(&'static str, String)], key: &str) -> Vec<&'a str> {
        query
            .iter()
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    fn spawn_server(
        status: &str,
        response_body: &str,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_string();
        let response_body = response_body.to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut headers = [0u8; 4096];
            let read = stream.read(&mut headers).unwrap();
            request.extend_from_slice(&headers[..read]);

            let header_end = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| position + 4)
                .unwrap();
            let header_text = String::from_utf8_lossy(&request[..header_end]);
            let content_length = header_text
                .lines()
                .find_map(|line| {
                    line.strip_prefix("Content-Length: ")
                        .or_else(|| line.strip_prefix("content-length: "))
                })
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while request.len() - header_end < content_length {
                let mut chunk = [0u8; 4096];
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
            }

            let response = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (format!("http://{}/v1/listen", address), handle)
    }

    #[test]
    fn environment_key_takes_precedence() {
        let _guard = env_lock();
        clear_key_env();
        std::env::set_var("DEEPGRAM_API_KEY", "environment-key");
        let transcriber = DeepgramTranscriber::new(config_with_key()).unwrap();
        assert_eq!(transcriber.api_key, "environment-key");

        std::env::set_var("VOXTYPE_DEEPGRAM_API_KEY", "voxtype-key");
        let transcriber = DeepgramTranscriber::new(config_with_key()).unwrap();
        clear_key_env();
        assert_eq!(transcriber.api_key, "voxtype-key");
    }

    #[test]
    fn a_blank_variable_does_not_hide_the_next_one() {
        let _guard = env_lock();
        clear_key_env();
        std::env::set_var("VOXTYPE_DEEPGRAM_API_KEY", " ");
        std::env::set_var("DEEPGRAM_API_KEY", "environment-key");
        let result = DeepgramTranscriber::new(DeepgramConfig::default());
        clear_key_env();
        assert_eq!(result.unwrap().api_key, "environment-key");
    }

    /// Every `[deepgram]` field has a default and the key can come from the
    /// environment, so a config with no table still builds the engine.
    #[test]
    fn a_missing_table_runs_on_the_defaults() {
        let _guard = env_lock();
        clear_key_env();
        std::env::set_var("DEEPGRAM_API_KEY", "environment-key");
        let config = crate::config::Config {
            engine: crate::config::TranscriptionEngine::Deepgram,
            deepgram: None,
            ..Default::default()
        };
        let result = crate::transcribe::create_transcriber(&config);
        clear_key_env();
        assert!(result.is_ok(), "{:?}", result.err());
    }

    #[test]
    fn config_key_is_fallback() {
        let _guard = env_lock();
        clear_key_env();
        let transcriber = DeepgramTranscriber::new(config_with_key()).unwrap();
        assert_eq!(transcriber.api_key, "config-key");
    }

    #[test]
    fn missing_key_is_rejected() {
        let _guard = env_lock();
        clear_key_env();
        let error = DeepgramTranscriber::new(DeepgramConfig::default()).unwrap_err();
        assert!(error.to_string().contains("DEEPGRAM_API_KEY"));
    }

    #[test]
    fn endpoint_requires_https_except_localhost() {
        let mut config = config_with_key();
        config.endpoint = "http://example.com/v1/listen".into();
        let error = DeepgramTranscriber::new(config).unwrap_err();
        assert!(error.to_string().contains("HTTPS"));

        let mut insecure_socket = config_with_key();
        insecure_socket.endpoint = "ws://example.com/v1/listen".into();
        assert!(DeepgramTranscriber::new(insecure_socket).is_err());

        let mut lookalike = config_with_key();
        lookalike.endpoint = "http://localhost.example.com/v1/listen".into();
        assert!(DeepgramTranscriber::new(lookalike).is_err());

        let mut credentials = config_with_key();
        credentials.endpoint = "https://user:password@api.deepgram.com/v1/listen".into();
        assert!(DeepgramTranscriber::new(credentials)
            .unwrap_err()
            .to_string()
            .contains("must not contain credentials"));
    }

    #[test]
    fn either_scheme_selects_both_apis() {
        let (rest, ws) = endpoints("https://api.deepgram.com/v1/listen").unwrap();
        assert_eq!(rest.as_str(), "https://api.deepgram.com/v1/listen");
        assert_eq!(ws.as_str(), "wss://api.deepgram.com/v1/listen");

        let (rest, ws) = endpoints("wss://api.eu.deepgram.com/v1/listen").unwrap();
        assert_eq!(rest.as_str(), "https://api.eu.deepgram.com/v1/listen");
        assert_eq!(ws.as_str(), "wss://api.eu.deepgram.com/v1/listen");

        let (rest, ws) = endpoints("http://127.0.0.1:8080/v1/listen").unwrap();
        assert_eq!(rest.as_str(), "http://127.0.0.1:8080/v1/listen");
        assert_eq!(ws.as_str(), "ws://127.0.0.1:8080/v1/listen");
    }

    #[test]
    fn keyterm_models_selected_by_prefix() {
        assert!(uses_keyterms("nova-3"));
        assert!(uses_keyterms("nova-3-medical"));
        assert!(!uses_keyterms("nova-2"));
        assert!(!uses_keyterms("enhanced"));
        assert!(!uses_keyterms("base"));
    }

    #[test]
    fn keyterms_stop_at_100_terms() {
        let terms: Vec<String> = (0..150).map(|i| format!("t{i}")).collect();
        let kept = budget_terms(terms, true);
        assert_eq!(kept.len(), 100);
        assert_eq!(kept[0], "t0");
    }

    #[test]
    fn keyterm_tokens_are_estimated_from_characters() {
        // 30 characters each, fifteen estimated tokens: 33 fit in 500.
        let terms: Vec<String> = (0..60).map(|i| format!("{i:0>30}")).collect();
        let kept = budget_terms(terms, true);
        assert_eq!(kept.len(), 33);
        // Each word is at least one token.
        assert_eq!(estimated_tokens("a b c d"), 4);
        assert_eq!(estimated_tokens("Hyprland"), 4);
    }

    #[test]
    fn budget_caps_keywords_at_100_terms() {
        let terms: Vec<String> = (0..150).map(|i| format!("term number{i}")).collect();
        let kept = budget_terms(terms, false);
        assert_eq!(kept.len(), 100);
        assert_eq!(kept[0], "term number0");
    }

    #[test]
    fn vocabulary_goes_to_the_parameter_the_model_takes() {
        let nova3 = new_transcriber(config_with_key());
        let terms = vec!["Voxtype".to_string(), "Hyprland".to_string()];
        assert!(query_value(&nova3.query(true), "keyterm").is_empty());
        nova3.set_vocabulary(&terms);
        for streaming in [true, false] {
            let query = nova3.query(streaming);
            assert_eq!(query_value(&query, "keyterm"), ["Voxtype", "Hyprland"]);
            assert!(query_value(&query, "keywords").is_empty());
        }

        let nova2 = new_transcriber(DeepgramConfig {
            model: "nova-2".into(),
            ..config_with_key()
        });
        nova2.set_vocabulary(&terms);
        let query = nova2.query(true);
        assert_eq!(query_value(&query, "keywords"), ["Voxtype", "Hyprland"]);
        assert!(query_value(&query, "keyterm").is_empty());

        // The next recording's vocabulary replaces this one's.
        nova2.set_vocabulary(&[]);
        assert!(query_value(&nova2.query(true), "keywords").is_empty());
    }

    #[test]
    fn streaming_query_describes_the_audio_and_never_detects_language() {
        let transcriber = new_transcriber(DeepgramConfig {
            language: "auto".into(),
            endpointing_ms: Some(300),
            ..config_with_key()
        });
        let live = transcriber.query(true);
        assert_eq!(query_value(&live, "encoding"), ["linear16"]);
        assert_eq!(query_value(&live, "sample_rate"), ["16000"]);
        assert_eq!(query_value(&live, "channels"), ["1"]);
        assert_eq!(query_value(&live, "endpointing"), ["300"]);
        assert!(query_value(&live, "detect_language").is_empty());
        assert!(query_value(&live, "language").is_empty());

        let recorded = transcriber.query(false);
        assert_eq!(query_value(&recorded, "detect_language"), ["true"]);
        assert!(query_value(&recorded, "encoding").is_empty());
        assert!(query_value(&recorded, "endpointing").is_empty());
    }

    #[test]
    fn ws_request_carries_the_key_in_a_header_only() {
        let transcriber = new_transcriber(DeepgramConfig {
            api_key: Some("ws-secret".into()),
            ..Default::default()
        });
        let request = transcriber.ws_request().unwrap();
        assert_eq!(request.headers()[AUTHORIZATION], "Token ws-secret");
        let uri = request.uri().to_string();
        assert!(
            uri.starts_with("wss://api.deepgram.com/v1/listen?"),
            "{uri}"
        );
        assert!(uri.contains("model=nova-3"), "{uri}");
        assert!(!uri.contains("ws-secret"), "{uri}");
    }

    /// Whitespace around a key is not part of it: a trailing space, `\n` or
    /// `\r\n` from a pasted key or a key file is trimmed, and the trimmed key
    /// is sent. A key that still cannot be sent in a header is refused at
    /// startup, for both APIs, with where to look: whitespace inside it, or
    /// a typographic quote, which the pre-recorded client refuses although
    /// a WebSocket header would take it.
    #[test]
    fn keys_are_trimmed_and_unsendable_keys_are_refused_at_startup() {
        let _guard = env_lock();
        clear_key_env();
        for key in ["abc\n", "abc\r\n", "abc ", " abc"] {
            let transcriber = DeepgramTranscriber::new(DeepgramConfig {
                api_key: Some(key.into()),
                ..Default::default()
            })
            .unwrap();
            assert_eq!(transcriber.api_key, "abc", "{key:?}");
        }
        for key in ["ab c", "ab\nc", "\u{201c}abc\u{201d}"] {
            let error = DeepgramTranscriber::new(DeepgramConfig {
                api_key: Some(key.into()),
                ..Default::default()
            })
            .unwrap_err();
            assert!(matches!(error, TranscribeError::ConfigError(_)), "{key:?}");
            assert!(
                error.to_string().contains("stray quotes"),
                "{key:?}: {error}"
            );
        }
        assert!(check_config(&config_with_key()).is_ok());
    }

    /// The trait reports a two-letter code; a BCP-47 tag gives its primary
    /// subtag, and automatic or multilingual detection gives none.
    #[test]
    fn the_reported_language_is_the_primary_subtag() {
        let language = |code: &str| {
            new_transcriber(DeepgramConfig {
                language: code.into(),
                ..config_with_key()
            })
            .last_detected_language()
        };
        assert_eq!(language("en-US").as_deref(), Some("en"));
        assert_eq!(language("fr").as_deref(), Some("fr"));
        assert_eq!(language("auto"), None);
        assert_eq!(language("multi"), None);
    }

    #[test]
    fn as_streaming_follows_the_streaming_setting() {
        let streaming = new_transcriber(config_with_key());
        assert!(streaming.as_streaming().is_some());
        let batch = new_transcriber(DeepgramConfig {
            streaming: false,
            ..config_with_key()
        });
        assert!(batch.as_streaming().is_none());
    }

    #[test]
    fn parses_transcript_and_accepts_an_empty_one() {
        let body =
            r#"{"results":{"channels":[{"alternatives":[{"transcript":" hello world "}]}]}}"#;
        assert_eq!(
            DeepgramTranscriber::parse_response(body).unwrap(),
            "hello world"
        );

        // Silence: the daemon logs "Transcription was empty" and types nothing.
        let empty = r#"{"results":{"channels":[{"alternatives":[{"transcript":""}]}]}}"#;
        assert_eq!(DeepgramTranscriber::parse_response(empty).unwrap(), "");
    }

    #[test]
    fn malformed_and_missing_results_are_rejected() {
        assert!(DeepgramTranscriber::parse_response("not json").is_err());
        assert!(DeepgramTranscriber::parse_response(
            r#"{"results":{"channels":[{"alternatives":[]}]}}"#
        )
        .is_err());
    }

    #[test]
    fn request_contains_expected_headers_query_and_wav() {
        let body = r#"{"results":{"channels":[{"alternatives":[{"transcript":"test"}]}]}}"#;
        let (endpoint, server) = spawn_server("200 OK", body);
        let transcriber = new_transcriber(DeepgramConfig {
            endpoint,
            api_key: Some("secret-test-key".into()),
            language: "auto".into(),
            ..Default::default()
        });
        transcriber.set_vocabulary(&["Voxtype".to_string()]);

        let transcript = transcriber.transcribe(&[0.0; 160]).unwrap();
        assert_eq!(transcript, "test");
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/listen?"));
        assert!(request.contains("model=nova-3"));
        assert!(request.contains("smart_format=true"));
        assert!(request.contains("mip_opt_out=true"));
        assert!(request.contains("detect_language=true"));
        assert!(request.contains("keyterm=Voxtype"));
        assert!(!request.contains("&language="));
        assert!(!request.contains("encoding="));
        assert!(request
            .to_ascii_lowercase()
            .contains("content-type: audio/wav"));
        assert!(request.contains("Authorization: Token secret-test-key"));
        assert!(request.contains("RIFF"));
        assert!(request.contains("WAVE"));
    }

    #[test]
    fn http_errors_have_actionable_categories() {
        let (endpoint, server) =
            spawn_server("429 Too Many Requests", r#"{"err_code":"TooManyRequests"}"#);
        let transcriber = new_transcriber(DeepgramConfig {
            endpoint,
            api_key: Some("key".into()),
            ..Default::default()
        });
        let error = transcriber.transcribe(&[0.0; 16]).unwrap_err();
        server.join().unwrap();
        assert!(error.to_string().contains("rate limit exceeded"));
        assert!(error.to_string().contains("429"));
    }

    #[test]
    fn request_timeout_is_a_network_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/listen", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(2));
        });
        let transcriber = new_transcriber(DeepgramConfig {
            endpoint,
            api_key: Some("key".into()),
            timeout_secs: 1,
            ..Default::default()
        });

        let error = transcriber.transcribe(&[0.0; 16]).unwrap_err();
        server.join().unwrap();
        assert!(matches!(error, TranscribeError::NetworkError(_)));
    }

    /// How the fake realtime server answers the stream.
    #[derive(Clone, Copy)]
    enum Script {
        /// One final while audio flows, and the answer to `Finalize`.
        Finalize,
        /// No answer to `Finalize`: the last final arrives after
        /// `CloseStream`, followed by the close.
        CloseOnly,
        /// The socket closes after the first audio frame.
        CloseEarly,
        /// The handshake is refused with 401.
        Unauthorized,
        /// Deepgram reports an error after the first audio frame.
        DeepgramError,
        /// One final while audio flows; the stop is never answered and the
        /// socket stays open.
        Silent,
        /// One final while audio flows; the stop is answered by a close with
        /// Deepgram's internal-error code.
        CloseWithError,
    }

    /// What the fake server saw during one session.
    #[derive(Default, Debug)]
    struct Seen {
        authorization: Option<String>,
        query: String,
        audio_bytes: usize,
        control: Vec<String>,
    }

    const FIRST_FINAL: &str = r#"{"type":"Results","is_final":true,"from_finalize":false,"channel":{"alternatives":[{"transcript":"Hello world."}]}}"#;
    const INTERIM: &str = r#"{"type":"Results","is_final":false,"channel":{"alternatives":[{"transcript":"How are"}]}}"#;
    const LAST_FINAL: &str = r#"{"type":"Results","is_final":true,"from_finalize":FROM,"channel":{"alternatives":[{"transcript":"How are you?"}]}}"#;

    /// A one-connection fake of Deepgram's realtime API on localhost.
    // tungstenite's handshake callback signature returns its large
    // `ErrorResponse`; not ours to shrink.
    #[allow(clippy::result_large_err)]
    async fn fake_deepgram(script: Script) -> (String, tokio::task::JoinHandle<Seen>) {
        use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
        use tokio_tungstenite::tungstenite::http::StatusCode;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/listen", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let seen = std::sync::Arc::new(Mutex::new(Seen::default()));
            let seen_cb = seen.clone();
            let accepted = tokio_tungstenite::accept_hdr_async(
                stream,
                move |request: &Request, response: Response| {
                    let mut seen = seen_cb.lock().unwrap();
                    seen.authorization = request
                        .headers()
                        .get("Authorization")
                        .map(|v| v.to_str().unwrap().to_string());
                    seen.query = request.uri().query().unwrap_or_default().to_string();
                    if matches!(script, Script::Unauthorized) {
                        let mut refusal = ErrorResponse::new(Some("INVALID_AUTH".into()));
                        *refusal.status_mut() = StatusCode::UNAUTHORIZED;
                        return Err(refusal);
                    }
                    Ok(response)
                },
            )
            .await;
            let Ok(mut ws) = accepted else {
                return std::mem::take(&mut *seen.lock().unwrap());
            };
            let mut answered_audio = false;
            while let Some(Ok(message)) = ws.next().await {
                match message {
                    Message::Binary(bytes) => {
                        seen.lock().unwrap().audio_bytes += bytes.len();
                        if answered_audio {
                            continue;
                        }
                        answered_audio = true;
                        match script {
                            Script::CloseEarly => {
                                let _ = ws.close(None).await;
                                break;
                            }
                            Script::DeepgramError => {
                                let error =
                                    r#"{"type":"Error","description":"NET-0001: no audio"}"#;
                                ws.send(Message::Text(error.into())).await.unwrap();
                            }
                            _ => {
                                ws.send(Message::Text(FIRST_FINAL.into())).await.unwrap();
                                ws.send(Message::Text(INTERIM.into())).await.unwrap();
                            }
                        }
                    }
                    Message::Text(text) => {
                        seen.lock().unwrap().control.push(text.clone());
                        if text == FINALIZE_FRAME && matches!(script, Script::Finalize) {
                            ws.send(Message::Text(LAST_FINAL.replace("FROM", "true")))
                                .await
                                .unwrap();
                        }
                        if text == CLOSE_STREAM_FRAME && matches!(script, Script::CloseWithError) {
                            let frame = CloseFrame {
                                code: CloseCode::Error,
                                reason: "NET-0000".into(),
                            };
                            let _ = ws.close(Some(frame)).await;
                            break;
                        }
                        if text == CLOSE_STREAM_FRAME && matches!(script, Script::CloseOnly) {
                            ws.send(Message::Text(LAST_FINAL.replace("FROM", "false")))
                                .await
                                .unwrap();
                            ws.send(Message::Text(r#"{"type":"Metadata"}"#.into()))
                                .await
                                .unwrap();
                            let _ = ws.close(None).await;
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            // A named result drops the guard before `seen` itself goes.
            let result = std::mem::take(&mut *seen.lock().unwrap());
            result
        });
        (url, handle)
    }

    /// Run one dictation of `chunks` 100 ms chunks against `script` and
    /// return the finals, the error (if any) and what the server saw.
    async fn dictate(script: Script, chunks: usize) -> (Vec<String>, Option<String>, Seen) {
        dictate_within(script, chunks, 5).await
    }

    async fn dictate_within(
        script: Script,
        chunks: usize,
        timeout_secs: u64,
    ) -> (Vec<String>, Option<String>, Seen) {
        let (endpoint, server) = fake_deepgram(script).await;
        let transcriber = new_transcriber(DeepgramConfig {
            endpoint,
            api_key: Some("stream-key".into()),
            timeout_secs,
            ..Default::default()
        });
        let (samples_tx, samples_rx) = mpsc::channel(16);
        let mut handle = transcriber.start_stream(samples_rx).unwrap();
        for _ in 0..chunks {
            if samples_tx.send(vec![0.0_f32; 1600]).await.is_err() {
                break;
            }
        }
        // Give the server time to answer the audio before the stop.
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(samples_tx);

        let mut finals = Vec::new();
        let mut error = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let event = tokio::time::timeout_at(deadline, handle.events.recv())
                .await
                .expect("stream ended in time");
            match event {
                Some(StreamingEvent::Final { text, .. }) => finals.push(text),
                Some(StreamingEvent::Error(e)) => error = Some(e.to_string()),
                Some(StreamingEvent::Ended) | None => break,
                Some(_) => {}
            }
        }
        handle.task.await.unwrap().unwrap();
        (finals, error, server.await.unwrap())
    }

    #[tokio::test]
    async fn stream_ends_on_the_answer_to_finalize() {
        let (finals, error, seen) = dictate(Script::Finalize, 3).await;
        assert_eq!(error, None);
        assert_eq!(finals, ["Hello world.", " How are you?"]);
        assert_eq!(seen.authorization.as_deref(), Some("Token stream-key"));
        assert!(seen.query.contains("encoding=linear16"), "{}", seen.query);
        assert!(seen.query.contains("sample_rate=16000"), "{}", seen.query);
        // The silence primer, then three chunks of 16-bit samples.
        assert_eq!(seen.audio_bytes, 9600 + 3 * 3200);
        assert_eq!(seen.control[..2], [FINALIZE_FRAME, CLOSE_STREAM_FRAME]);
    }

    #[tokio::test]
    async fn stream_ends_on_close_when_finalize_is_not_answered() {
        let (finals, error, _) = dictate(Script::CloseOnly, 2).await;
        assert_eq!(error, None);
        assert_eq!(finals, ["Hello world.", " How are you?"]);
    }

    #[tokio::test]
    async fn close_during_recording_is_an_error() {
        let (finals, error, _) = dictate(Script::CloseEarly, 8).await;
        assert!(finals.is_empty());
        let error = error.expect("an early close is reported");
        // The close is usually read first; an audio write can lose the race.
        assert!(
            error.contains("closed the connection during recording")
                || error.contains("send audio failed"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn refused_handshake_reads_like_a_refused_request() {
        let (finals, error, seen) = dictate(Script::Unauthorized, 1).await;
        assert!(finals.is_empty());
        let error = error.expect("a refused handshake is reported");
        assert!(error.contains("authentication failed"), "{error}");
        assert!(error.contains("401"), "{error}");
        assert_eq!(seen.audio_bytes, 0);
    }

    #[tokio::test]
    async fn a_deepgram_error_message_ends_the_stream_with_it() {
        let (finals, error, seen) = dictate(Script::DeepgramError, 3).await;
        assert!(finals.is_empty());
        let error = error.expect("Deepgram's error is reported");
        assert!(
            error.contains("Deepgram error: NET-0001: no audio"),
            "{error}"
        );
        // The connection is released after the error.
        assert_eq!(seen.control, [CLOSE_STREAM_FRAME]);
    }

    #[tokio::test]
    async fn an_error_close_after_the_stop_is_an_error() {
        let (finals, error, _) = dictate(Script::CloseWithError, 2).await;
        assert_eq!(finals, ["Hello world."]);
        let error = error.expect("the error close is reported");
        assert!(
            error.contains("closed the connection after recording stopped (1011: NET-0000)"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_stop_deepgram_never_answers_times_out() {
        let (finals, error, _) = dictate_within(Script::Silent, 2, 1).await;
        assert_eq!(finals, ["Hello world."]);
        let error = error.expect("the drain timeout is reported");
        assert!(error.contains("did not finish within 1s"), "{error}");
    }

    #[tokio::test]
    async fn cancel_ends_the_stream_without_an_error() {
        let (endpoint, server) = fake_deepgram(Script::Finalize).await;
        let transcriber = new_transcriber(DeepgramConfig {
            endpoint,
            api_key: Some("stream-key".into()),
            ..Default::default()
        });
        let (samples_tx, samples_rx) = mpsc::channel(16);
        let mut handle = transcriber.start_stream(samples_rx).unwrap();
        samples_tx.send(vec![0.0_f32; 1600]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.cancel.send(()).unwrap();

        let mut saw_error = false;
        while let Some(event) = handle.events.recv().await {
            match event {
                StreamingEvent::Error(_) => saw_error = true,
                StreamingEvent::Ended => break,
                _ => {}
            }
        }
        assert!(!saw_error);
        handle.task.await.unwrap().unwrap();
        let seen = server.await.unwrap();
        assert_eq!(seen.control, [CLOSE_STREAM_FRAME]);
    }

    #[test]
    #[ignore = "requires DEEPGRAM_API_KEY and DEEPGRAM_LIVE_TEST_WAV"]
    fn deepgram_live() {
        let path = std::env::var("DEEPGRAM_LIVE_TEST_WAV")
            .expect("set DEEPGRAM_LIVE_TEST_WAV to a short 16 kHz mono PCM WAV");
        let mut reader = hound::WavReader::open(path).expect("open live-test WAV");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1, "live-test WAV must be mono");
        assert_eq!(
            spec.sample_rate, SAMPLE_RATE,
            "live-test WAV must be 16 kHz"
        );
        assert_eq!(spec.bits_per_sample, 16, "live-test WAV must be 16-bit PCM");
        let samples = reader
            .samples::<i16>()
            .map(|sample| sample.expect("decode live-test sample") as f32 / i16::MAX as f32)
            .collect::<Vec<_>>();

        let transcript = DeepgramTranscriber::new(DeepgramConfig::default())
            .expect("configure Deepgram from DEEPGRAM_API_KEY")
            .transcribe(&samples)
            .expect("live Deepgram transcription");
        assert!(!transcript.trim().is_empty());
    }
}
