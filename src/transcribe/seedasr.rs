//! Volcengine Seed-ASR bidirectional streaming WebSocket backend.
//!
//! Seed-ASR uses a compact binary framing protocol over WebSocket. The first
//! client frame contains a gzip-compressed JSON request; subsequent frames
//! contain gzip-compressed PCM16 audio. Server responses contain cumulative,
//! revisable transcript snapshots, which are converted into voxtype's delta
//! oriented [`StreamingEvent`] contract by [`Reconciler`].

use crate::config::SeedAsrConfig;
use crate::error::TranscribeError;
use crate::transcribe::streaming::{SegmentId, StreamHandle, StreamingEvent, StreamingTranscriber};
use crate::transcribe::Transcriber;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const SAMPLE_RATE: u32 = 16_000;
const AUDIO_FRAME_SAMPLES: usize = 3_200; // 200 ms at 16 kHz.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const BATCH_TIMEOUT: Duration = Duration::from_secs(60);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

const PROTOCOL_VERSION: u8 = 1;
const HEADER_SIZE_WORDS: u8 = 1;
const MESSAGE_FULL_CLIENT_REQUEST: u8 = 0x1;
const MESSAGE_AUDIO_ONLY_REQUEST: u8 = 0x2;
const MESSAGE_FULL_SERVER_RESPONSE: u8 = 0x9;
const MESSAGE_SERVER_ERROR: u8 = 0xf;
const FLAG_NONE: u8 = 0x0;
const FLAG_SEQUENCE: u8 = 0x1;
const FLAG_LAST: u8 = 0x2;
const FLAG_LAST_WITH_SEQUENCE: u8 = 0x3;
const SERIALIZATION_NONE: u8 = 0x0;
const SERIALIZATION_JSON: u8 = 0x1;
const COMPRESSION_NONE: u8 = 0x0;
const COMPRESSION_GZIP: u8 = 0x1;

#[derive(Debug, Clone)]
enum Authentication {
    ApiKey(String),
    Legacy {
        app_id: String,
        access_token: String,
    },
}

impl Authentication {
    fn kind(&self) -> &'static str {
        match self {
            Self::ApiKey(_) => "api_key",
            Self::Legacy { .. } => "legacy",
        }
    }
}

#[derive(Debug, Clone)]
struct ConnectionConfig {
    url: String,
    resource_id: String,
    auth: Authentication,
    request_payload: Vec<u8>,
    type_partials: bool,
}

/// Volcengine Seed-ASR transcriber.
#[derive(Debug)]
pub struct SeedAsrTranscriber {
    config: SeedAsrConfig,
    auth: Authentication,
}

impl SeedAsrTranscriber {
    pub fn new(mut config: SeedAsrConfig) -> Result<Self, TranscribeError> {
        fill_credential_from_env(&mut config.api_key, "SEEDASR_API_KEY");
        fill_credential_from_env(&mut config.app_id, "SEEDASR_APP_ID");
        fill_credential_from_env(&mut config.access_token, "SEEDASR_ACCESS_TOKEN");
        if let Ok(value) = std::env::var("SEEDASR_RESOURCE_ID") {
            if !value.trim().is_empty() {
                config.resource_id = value;
            }
        }
        if let Ok(value) = std::env::var("SEEDASR_URL") {
            if !value.trim().is_empty() {
                config.url = value;
            }
        }

        normalize_optional(&mut config.api_key);
        normalize_optional(&mut config.app_id);
        normalize_optional(&mut config.access_token);

        let auth = resolve_authentication(&config)?;
        if config.resource_id.trim().is_empty() {
            return Err(TranscribeError::ConfigError(
                "Seed-ASR resource_id cannot be empty".into(),
            ));
        }
        if config.url.trim().is_empty() {
            return Err(TranscribeError::ConfigError(
                "Seed-ASR url cannot be empty".into(),
            ));
        }
        if !(300..=5_000).contains(&config.end_window_ms) {
            return Err(TranscribeError::ConfigError(
                "Seed-ASR end_window_ms must be between 300 and 5000".into(),
            ));
        }

        tracing::info!(
            "Seed-ASR backend configured: streaming={}, type_partials={}, auth={}, resource_id={}, url={}",
            config.streaming,
            config.type_partials,
            auth.kind(),
            config.resource_id,
            config.url,
        );

        Ok(Self { config, auth })
    }

    fn connection_config(&self) -> Result<ConnectionConfig, TranscribeError> {
        Ok(ConnectionConfig {
            url: self.config.url.clone(),
            resource_id: self.config.resource_id.clone(),
            auth: self.auth.clone(),
            request_payload: encode_full_client_request(&self.request_json())?,
            type_partials: self.config.type_partials,
        })
    }

    fn request_json(&self) -> Value {
        let mut request = json!({
            "model_name": "bigmodel",
            "enable_nonstream": true,
            "enable_itn": self.config.enable_itn,
            "enable_punc": self.config.enable_punc,
            "enable_ddc": self.config.enable_ddc,
            "show_utterances": true,
            "result_type": "full",
            "end_window_size": self.config.end_window_ms,
        });
        if let Some(language) = self
            .config
            .language
            .as_ref()
            .filter(|value| !value.trim().is_empty() && value.as_str() != "auto")
        {
            request["language"] = Value::String(language.clone());
        }

        json!({
            "user": {
                "uid": "voxtype"
            },
            "audio": {
                "format": "pcm",
                "codec": "raw",
                "rate": SAMPLE_RATE,
                "bits": 16,
                "channel": 1
            },
            "request": request
        })
    }

    async fn transcribe_async(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        let connection = self.connection_config()?;
        let (stream, _) = connect(&connection).await?;
        let (mut write, mut read) = stream.split();

        write
            .send(Message::Binary(connection.request_payload.clone()))
            .await
            .map_err(|e| inference_error(format!("send request failed: {e}")))?;

        let pcm = f32_to_s16le_bytes(samples);
        let frame_bytes = AUDIO_FRAME_SAMPLES * 2;
        let mut chunks = pcm.chunks(frame_bytes).peekable();
        while let Some(chunk) = chunks.next() {
            let last = chunks.peek().is_none();
            write
                .send(Message::Binary(encode_audio_request(chunk, last)?))
                .await
                .map_err(|e| inference_error(format!("send audio failed: {e}")))?;
        }

        let deadline = tokio::time::Instant::now() + BATCH_TIMEOUT;
        let mut transcript = String::new();
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(inference_error("batch response timeout"));
            }

            let message = match tokio::time::timeout(remaining, read.next()).await {
                Ok(Some(Ok(message))) => message,
                Ok(Some(Err(error))) => {
                    return Err(inference_error(format!("WebSocket error: {error}")))
                }
                Ok(None) => break,
                Err(_) => return Err(inference_error("batch response timeout")),
            };

            match message {
                Message::Binary(bytes) => {
                    let frame = decode_server_frame(&bytes)?;
                    match frame {
                        ServerFrame::Response { payload, is_last } => {
                            let snapshot = parse_recognition_payload(&payload, is_last)?;
                            if !snapshot.text.is_empty() {
                                transcript = snapshot.text;
                            }
                            if snapshot.is_last {
                                break;
                            }
                        }
                        ServerFrame::Error { code, message } => {
                            return Err(inference_error(format!("server error {code}: {message}")))
                        }
                    }
                }
                Message::Text(text) => {
                    let snapshot = parse_recognition_payload(text.as_bytes(), false)?;
                    if !snapshot.text.is_empty() {
                        transcript = snapshot.text;
                    }
                    if snapshot.is_last {
                        break;
                    }
                }
                Message::Close(_) => break,
                Message::Ping(payload) => {
                    write
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|e| inference_error(format!("send pong failed: {e}")))?;
                }
                _ => {}
            }
        }

        let _ = write.send(Message::Close(None)).await;
        Ok(transcript.trim().to_string())
    }
}

impl Transcriber for SeedAsrTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".into()));
        }

        let future = self.transcribe_async(samples);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
            Err(_) => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| inference_error(format!("create runtime failed: {e}")))?;
                runtime.block_on(future)
            }
        }
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        self.config.streaming.then_some(self as _)
    }
}

impl StreamingTranscriber for SeedAsrTranscriber {
    fn start_stream(
        &self,
        samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        let connection = self.connection_config()?;
        let (events_tx, events_rx) = mpsc::channel(64);
        let (cancel_tx, cancel_rx) = oneshot::channel();

        let task = tokio::spawn(async move {
            if let Err(error) =
                run_streaming_session(connection, samples_rx, &events_tx, cancel_rx).await
            {
                let _ = events_tx.send(StreamingEvent::Error(error)).await;
            }
            let _ = events_tx.send(StreamingEvent::Ended).await;
            Ok(())
        });

        Ok(StreamHandle {
            events: events_rx,
            cancel: cancel_tx,
            task,
        })
    }
}

async fn run_streaming_session(
    connection: ConnectionConfig,
    mut samples_rx: mpsc::Receiver<Vec<f32>>,
    events_tx: &mpsc::Sender<StreamingEvent>,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<(), TranscribeError> {
    let (stream, _) = connect(&connection).await?;
    let (mut write, mut read) = stream.split();
    write
        .send(Message::Binary(connection.request_payload))
        .await
        .map_err(|e| inference_error(format!("send request failed: {e}")))?;

    let mut reconciler = Reconciler::default();
    let mut audio_buffer = Vec::with_capacity(AUDIO_FRAME_SAMPLES * 4);
    let mut input_closed = false;
    let mut last_packet_sent = false;
    let mut drain_deadline = None;

    loop {
        let drain_timer = async {
            match drain_deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            biased;

            _ = &mut cancel_rx => {
                tracing::debug!("Seed-ASR streaming session cancelled");
                break;
            }

            _ = drain_timer, if drain_deadline.is_some() => {
                return Err(inference_error("drain timeout after end of audio"));
            }

            chunk = samples_rx.recv(), if !input_closed => {
                match chunk {
                    Some(samples) => {
                        append_pcm_s16le(&mut audio_buffer, &samples);
                        let frame_bytes = AUDIO_FRAME_SAMPLES * 2;
                        while audio_buffer.len() >= frame_bytes {
                            let remainder = audio_buffer.split_off(frame_bytes);
                            let frame = std::mem::replace(&mut audio_buffer, remainder);
                            write
                                .send(Message::Binary(encode_audio_request(&frame, false)?))
                                .await
                                .map_err(|e| inference_error(format!("send audio failed: {e}")))?;
                        }
                    }
                    None => {
                        input_closed = true;
                        write
                            .send(Message::Binary(encode_audio_request(&audio_buffer, true)?))
                            .await
                            .map_err(|e| inference_error(format!("send final audio failed: {e}")))?;
                        audio_buffer.clear();
                        last_packet_sent = true;
                        drain_deadline = Some(tokio::time::Instant::now() + DRAIN_TIMEOUT);
                    }
                }
            }

            message = read.next() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => return Err(inference_error(format!("WebSocket error: {error}"))),
                    None if last_packet_sent => break,
                    None => return Err(inference_error("WebSocket closed before end of audio")),
                };

                let snapshot = match message {
                    Message::Binary(bytes) => match decode_server_frame(&bytes)? {
                        ServerFrame::Response { payload, is_last } => {
                            parse_recognition_payload(&payload, is_last)?
                        }
                        ServerFrame::Error { code, message } => {
                            return Err(inference_error(format!("server error {code}: {message}")))
                        }
                    },
                    Message::Text(text) => parse_recognition_payload(text.as_bytes(), false)?,
                    Message::Close(_) if last_packet_sent => break,
                    Message::Close(_) => return Err(inference_error("server closed the stream early")),
                    Message::Ping(payload) => {
                        write
                            .send(Message::Pong(payload))
                            .await
                            .map_err(|e| inference_error(format!("send pong failed: {e}")))?;
                        continue;
                    }
                    _ => continue,
                };

                tracing::trace!(
                    target: "voxtype::seedasr::wire",
                    text = %snapshot.text,
                    stable = %snapshot.stable_text,
                    is_last = snapshot.is_last,
                    "Seed-ASR transcript snapshot"
                );
                for event in reconciler.process(snapshot, connection.type_partials)? {
                    if events_tx.send(event).await.is_err() {
                        return Ok(());
                    }
                }
                if reconciler.finished {
                    break;
                }
            }
        }
    }

    let _ = write.send(Message::Close(None)).await;
    Ok(())
}

async fn connect(
    connection: &ConnectionConfig,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
    ),
    TranscribeError,
> {
    let request = websocket_request(connection)?;

    let connected =
        tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request))
            .await
            .map_err(|_| inference_error("connect timeout"))?
            .map_err(|e| inference_error(format!("WebSocket connect failed: {e}")))?;

    if let Some(log_id) = connected.1.headers().get("x-tt-logid") {
        tracing::debug!(
            "Seed-ASR connected: log_id={}",
            log_id.to_str().unwrap_or("<invalid>")
        );
    }
    Ok(connected)
}

fn websocket_request(
    connection: &ConnectionConfig,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, TranscribeError> {
    let mut request = connection
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| TranscribeError::ConfigError(format!("Invalid Seed-ASR url: {e}")))?;

    match &connection.auth {
        Authentication::ApiKey(api_key) => {
            insert_header(request.headers_mut(), "x-api-key", api_key)?;
        }
        Authentication::Legacy {
            app_id,
            access_token,
            ..
        } => {
            insert_header(request.headers_mut(), "x-api-app-key", app_id)?;
            insert_header(request.headers_mut(), "x-api-access-key", access_token)?;
        }
    }
    insert_header(
        request.headers_mut(),
        "x-api-resource-id",
        &connection.resource_id,
    )?;
    insert_header(
        request.headers_mut(),
        "x-api-request-id",
        &Uuid::new_v4().to_string(),
    )?;
    Ok(request)
}

fn insert_header(
    headers: &mut tokio_tungstenite::tungstenite::http::HeaderMap,
    name: &'static str,
    value: &str,
) -> Result<(), TranscribeError> {
    let value = HeaderValue::from_str(value).map_err(|e| {
        TranscribeError::ConfigError(format!("Invalid value for Seed-ASR header {name}: {e}"))
    })?;
    headers.insert(HeaderName::from_static(name), value);
    Ok(())
}

fn fill_credential_from_env(target: &mut Option<String>, variable: &str) {
    if target.as_ref().is_none_or(|value| value.trim().is_empty()) {
        if let Ok(value) = std::env::var(variable) {
            *target = Some(value);
        }
    }
}

fn normalize_optional(value: &mut Option<String>) {
    *value = value.take().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    });
}

fn resolve_authentication(config: &SeedAsrConfig) -> Result<Authentication, TranscribeError> {
    let has_legacy = config.app_id.is_some() || config.access_token.is_some();
    if config.api_key.is_some() && has_legacy {
        return Err(TranscribeError::ConfigError(
            "Seed-ASR credentials are ambiguous: configure api_key or legacy app_id/access_token, not both"
                .into(),
        ));
    }

    if let Some(api_key) = &config.api_key {
        return Ok(Authentication::ApiKey(api_key.clone()));
    }

    match (&config.app_id, &config.access_token) {
        (Some(app_id), Some(access_token)) => Ok(Authentication::Legacy {
            app_id: app_id.clone(),
            access_token: access_token.clone(),
        }),
        (Some(_), None) => Err(TranscribeError::ConfigError(
            "Seed-ASR legacy authentication requires access_token".into(),
        )),
        (None, Some(_)) => Err(TranscribeError::ConfigError(
            "Seed-ASR legacy authentication requires app_id".into(),
        )),
        (None, None) => Err(TranscribeError::ConfigError(
            "Seed-ASR credentials required: set api_key, or app_id and access_token, in [seedasr] or SEEDASR_* environment variables"
                .into(),
        )),
    }
}

fn inference_error(message: impl Into<String>) -> TranscribeError {
    TranscribeError::InferenceFailed(format!("Seed-ASR: {}", message.into()))
}

fn encode_full_client_request(request: &Value) -> Result<Vec<u8>, TranscribeError> {
    let json = serde_json::to_vec(request)
        .map_err(|e| inference_error(format!("serialize request failed: {e}")))?;
    encode_client_frame(
        MESSAGE_FULL_CLIENT_REQUEST,
        FLAG_NONE,
        SERIALIZATION_JSON,
        &json,
    )
}

fn encode_audio_request(pcm: &[u8], last: bool) -> Result<Vec<u8>, TranscribeError> {
    encode_client_frame(
        MESSAGE_AUDIO_ONLY_REQUEST,
        if last { FLAG_LAST } else { FLAG_NONE },
        SERIALIZATION_NONE,
        pcm,
    )
}

fn encode_client_frame(
    message_type: u8,
    flags: u8,
    serialization: u8,
    payload: &[u8],
) -> Result<Vec<u8>, TranscribeError> {
    let compressed = gzip(payload)?;
    let payload_len = u32::try_from(compressed.len())
        .map_err(|_| inference_error("request payload is too large"))?;
    let mut frame = Vec::with_capacity(8 + compressed.len());
    frame.push((PROTOCOL_VERSION << 4) | HEADER_SIZE_WORDS);
    frame.push((message_type << 4) | flags);
    frame.push((serialization << 4) | COMPRESSION_GZIP);
    frame.push(0);
    frame.extend_from_slice(&payload_len.to_be_bytes());
    frame.extend_from_slice(&compressed);
    Ok(frame)
}

fn gzip(payload: &[u8]) -> Result<Vec<u8>, TranscribeError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(payload)
        .map_err(|e| inference_error(format!("gzip encode failed: {e}")))?;
    encoder
        .finish()
        .map_err(|e| inference_error(format!("gzip encode failed: {e}")))
}

fn gunzip(payload: &[u8]) -> Result<Vec<u8>, TranscribeError> {
    let mut decoder = GzDecoder::new(payload);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|e| inference_error(format!("gzip decode failed: {e}")))?;
    Ok(decoded)
}

#[derive(Debug, PartialEq, Eq)]
enum ServerFrame {
    Response { payload: Vec<u8>, is_last: bool },
    Error { code: u32, message: String },
}

fn decode_server_frame(frame: &[u8]) -> Result<ServerFrame, TranscribeError> {
    if frame.len() < 4 {
        return Err(inference_error("server frame is shorter than its header"));
    }
    let version = frame[0] >> 4;
    if version != PROTOCOL_VERSION {
        return Err(inference_error(format!(
            "unsupported protocol version {version}"
        )));
    }
    let header_len = usize::from(frame[0] & 0x0f) * 4;
    if header_len < 4 || frame.len() < header_len {
        return Err(inference_error("invalid server header size"));
    }

    let message_type = frame[1] >> 4;
    let flags = frame[1] & 0x0f;
    let compression = frame[2] & 0x0f;
    let mut cursor = header_len;

    match message_type {
        MESSAGE_FULL_SERVER_RESPONSE => {
            let mut sequence = None;
            if matches!(flags, FLAG_SEQUENCE | FLAG_LAST_WITH_SEQUENCE) {
                sequence = Some(read_i32(frame, &mut cursor)?);
            }
            let payload_len = read_u32(frame, &mut cursor)? as usize;
            let payload = read_payload(frame, cursor, payload_len)?;
            let payload = decode_compression(payload, compression)?;
            let is_last = matches!(flags, FLAG_LAST | FLAG_LAST_WITH_SEQUENCE)
                || sequence.is_some_and(|value| value < 0);
            Ok(ServerFrame::Response { payload, is_last })
        }
        MESSAGE_SERVER_ERROR => {
            let code = read_u32(frame, &mut cursor)?;
            let payload_len = read_u32(frame, &mut cursor)? as usize;
            let payload = read_payload(frame, cursor, payload_len)?;
            let payload = decode_compression(payload, compression)?;
            Ok(ServerFrame::Error {
                code,
                message: String::from_utf8_lossy(&payload).into_owned(),
            })
        }
        other => Err(inference_error(format!(
            "unsupported server message type 0x{other:x}"
        ))),
    }
}

fn read_u32(frame: &[u8], cursor: &mut usize) -> Result<u32, TranscribeError> {
    let bytes = frame
        .get(*cursor..*cursor + 4)
        .ok_or_else(|| inference_error("truncated server frame"))?;
    *cursor += 4;
    Ok(u32::from_be_bytes(
        bytes.try_into().expect("four-byte slice"),
    ))
}

fn read_i32(frame: &[u8], cursor: &mut usize) -> Result<i32, TranscribeError> {
    let bytes = frame
        .get(*cursor..*cursor + 4)
        .ok_or_else(|| inference_error("truncated server frame"))?;
    *cursor += 4;
    Ok(i32::from_be_bytes(
        bytes.try_into().expect("four-byte slice"),
    ))
}

fn read_payload(frame: &[u8], cursor: usize, len: usize) -> Result<&[u8], TranscribeError> {
    let end = cursor
        .checked_add(len)
        .ok_or_else(|| inference_error("invalid server payload length"))?;
    frame
        .get(cursor..end)
        .ok_or_else(|| inference_error("truncated server payload"))
}

fn decode_compression(payload: &[u8], compression: u8) -> Result<Vec<u8>, TranscribeError> {
    match compression {
        COMPRESSION_NONE => Ok(payload.to_vec()),
        COMPRESSION_GZIP => gunzip(payload),
        other => Err(inference_error(format!(
            "unsupported server compression 0x{other:x}"
        ))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecognitionSnapshot {
    text: String,
    stable_text: String,
    is_last: bool,
}

fn parse_recognition_payload(
    payload: &[u8],
    frame_is_last: bool,
) -> Result<RecognitionSnapshot, TranscribeError> {
    let root: Value = serde_json::from_slice(payload)
        .map_err(|e| inference_error(format!("parse response JSON failed: {e}")))?;

    if let Some(code) = root.get("code").and_then(Value::as_i64) {
        if code != 0 {
            let message = root
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(inference_error(format!("server error {code}: {message}")));
        }
    }

    let wrapper_is_last = root
        .get("is_last_package")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let body = match root.get("payload_msg") {
        Some(Value::Object(_)) => root.get("payload_msg").expect("checked above"),
        Some(Value::String(encoded)) => {
            return parse_recognition_payload(encoded.as_bytes(), frame_is_last || wrapper_is_last)
        }
        _ => &root,
    };
    let result = body.get("result").unwrap_or(body);
    let text = result
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let is_last = frame_is_last || wrapper_is_last;

    let stable_text = if is_last {
        text.clone()
    } else {
        stable_prefix(result, &text)
    };

    Ok(RecognitionSnapshot {
        text,
        stable_text,
        is_last,
    })
}

fn stable_prefix(result: &Value, text: &str) -> String {
    let Some(utterances) = result.get("utterances").and_then(Value::as_array) else {
        return String::new();
    };

    if let Some(definite) = align_definite_prefix(utterances, text) {
        return definite.to_string();
    }

    utterances
        .iter()
        .filter_map(|utterance| {
            utterance
                .get("additions")
                .and_then(|value| value.get("fixed_prefix_result"))
                .and_then(Value::as_str)
        })
        .filter(|prefix| !prefix.is_empty() && text.starts_with(prefix))
        .max_by_key(|prefix| prefix.len())
        .unwrap_or_default()
        .to_string()
}

/// Align the leading definite utterances with the cumulative transcript and
/// return the exact prefix from `text` that they cover.
///
/// Seed-ASR can insert whitespace between utterances in `result.text` even
/// though that whitespace is absent from the individual `utterance.text`
/// values. Concatenating utterance strings directly therefore turns
/// `"hello"` + `"world"` into `"helloworld"`, which does not match the
/// authoritative cumulative transcript `"hello world"`. Match each utterance
/// in sequence instead, allowing only whitespace between them, and retain the
/// exact separator chosen by the service in the returned prefix.
fn align_definite_prefix<'a>(utterances: &[Value], text: &'a str) -> Option<&'a str> {
    let mut offset = 0;
    let mut matched_any = false;

    for utterance in utterances.iter().take_while(|utterance| {
        utterance
            .get("definite")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }) {
        let utterance_text = utterance.get("text").and_then(Value::as_str)?;
        if utterance_text.is_empty() {
            continue;
        }

        let remaining = &text[offset..];
        if remaining.starts_with(utterance_text) {
            // The utterance text already contains exactly the separator used
            // by the cumulative transcript, so consume it verbatim.
            offset += utterance_text.len();
        } else if matched_any {
            // Some responses keep inter-utterance whitespace only in the
            // cumulative transcript. Skip that separator and try again.
            let candidate = remaining.trim_start_matches(char::is_whitespace);
            if !candidate.starts_with(utterance_text) {
                // Stability metadata for later utterances can be temporarily
                // inconsistent. Keep the prefix aligned so far instead of
                // regressing the entire stable prefix to empty.
                break;
            }
            let separator_len = remaining.len() - candidate.len();
            offset += separator_len + utterance_text.len();
        } else {
            return None;
        }
        matched_any = true;
    }

    matched_any.then(|| &text[..offset])
}

#[derive(Debug, Default)]
struct Reconciler {
    committed: String,
    typed_partial: String,
    next_segment_id: SegmentId,
    finished: bool,
}

impl Reconciler {
    fn process(
        &mut self,
        snapshot: RecognitionSnapshot,
        type_partials: bool,
    ) -> Result<Vec<StreamingEvent>, TranscribeError> {
        let mut events = Vec::new();

        if !snapshot.text.starts_with(&self.committed) {
            return Err(inference_error(
                "server revised text that was already finalized",
            ));
        }

        // `utterances` stability metadata is not monotonic: intermediate
        // responses can temporarily omit it or report a shorter prefix. The
        // cumulative transcript is authoritative for protecting text already
        // committed to the output, so retain that prefix while the transcript
        // itself still extends it.
        let stable_text = if snapshot.stable_text.starts_with(&self.committed) {
            snapshot.stable_text
        } else if self.committed.starts_with(&snapshot.stable_text) {
            tracing::debug!(
                reported = %snapshot.stable_text,
                committed = %self.committed,
                "Seed-ASR stable prefix regressed; retaining committed text"
            );
            self.committed.clone()
        } else {
            return Err(inference_error(
                "server stable prefix does not extend finalized text",
            ));
        };

        if stable_text.len() > self.committed.len() {
            let stable_tail = &stable_text[self.committed.len()..];
            let segment_id = self.next_segment_id;
            self.next_segment_id += 1;

            if stable_tail.starts_with(&self.typed_partial) {
                events.push(StreamingEvent::Final {
                    text: stable_tail[self.typed_partial.len()..].to_string(),
                    segment_id,
                });
            } else {
                let common_chars = common_prefix_char_count(&self.typed_partial, stable_tail);
                let replacement: String = stable_tail.chars().skip(common_chars).collect();
                events.push(StreamingEvent::Replace {
                    backspace: self.typed_partial.chars().count() - common_chars,
                    text: replacement,
                    segment_id,
                });
            }
            self.committed = stable_text;
            self.typed_partial.clear();
        }

        let candidate_tail = &snapshot.text[self.committed.len()..];
        if type_partials && candidate_tail.starts_with(&self.typed_partial) {
            let delta = &candidate_tail[self.typed_partial.len()..];
            if !delta.is_empty() {
                events.push(StreamingEvent::Partial {
                    text: delta.to_string(),
                    segment_id: self.next_segment_id,
                });
                self.typed_partial = candidate_tail.to_string();
            }
        }

        self.finished = snapshot.is_last;
        Ok(events)
    }
}

fn common_prefix_char_count(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

fn append_pcm_s16le(output: &mut Vec<u8>, samples: &[f32]) {
    output.reserve(samples.len() * 2);
    for sample in samples {
        let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        output.extend_from_slice(&pcm.to_le_bytes());
    }
}

fn f32_to_s16le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut output = Vec::with_capacity(samples.len() * 2);
    append_pcm_s16le(&mut output, samples);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::handshake::server::{
        Callback, ErrorResponse, Request, Response,
    };

    struct AssertAuthenticationHeaders;

    impl Callback for AssertAuthenticationHeaders {
        fn on_request(
            self,
            request: &Request,
            response: Response,
        ) -> Result<Response, ErrorResponse> {
            assert_eq!(request.headers()["x-api-key"], "test-api-key");
            assert_eq!(
                request.headers()["x-api-resource-id"],
                "volc.seedasr.sauc.duration"
            );
            Ok(response)
        }
    }

    fn config() -> SeedAsrConfig {
        SeedAsrConfig {
            api_key: Some("test-api-key".into()),
            ..SeedAsrConfig::default()
        }
    }

    #[test]
    fn accepts_new_console_api_key() {
        let transcriber = SeedAsrTranscriber::new(config()).unwrap();
        assert!(matches!(transcriber.auth, Authentication::ApiKey(_)));
    }

    #[test]
    fn accepts_legacy_credentials() {
        let cfg = SeedAsrConfig {
            api_key: None,
            app_id: Some("app".into()),
            access_token: Some("token".into()),
            ..SeedAsrConfig::default()
        };
        let transcriber = SeedAsrTranscriber::new(cfg).unwrap();
        assert!(matches!(transcriber.auth, Authentication::Legacy { .. }));
    }

    #[test]
    fn rejects_mixed_authentication_modes() {
        let mut cfg = config();
        cfg.app_id = Some("app".into());
        cfg.access_token = Some("token".into());
        let error = SeedAsrTranscriber::new(cfg).unwrap_err().to_string();
        assert!(error.contains("ambiguous"));
    }

    #[test]
    fn builds_new_console_authentication_headers() {
        let transcriber = SeedAsrTranscriber::new(config()).unwrap();
        let connection = transcriber.connection_config().unwrap();
        let request = websocket_request(&connection).unwrap();
        assert_eq!(request.headers()["x-api-key"], "test-api-key");
        assert_eq!(
            request.headers()["x-api-resource-id"],
            "volc.seedasr.sauc.duration"
        );
        assert!(request.headers().get("x-api-app-key").is_none());
        assert!(Uuid::parse_str(request.headers()["x-api-request-id"].to_str().unwrap()).is_ok());
    }

    #[test]
    fn builds_legacy_authentication_headers() {
        let cfg = SeedAsrConfig {
            api_key: None,
            app_id: Some("legacy-app".into()),
            access_token: Some("legacy-token".into()),
            ..SeedAsrConfig::default()
        };
        let transcriber = SeedAsrTranscriber::new(cfg).unwrap();
        let connection = transcriber.connection_config().unwrap();
        let request = websocket_request(&connection).unwrap();
        assert_eq!(request.headers()["x-api-app-key"], "legacy-app");
        assert_eq!(request.headers()["x-api-access-key"], "legacy-token");
        assert!(request.headers().get("x-api-key").is_none());
    }

    #[test]
    fn encodes_expected_client_headers() {
        let full = encode_full_client_request(&json!({"request": {}})).unwrap();
        assert_eq!(&full[..4], &[0x11, 0x10, 0x11, 0x00]);

        let audio = encode_audio_request(&[1, 2], false).unwrap();
        assert_eq!(&audio[..4], &[0x11, 0x20, 0x01, 0x00]);

        let last = encode_audio_request(&[], true).unwrap();
        assert_eq!(&last[..4], &[0x11, 0x22, 0x01, 0x00]);
    }

    #[test]
    fn decodes_gzip_server_response_with_negative_sequence() {
        let payload = br#"{"result":{"text":"hello"}}"#;
        let compressed = gzip(payload).unwrap();
        let mut frame = vec![0x11, 0x93, 0x11, 0x00];
        frame.extend_from_slice(&(-7_i32).to_be_bytes());
        frame.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        frame.extend_from_slice(&compressed);

        assert_eq!(
            decode_server_frame(&frame).unwrap(),
            ServerFrame::Response {
                payload: payload.to_vec(),
                is_last: true,
            }
        );
    }

    #[test]
    fn parses_direct_and_wrapped_response_shapes() {
        let direct = parse_recognition_payload(
            br#"{"result":{"text":"hello","utterances":[{"text":"hello","definite":true}]}}"#,
            false,
        )
        .unwrap();
        assert_eq!(direct.stable_text, "hello");

        let wrapped = parse_recognition_payload(
            br#"{"code":0,"is_last_package":true,"payload_msg":{"result":{"text":"hello"}}}"#,
            false,
        )
        .unwrap();
        assert!(wrapped.is_last);
        assert_eq!(wrapped.stable_text, "hello");
    }

    #[test]
    fn stable_prefix_preserves_whitespace_between_definite_utterances() {
        let snapshot = parse_recognition_payload(
            br#"{"result":{"text":"hello world","utterances":[{"text":"hello","definite":true},{"text":"world","definite":true}]}}"#,
            false,
        )
        .unwrap();

        assert_eq!(snapshot.stable_text, "hello world");
    }

    #[test]
    fn stable_prefix_accepts_whitespace_already_present_in_utterance_text() {
        let snapshot = parse_recognition_payload(
            br#"{"result":{"text":"hello world","utterances":[{"text":"hello","definite":true},{"text":" world","definite":true}]}}"#,
            false,
        )
        .unwrap();

        assert_eq!(snapshot.stable_text, "hello world");
    }

    #[test]
    fn stable_prefix_keeps_the_last_aligned_utterance_on_a_later_mismatch() {
        let snapshot = parse_recognition_payload(
            br#"{"result":{"text":"hello world","utterances":[{"text":"hello","definite":true},{"text":"there","definite":true}]}}"#,
            false,
        )
        .unwrap();

        assert_eq!(snapshot.stable_text, "hello");
    }

    #[test]
    fn stable_prefix_stops_before_a_provisional_utterance_and_its_separator() {
        let snapshot = parse_recognition_payload(
            br#"{"result":{"text":"hello world","utterances":[{"text":"hello","definite":true},{"text":"world","definite":false}]}}"#,
            false,
        )
        .unwrap();

        assert_eq!(snapshot.stable_text, "hello");
    }

    #[test]
    fn reconciler_commits_a_whitespace_separated_definite_utterance() {
        let mut reconciler = Reconciler::default();
        let first = reconciler
            .process(
                RecognitionSnapshot {
                    text: "hello".into(),
                    stable_text: "hello".into(),
                    is_last: false,
                },
                false,
            )
            .unwrap();
        assert!(matches!(
            &first[0],
            StreamingEvent::Final { text, .. } if text == "hello"
        ));

        let snapshot = parse_recognition_payload(
            br#"{"result":{"text":"hello world","utterances":[{"text":"hello","definite":true},{"text":"world","definite":true}]}}"#,
            false,
        )
        .unwrap();
        let second = reconciler.process(snapshot, false).unwrap();

        assert!(matches!(
            &second[0],
            StreamingEvent::Final { text, .. } if text == " world"
        ));
    }

    #[test]
    fn reconciler_emits_deltas_and_final_promotion() {
        let mut reconciler = Reconciler::default();
        let partial = reconciler
            .process(
                RecognitionSnapshot {
                    text: "hello".into(),
                    stable_text: String::new(),
                    is_last: false,
                },
                true,
            )
            .unwrap();
        assert!(matches!(
            &partial[0],
            StreamingEvent::Partial { text, .. } if text == "hello"
        ));

        let final_events = reconciler
            .process(
                RecognitionSnapshot {
                    text: "hello world".into(),
                    stable_text: "hello world".into(),
                    is_last: true,
                },
                true,
            )
            .unwrap();
        assert!(matches!(
            &final_events[0],
            StreamingEvent::Final { text, .. } if text == " world"
        ));
        assert!(reconciler.finished);
    }

    #[test]
    fn reconciler_replaces_revised_unicode_partial() {
        let mut reconciler = Reconciler {
            typed_partial: "你好呀".into(),
            ..Reconciler::default()
        };
        let events = reconciler
            .process(
                RecognitionSnapshot {
                    text: "你好。".into(),
                    stable_text: "你好。".into(),
                    is_last: true,
                },
                true,
            )
            .unwrap();

        assert!(matches!(
            &events[0],
            StreamingEvent::Replace {
                backspace: 1,
                text,
                ..
            } if text == "。"
        ));
    }

    #[test]
    fn reconciler_tolerates_regressed_stability_metadata() {
        let mut reconciler = Reconciler::default();
        reconciler
            .process(
                RecognitionSnapshot {
                    text: "hello".into(),
                    stable_text: "hello".into(),
                    is_last: false,
                },
                false,
            )
            .unwrap();

        let regressed = reconciler
            .process(
                RecognitionSnapshot {
                    text: "hello world".into(),
                    stable_text: String::new(),
                    is_last: false,
                },
                false,
            )
            .unwrap();
        assert!(regressed.is_empty());
        assert_eq!(reconciler.committed, "hello");

        let recovered = reconciler
            .process(
                RecognitionSnapshot {
                    text: "hello world".into(),
                    stable_text: "hello world".into(),
                    is_last: true,
                },
                false,
            )
            .unwrap();
        assert!(matches!(
            &recovered[0],
            StreamingEvent::Final { text, .. } if text == " world"
        ));
        assert!(reconciler.finished);
    }

    #[test]
    fn reconciler_still_rejects_revision_of_committed_text() {
        let mut reconciler = Reconciler {
            committed: "hello".into(),
            ..Reconciler::default()
        };
        let error = reconciler
            .process(
                RecognitionSnapshot {
                    text: "hullo".into(),
                    stable_text: "hullo".into(),
                    is_last: false,
                },
                false,
            )
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("server revised text that was already finalized"));
    }

    #[test]
    fn pcm_conversion_clamps_samples() {
        let bytes = f32_to_s16le_bytes(&[-2.0, 0.0, 2.0]);
        assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), -32767);
        assert_eq!(i16::from_le_bytes([bytes[2], bytes[3]]), 0);
        assert_eq!(i16::from_le_bytes([bytes[4], bytes[5]]), 32767);
    }

    #[tokio::test]
    async fn streaming_round_trip_with_local_websocket_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut stream =
                tokio_tungstenite::accept_hdr_async(socket, AssertAuthenticationHeaders)
                    .await
                    .unwrap();

            let request = stream.next().await.unwrap().unwrap();
            let Message::Binary(request) = request else {
                panic!("expected full client request");
            };
            assert_eq!(&request[..4], &[0x11, 0x10, 0x11, 0x00]);

            loop {
                let message = stream.next().await.unwrap().unwrap();
                let Message::Binary(audio) = message else {
                    continue;
                };
                if audio[1] & 0x0f == FLAG_LAST {
                    break;
                }
            }

            let payload =
                br#"{"result":{"text":"hello","utterances":[{"text":"hello","definite":true}]}}"#;
            let compressed = gzip(payload).unwrap();
            let mut response = vec![0x11, 0x93, 0x11, 0x00];
            response.extend_from_slice(&(-1_i32).to_be_bytes());
            response.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
            response.extend_from_slice(&compressed);
            stream.send(Message::Binary(response)).await.unwrap();
        });

        let transcriber = SeedAsrTranscriber::new(SeedAsrConfig {
            url: format!("ws://{address}"),
            type_partials: false,
            ..config()
        })
        .unwrap();
        let (samples_tx, samples_rx) = mpsc::channel(2);
        let mut handle = transcriber.start_stream(samples_rx).unwrap();
        samples_tx.send(vec![0.1; 400]).await.unwrap();
        drop(samples_tx);

        let event = tokio::time::timeout(Duration::from_secs(2), handle.events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            StreamingEvent::Final { text, .. } if text == "hello"
        ));
        let ended = tokio::time::timeout(Duration::from_secs(2), handle.events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(ended, StreamingEvent::Ended));
        handle.task.await.unwrap().unwrap();
        server.await.unwrap();
    }
}
