//! ElevenLabs Scribe realtime transcription over a WebSocket.
//!
//! Audio framing, the wire format, transcript reconciliation and the session
//! lifecycle live here; the transport loop drives them.

use std::{
    collections::VecDeque,
    future::{pending, Future},
    time::Duration,
};

use base64::Engine as _;
use futures_util::{FutureExt, Sink, SinkExt, StreamExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
};
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, http::Request, Error as WebSocketError, Message},
    MaybeTlsStream, WebSocketStream,
};

use super::{
    api_origin, f32_to_pcm16le, redact_and_bound, sensitive_api_key_header, ElevenLabsTranscriber,
    API_KEY_HEADER, SAMPLE_RATE,
};
use crate::{
    config::{ElevenLabsCommitStrategy, ElevenLabsConfig},
    error::TranscribeError,
    transcribe::{SegmentId, StreamHandle, StreamingEvent, StreamingTranscriber},
};

const REALTIME_AUDIO_FORMAT: &str = "pcm_16000";
const VAD_THRESHOLD: &str = "0.4";
const MIN_SPEECH_DURATION_MS: &str = "100";
const MIN_SILENCE_DURATION_MS: &str = "100";
const FRAME_DURATION_MS: usize = 100;
const SAMPLES_PER_FRAME: usize = SAMPLE_RATE as usize * FRAME_DURATION_MS / 1_000;
const BYTES_PER_FRAME: usize = SAMPLES_PER_FRAME * size_of::<i16>();
const SESSION_START_TIMEOUT: Duration = Duration::from_secs(10);
const PRE_SESSION_AUDIO_LIMIT_SAMPLES: usize = SAMPLE_RATE as usize * 10;
const FINAL_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
const POST_FINAL_QUIET_PERIOD: Duration = Duration::from_secs(1);
/// How long the drain waits after the stop's first committed transcript when
/// that transcript may have answered a VAD commit instead (see
/// `SessionPhase::DrainingAwaitingFinal`).
const AMBIGUOUS_FINAL_QUIET_PERIOD: Duration = Duration::from_secs(3);
const STREAM_EVENT_CHANNEL_CAPACITY: usize = 64;
const WEBSOCKET_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
// Keep finalization responsive even if the provider has a large ready backlog.
const MAX_PRE_COMMIT_DRAIN_MESSAGES: usize = 64;

fn realtime_url(config: &ElevenLabsConfig) -> Result<reqwest::Url, TranscribeError> {
    let endpoint = format!(
        "wss://{}/v1/speech-to-text/realtime",
        api_origin(config.region)
    );
    let mut url = reqwest::Url::parse(&endpoint).map_err(|error| {
        TranscribeError::ConfigError(format!("Invalid ElevenLabs realtime endpoint: {error}"))
    })?;

    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("model_id", &config.realtime_model)
            .append_pair("audio_format", REALTIME_AUDIO_FORMAT)
            .append_pair("commit_strategy", &config.commit_strategy.to_string())
            .append_pair("no_verbatim", &config.no_verbatim.to_string());
        if config.commit_strategy == ElevenLabsCommitStrategy::Vad {
            query
                .append_pair(
                    "vad_silence_threshold_secs",
                    &config.vad_silence_threshold_secs.to_string(),
                )
                .append_pair("vad_threshold", VAD_THRESHOLD)
                .append_pair("min_speech_duration_ms", MIN_SPEECH_DURATION_MS)
                .append_pair("min_silence_duration_ms", MIN_SILENCE_DURATION_MS);
        }
        if let Some(language_code) = config.language_code.as_deref() {
            query.append_pair("language_code", language_code);
        }
    }

    Ok(url)
}

fn realtime_request(
    config: &ElevenLabsConfig,
    api_key: &str,
) -> Result<Request<()>, TranscribeError> {
    let url = realtime_url(config)?;
    let mut request = url.as_str().into_client_request().map_err(|error| {
        TranscribeError::ConfigError(format!(
            "Could not construct ElevenLabs realtime request: {error}"
        ))
    })?;
    request
        .headers_mut()
        .insert(API_KEY_HEADER, sensitive_api_key_header(api_key)?);
    Ok(request)
}

impl StreamingTranscriber for ElevenLabsTranscriber {
    fn start_stream(
        &self,
        samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        let request = realtime_request(&self.config, &self.api_key)?;
        let api_key = self.api_key.clone();
        let type_partials = self.config.types_partial_transcripts();
        let vad_commits = self.config.commit_strategy == ElevenLabsCommitStrategy::Vad;
        let (events_tx, events_rx) = mpsc::channel(STREAM_EVENT_CHANNEL_CAPACITY);
        let (cancel_tx, cancel_rx) = oneshot::channel();

        let task = tokio::spawn(async move {
            // Reserve both fatal-epilogue slots before normal traffic. The
            // daemon awaits cancellation without polling events, so neither
            // Error nor Ended may depend on free capacity at shutdown.
            let Some(epilogue) = StreamEpilogue::reserve(&events_tx) else {
                return Ok(());
            };
            let exit = run_streaming_session(
                request,
                &api_key,
                type_partials,
                vad_commits,
                samples_rx,
                &events_tx,
                cancel_rx,
            )
            .await;

            epilogue.finish(exit);
            Ok(())
        });

        Ok(StreamHandle {
            events: events_rx,
            cancel: cancel_tx,
            task,
        })
    }
}

#[derive(Debug, Default)]
struct PcmFrameAccumulator {
    pending: Vec<u8>,
}

impl PcmFrameAccumulator {
    fn push(&mut self, samples: &[f32]) -> Vec<Vec<u8>> {
        self.pending.extend(f32_to_pcm16le(samples));

        let complete_bytes = self.pending.len() / BYTES_PER_FRAME * BYTES_PER_FRAME;
        let frames = self.pending[..complete_bytes]
            .chunks_exact(BYTES_PER_FRAME)
            .map(<[u8]>::to_vec)
            .collect();
        self.pending.drain(..complete_bytes);
        frames
    }

    fn flush(&mut self) -> Option<Vec<u8>> {
        (!self.pending.is_empty()).then(|| std::mem::take(&mut self.pending))
    }
}

#[derive(Debug, Default)]
struct RealtimeAudioEncoder {
    frames: PcmFrameAccumulator,
    final_commit_sent: bool,
}

impl RealtimeAudioEncoder {
    fn push(&mut self, samples: &[f32]) -> Result<Vec<String>, TranscribeError> {
        if self.final_commit_sent {
            return Err(TranscribeError::InferenceFailed(
                "Cannot send ElevenLabs audio after the final commit".to_string(),
            ));
        }

        self.frames
            .push(samples)
            .into_iter()
            .map(|frame| serialize_audio_chunk(&frame))
            .collect()
    }

    fn finish(&mut self) -> Result<Vec<String>, TranscribeError> {
        if self.final_commit_sent {
            return Ok(Vec::new());
        }

        let mut messages = Vec::with_capacity(2);
        if let Some(tail) = self.frames.flush() {
            messages.push(serialize_audio_chunk(&tail)?);
        }
        messages.push(serialize_final_commit()?);
        self.final_commit_sent = true;
        Ok(messages)
    }
}

#[derive(Debug, Default)]
struct PreSessionAudioBuffer {
    chunks: VecDeque<Vec<f32>>,
    sample_count: usize,
}

impl PreSessionAudioBuffer {
    fn push(&mut self, chunk: Vec<f32>) -> Result<(), TranscribeError> {
        if chunk.is_empty() {
            return Ok(());
        }

        let sample_count = self
            .sample_count
            .checked_add(chunk.len())
            .ok_or_else(pre_session_overflow_error)?;
        if sample_count > PRE_SESSION_AUDIO_LIMIT_SAMPLES {
            return Err(pre_session_overflow_error());
        }

        self.sample_count = sample_count;
        self.chunks.push_back(chunk);
        Ok(())
    }

    fn into_chunks(self) -> VecDeque<Vec<f32>> {
        self.chunks
    }
}

fn pre_session_overflow_error() -> TranscribeError {
    TranscribeError::InferenceFailed(
        "ElevenLabs realtime startup exceeded the 10-second pre-session audio buffer".to_string(),
    )
}

#[derive(Debug, Serialize)]
struct InputAudioChunk {
    message_type: &'static str,
    audio_base_64: String,
    commit: bool,
    sample_rate: u32,
}

fn serialize_audio_chunk(pcm16le: &[u8]) -> Result<String, TranscribeError> {
    serialize_input_audio_chunk(pcm16le, false)
}

fn serialize_final_commit() -> Result<String, TranscribeError> {
    serialize_input_audio_chunk(&[], true)
}

fn serialize_input_audio_chunk(pcm16le: &[u8], commit: bool) -> Result<String, TranscribeError> {
    let message = InputAudioChunk {
        message_type: "input_audio_chunk",
        audio_base_64: base64::engine::general_purpose::STANDARD.encode(pcm16le),
        commit,
        sample_rate: SAMPLE_RATE,
    };
    serde_json::to_string(&message).map_err(|error| {
        TranscribeError::InferenceFailed(format!(
            "Could not serialize ElevenLabs audio message: {error}"
        ))
    })
}

#[derive(Debug, PartialEq, Eq)]
enum WireMessage {
    SessionStarted { session_id: String },
    Partial { text: String },
    Committed { text: String },
    CommittedWithTimestamps { text: String },
    ProviderError { kind: String, message: String },
    Unknown,
}

#[derive(Debug, Deserialize)]
struct MessageEnvelope {
    message_type: String,
}

#[derive(Debug, Deserialize)]
struct SessionStartedPayload {
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct TranscriptPayload {
    text: String,
}

#[derive(Debug, Deserialize)]
struct ProviderErrorPayload {
    error: String,
}

const PROVIDER_ERROR_EVENTS: &[&str] = &[
    "error",
    "auth_error",
    "quota_exceeded",
    "commit_throttled",
    "unaccepted_terms",
    "rate_limited",
    "queue_overflow",
    "resource_exhausted",
    "session_time_limit_exceeded",
    "input_error",
    "chunk_size_exceeded",
    "insufficient_audio_activity",
    "transcriber_error",
];

/// serde quotes the response text it cannot use, which may hold the key.
fn malformed_message(
    message_type: &str,
    error: &serde_json::Error,
    api_key: &str,
) -> TranscribeError {
    TranscribeError::InferenceFailed(format!(
        "Malformed ElevenLabs {message_type} message: {}",
        redact_and_bound(&error.to_string(), api_key)
    ))
}

fn parse_payload<T: DeserializeOwned>(
    value: &serde_json::Value,
    message_type: &str,
    api_key: &str,
) -> Result<T, TranscribeError> {
    serde_json::from_value(value.clone())
        .map_err(|error| malformed_message(message_type, &error, api_key))
}

fn parse_wire_message(payload: &str, api_key: &str) -> Result<WireMessage, TranscribeError> {
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|error| malformed_message("realtime", &error, api_key))?;
    let envelope: MessageEnvelope = parse_payload(&value, "realtime", api_key)?;

    match envelope.message_type.as_str() {
        "session_started" => {
            let parsed: SessionStartedPayload = parse_payload(&value, "session_started", api_key)?;
            Ok(WireMessage::SessionStarted {
                session_id: parsed.session_id,
            })
        }
        "partial_transcript" => {
            let parsed: TranscriptPayload = parse_payload(&value, "partial_transcript", api_key)?;
            Ok(WireMessage::Partial { text: parsed.text })
        }
        "committed_transcript" => {
            let parsed: TranscriptPayload = parse_payload(&value, "committed_transcript", api_key)?;
            Ok(WireMessage::Committed { text: parsed.text })
        }
        "committed_transcript_with_timestamps" => {
            let parsed: TranscriptPayload =
                parse_payload(&value, "committed_transcript_with_timestamps", api_key)?;
            Ok(WireMessage::CommittedWithTimestamps { text: parsed.text })
        }
        message_type if PROVIDER_ERROR_EVENTS.contains(&message_type) => {
            let parsed: ProviderErrorPayload = parse_payload(&value, message_type, api_key)?;
            Ok(WireMessage::ProviderError {
                kind: message_type.to_string(),
                message: redact_and_bound(&parsed.error, api_key),
            })
        }
        _ => {
            // Do not log or retain the provider-controlled event name because a
            // malformed response could echo credentials or audio data in it.
            tracing::debug!("Ignoring unknown ElevenLabs realtime event");
            Ok(WireMessage::Unknown)
        }
    }
}

#[derive(Debug)]
struct TranscriptReconciler {
    type_partials: bool,
    /// The current segment as it stands at the cursor, separator included.
    typed_partial: String,
    segment_id: SegmentId,
    /// The end of the text committed so far, which decides whether the next
    /// segment needs a space in front of it (see `spacing_end`).
    committed_end: Option<char>,
    /// Whether the provider reported speech it has not committed yet.
    segment_open: bool,
    /// The provider's text for the last committed segment, as it sent it.
    last_committed: String,
    /// The newest partial of the open segment that the cursor never got,
    /// because partials are not typed or the drain held it back.
    unsent_partial: String,
}

impl TranscriptReconciler {
    fn new(type_partials: bool) -> Self {
        Self {
            type_partials,
            typed_partial: String::new(),
            segment_id: 0,
            committed_end: None,
            segment_open: false,
            last_committed: String::new(),
            unsent_partial: String::new(),
        }
    }

    /// A segment's text as it goes at the cursor. Each committed segment
    /// arrives without a leading space, so one goes in front of it where
    /// the text on both sides is from a script that separates words with
    /// spaces (see `segment_separator`). The choice depends only on the
    /// committed text and the snapshot, so a partial and its commit agree.
    fn at_cursor(&self, text: String) -> String {
        match segment_separator(self.committed_end, &text) {
            "" => text,
            separator => format!("{separator}{text}"),
        }
    }

    fn process(&mut self, message: WireMessage) -> Vec<StreamingEvent> {
        match &message {
            WireMessage::Partial { text } if !text.is_empty() => {
                self.segment_open = true;
                if !self.type_partials {
                    self.unsent_partial.clone_from(text);
                }
            }
            WireMessage::Committed { text } => {
                self.segment_open = false;
                self.last_committed.clone_from(text);
                self.unsent_partial.clear();
            }
            _ => {}
        }
        match message {
            WireMessage::Partial { text } => self.process_partial(text),
            WireMessage::Committed { text } => vec![self.process_commit(text)],
            // The service sends this after `committed_transcript` when timestamps
            // are requested. The preceding event already finalized the segment.
            WireMessage::CommittedWithTimestamps { .. }
            | WireMessage::SessionStarted { .. }
            | WireMessage::ProviderError { .. }
            | WireMessage::Unknown => Vec::new(),
        }
    }

    fn process_partial(&mut self, snapshot: String) -> Vec<StreamingEvent> {
        if !self.type_partials {
            return Vec::new();
        }
        let snapshot = self.at_cursor(snapshot);
        if snapshot == self.typed_partial {
            return Vec::new();
        }

        let event = if snapshot.starts_with(&self.typed_partial) {
            StreamingEvent::Partial {
                text: snapshot[self.typed_partial.len()..].to_string(),
                segment_id: self.segment_id,
            }
        } else {
            let common_chars = common_prefix_char_count(&self.typed_partial, &snapshot);
            StreamingEvent::RevisePartial {
                backspace: self.typed_partial.chars().count() - common_chars,
                text: snapshot.chars().skip(common_chars).collect(),
                segment_id: self.segment_id,
            }
        };

        self.typed_partial = snapshot;
        vec![event]
    }

    /// The unsent partial as an append, once text was committed before it.
    /// A failed drain types it so the words are not lost: the daemon cannot
    /// recover them from the audio without repeating the committed text.
    /// With nothing committed, it recovers the whole recording instead.
    fn take_unconfirmed_tail(&mut self) -> Option<StreamingEvent> {
        if self.committed_end.is_none()
            || !is_new_speech(&self.unsent_partial, &self.last_committed)
        {
            return None;
        }
        let text = std::mem::take(&mut self.unsent_partial);
        Some(StreamingEvent::Partial {
            text: self.at_cursor(text),
            segment_id: self.segment_id,
        })
    }

    fn process_commit(&mut self, committed: String) -> StreamingEvent {
        let committed = self.at_cursor(committed);
        if let Some(end) = spacing_end(&committed) {
            self.committed_end = Some(end);
        }
        let segment_id = self.segment_id;
        let event = if committed.starts_with(&self.typed_partial) {
            StreamingEvent::Final {
                text: committed[self.typed_partial.len()..].to_string(),
                segment_id,
            }
        } else {
            let common_chars = common_prefix_char_count(&self.typed_partial, &committed);
            StreamingEvent::Replace {
                backspace: self.typed_partial.chars().count() - common_chars,
                text: committed.chars().skip(common_chars).collect(),
                segment_id,
            }
        };

        self.typed_partial.clear();
        self.segment_id += 1;
        event
    }
}

/// The separator between committed text ending in `before` and a segment
/// starting with `text`: a space where a script that separates words with
/// spaces continues with a word, nothing otherwise. So `Hello.` and `World`
/// join as `Hello. World`, but `今日は` and `雨です。` join as `今日は雨です。`,
/// and a segment that brings its own space or starts with `,` gets none.
fn segment_separator(before: Option<char>, text: &str) -> &'static str {
    let (Some(before), Some(first)) = (before, text.chars().next()) else {
        return "";
    };
    let starts_word = first.is_alphanumeric() || matches!(first, '(' | '"' | '\'' | '“' | '‘');
    if !before.is_whitespace() && starts_word && uses_spaces(before) && uses_spaces(first) {
        " "
    } else {
        ""
    }
}

/// The character that ends `text` for spacing: trailing whitespace itself,
/// otherwise the last letter or digit, so punctuation after it does not hide
/// its script. `ไปไหน?` ends in Thai, `Done?` in Latin.
fn spacing_end(text: &str) -> Option<char> {
    let last = text.chars().next_back()?;
    if last.is_whitespace() {
        return Some(last);
    }
    Some(text.chars().rfind(|c| c.is_alphanumeric()).unwrap_or(last))
}

/// Whether `c` belongs to a script that separates words with spaces. Chinese,
/// Japanese, Thai, Lao, Khmer and Myanmar text does not, nor does the
/// punctuation written with it. Korean does.
fn uses_spaces(c: char) -> bool {
    !matches!(
        c,
        '\u{0E00}'..='\u{0EFF}' // Thai, Lao
            | '\u{1000}'..='\u{109F}' // Myanmar
            | '\u{1780}'..='\u{17FF}' // Khmer
            | '\u{2E80}'..='\u{2FDF}' // CJK radicals
            | '\u{3000}'..='\u{30FF}' // CJK punctuation, Hiragana, Katakana
            | '\u{31F0}'..='\u{31FF}' // Katakana extensions
            | '\u{3400}'..='\u{4DBF}' // CJK extension A
            | '\u{4E00}'..='\u{9FFF}' // CJK unified ideographs
            | '\u{F900}'..='\u{FAFF}' // CJK compatibility ideographs
            | '\u{FF00}'..='\u{FFEF}' // fullwidth and halfwidth forms
            | '\u{20000}'..='\u{3FFFF}' // CJK extensions B and later
    )
}

fn common_prefix_char_count(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

#[derive(Debug)]
struct ProcessedIncoming {
    events: Vec<StreamingEvent>,
    committed: bool,
}

fn process_incoming_text(
    payload: &str,
    api_key: &str,
    reconciler: &mut TranscriptReconciler,
) -> Result<ProcessedIncoming, TranscribeError> {
    process_message(parse_wire_message(payload, api_key)?, reconciler)
}

fn process_message(
    message: WireMessage,
    reconciler: &mut TranscriptReconciler,
) -> Result<ProcessedIncoming, TranscribeError> {
    if let WireMessage::ProviderError { kind, message } = &message {
        return Err(TranscribeError::InferenceFailed(format!(
            "ElevenLabs realtime {kind}: {message}"
        )));
    }

    let committed = matches!(message, WireMessage::Committed { .. });
    Ok(ProcessedIncoming {
        events: reconciler.process(message),
        committed,
    })
}

/// Process a message that arrived while the session finalizes.
///
/// A partial after a final starts no new cursor segment. It is either a stale
/// echo of that final, or speech the stop's commit still covers: a VAD commit
/// in flight when the stop committed answers first, and the stop's own final
/// follows. New speech leaves the drain waiting for that final.
fn process_drain_text(
    payload: &str,
    api_key: &str,
    reconciler: &mut TranscriptReconciler,
    lifecycle: &mut RealtimeLifecycle,
) -> Result<ProcessedIncoming, TranscribeError> {
    let message = parse_wire_message(payload, api_key)?;
    if lifecycle.final_received() {
        if let WireMessage::Partial { text } = &message {
            if is_new_speech(text, &reconciler.last_committed) {
                lifecycle.note_speech_after_final();
                reconciler.unsent_partial.clone_from(text);
            }
            return Ok(ProcessedIncoming {
                events: Vec::new(),
                committed: false,
            });
        }
    }
    process_message(message, reconciler)
}

/// Hand a message that arrived while the session finalizes to the daemon,
/// and move the drain on: a committed transcript after the stop's commit
/// confirms a final once the daemon has it, so a final the daemon never
/// receives fails the drain instead of ending it cleanly.
#[allow(
    clippy::too_many_arguments,
    reason = "drain delivery shares lifecycle and deadline state"
)]
async fn deliver_drain_text(
    payload: &str,
    api_key: &str,
    reconciler: &mut TranscriptReconciler,
    events_tx: &mpsc::Sender<StreamingEvent>,
    cancel: &mut CancelSignal,
    deadline: tokio::time::Instant,
    lifecycle: &mut RealtimeLifecycle,
) -> Result<(), SessionExit> {
    let processed = process_drain_text(payload, api_key, reconciler, lifecycle)
        .map_err(|error| fatal_exit(lifecycle, error))?;
    match emit_streaming_events_until(events_tx, processed.events, cancel, deadline).await {
        AsyncDelivery::Delivered => {}
        AsyncDelivery::Cancelled => return Err(cancelled_exit(lifecycle)),
        AsyncDelivery::ReceiverClosed => return Err(SessionExit::Clean),
        AsyncDelivery::DeadlineElapsed if processed.committed => {
            return Err(fatal_exit(
                lifecycle,
                TranscribeError::NetworkError(
                    "ElevenLabs realtime final could not be delivered within 10 seconds"
                        .to_string(),
                ),
            ));
        }
        AsyncDelivery::DeadlineElapsed => {
            return Err(
                drain_timer_exit(lifecycle, SessionSignal::DrainDeadlineElapsed)
                    .expect("finalization deadline always terminates"),
            );
        }
    }
    if processed.committed {
        lifecycle.note_committed();
    }
    Ok(())
}

/// Whether a partial after a final says something other than that final. An
/// echo repeats the final's words; case, punctuation and spacing may differ.
/// A phrase that only begins like the final is new speech.
fn is_new_speech(partial: &str, last_committed: &str) -> bool {
    let words = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let partial = words(partial);
    !partial.is_empty() && partial != words(last_committed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionPhase {
    Connecting,
    AwaitingSession,
    Streaming,
    FinalizingCommit,
    /// The stop's commit went out and no final has answered it yet. With
    /// `first_final_ambiguous`, the first final may answer a VAD commit that
    /// was in flight instead, with the stop's own final still to come.
    DrainingAwaitingFinal {
        first_final_ambiguous: bool,
    },
    /// A final answered and the provider has reported no new speech since.
    /// The drain ends if it stays quiet until `quiet_until`.
    DrainingFinalConfirmed {
        quiet_until: tokio::time::Instant,
    },
    /// After a final, the provider reported speech it has not committed. The
    /// stop's commit covers it, so its final is still outstanding.
    DrainingSpeechOutstanding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionSignal {
    Cancelled,
    FatalError,
    SocketClosed,
    QuietPeriodElapsed,
    DrainDeadlineElapsed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionFailure {
    FatalError,
    SocketClosedBeforeFinal,
    FinalResponseTimeout,
    SpeechUncommittedAtDeadline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionDecision {
    Continue,
    EndCleanly,
    Fail(SessionFailure),
}

#[derive(Debug)]
struct RealtimeLifecycle {
    phase: SessionPhase,
    /// The provider also commits on its own VAD.
    vad_commits: bool,
}

impl RealtimeLifecycle {
    fn new() -> Self {
        Self {
            phase: SessionPhase::Connecting,
            vad_commits: false,
        }
    }

    fn with_vad_commits(mut self, vad_commits: bool) -> Self {
        self.vad_commits = vad_commits;
        self
    }

    fn connected(&mut self) {
        self.phase = SessionPhase::AwaitingSession;
    }

    fn session_started(&mut self) {
        self.phase = SessionPhase::Streaming;
    }

    fn begin_finalization(&mut self) {
        self.phase = SessionPhase::FinalizingCommit;
    }

    /// The stop's commit went out. The protocol does not say which commit a
    /// committed transcript answers, so with VAD commits and speech still
    /// open, the first final may answer a VAD commit that was in flight.
    fn explicit_commit_sent(&mut self, speech_open: bool) {
        self.phase = SessionPhase::DrainingAwaitingFinal {
            first_final_ambiguous: self.vad_commits && speech_open,
        };
    }

    /// A committed transcript arrived. After the stop's commit it confirms a
    /// final, and the provider must then stay quiet for a while before the
    /// drain ends: longer after an ambiguous first final, so a slow answer to
    /// the stop's own commit is not dropped. Returns whether it confirmed one.
    fn note_committed(&mut self) -> bool {
        let quiet_period = match self.phase {
            SessionPhase::DrainingAwaitingFinal {
                first_final_ambiguous: true,
            } => AMBIGUOUS_FINAL_QUIET_PERIOD,
            SessionPhase::DrainingAwaitingFinal { .. }
            | SessionPhase::DrainingFinalConfirmed { .. }
            | SessionPhase::DrainingSpeechOutstanding => POST_FINAL_QUIET_PERIOD,
            _ => return false,
        };
        self.phase = SessionPhase::DrainingFinalConfirmed {
            quiet_until: tokio::time::Instant::now() + quiet_period,
        };
        true
    }

    /// Whether a final answered the stop's commit.
    fn final_received(&self) -> bool {
        matches!(
            self.phase,
            SessionPhase::DrainingFinalConfirmed { .. } | SessionPhase::DrainingSpeechOutstanding
        )
    }

    /// The provider reported new speech after a final: its final is due.
    fn note_speech_after_final(&mut self) {
        if self.final_received() {
            self.phase = SessionPhase::DrainingSpeechOutstanding;
        }
    }

    /// When the drain ends if the provider stays quiet.
    fn quiet_deadline(&self) -> Option<tokio::time::Instant> {
        match self.phase {
            SessionPhase::DrainingFinalConfirmed { quiet_until } => Some(quiet_until),
            _ => None,
        }
    }

    fn decide(&self, signal: SessionSignal) -> SessionDecision {
        match signal {
            SessionSignal::Cancelled => SessionDecision::EndCleanly,
            SessionSignal::FatalError => SessionDecision::Fail(SessionFailure::FatalError),
            SessionSignal::SocketClosed => match self.phase {
                SessionPhase::DrainingFinalConfirmed { .. } => SessionDecision::EndCleanly,
                _ => SessionDecision::Fail(SessionFailure::SocketClosedBeforeFinal),
            },
            SessionSignal::QuietPeriodElapsed => match self.phase {
                SessionPhase::DrainingFinalConfirmed { .. } => SessionDecision::EndCleanly,
                _ => SessionDecision::Continue,
            },
            SessionSignal::DrainDeadlineElapsed => match self.phase {
                SessionPhase::FinalizingCommit | SessionPhase::DrainingAwaitingFinal { .. } => {
                    SessionDecision::Fail(SessionFailure::FinalResponseTimeout)
                }
                SessionPhase::DrainingSpeechOutstanding => {
                    SessionDecision::Fail(SessionFailure::SpeechUncommittedAtDeadline)
                }
                SessionPhase::DrainingFinalConfirmed { .. } => SessionDecision::EndCleanly,
                _ => SessionDecision::Continue,
            },
        }
    }
}

#[derive(Debug)]
enum SessionExit {
    Clean,
    Cancelled,
    Fatal(TranscribeError),
}

struct StreamEpilogue {
    error_permit: mpsc::OwnedPermit<StreamingEvent>,
    ended_permit: mpsc::OwnedPermit<StreamingEvent>,
}

impl StreamEpilogue {
    fn reserve(events_tx: &mpsc::Sender<StreamingEvent>) -> Option<Self> {
        let error_permit = events_tx.clone().try_reserve_owned().ok()?;
        let ended_permit = events_tx.clone().try_reserve_owned().ok()?;
        Some(Self {
            error_permit,
            ended_permit,
        })
    }

    fn finish(self, exit: SessionExit) {
        let Self {
            error_permit,
            ended_permit,
        } = self;
        if let SessionExit::Fatal(error) = exit {
            let _ = error_permit.send(StreamingEvent::Error(error));
        } else {
            drop(error_permit);
        }
        let _ = ended_permit.send(StreamingEvent::Ended);
    }
}

fn cancelled_exit(lifecycle: &RealtimeLifecycle) -> SessionExit {
    debug_assert_eq!(
        lifecycle.decide(SessionSignal::Cancelled),
        SessionDecision::EndCleanly
    );
    SessionExit::Cancelled
}

fn fatal_exit(lifecycle: &RealtimeLifecycle, error: TranscribeError) -> SessionExit {
    debug_assert_eq!(
        lifecycle.decide(SessionSignal::FatalError),
        SessionDecision::Fail(SessionFailure::FatalError)
    );
    SessionExit::Fatal(error)
}

fn close_exit(lifecycle: &RealtimeLifecycle) -> SessionExit {
    match lifecycle.decide(SessionSignal::SocketClosed) {
        SessionDecision::EndCleanly => SessionExit::Clean,
        SessionDecision::Fail(SessionFailure::SocketClosedBeforeFinal) => {
            SessionExit::Fatal(TranscribeError::NetworkError(
                "ElevenLabs realtime connection closed before the final commit response"
                    .to_string(),
            ))
        }
        _ => unreachable!("socket-close decision has an exhaustive lifecycle mapping"),
    }
}

fn drain_timer_exit(lifecycle: &RealtimeLifecycle, signal: SessionSignal) -> Option<SessionExit> {
    match lifecycle.decide(signal) {
        SessionDecision::Continue => None,
        SessionDecision::EndCleanly => Some(SessionExit::Clean),
        SessionDecision::Fail(SessionFailure::FinalResponseTimeout) => {
            Some(SessionExit::Fatal(TranscribeError::NetworkError(
                "ElevenLabs realtime finalization timed out after 10 seconds".to_string(),
            )))
        }
        SessionDecision::Fail(SessionFailure::SpeechUncommittedAtDeadline) => {
            Some(SessionExit::Fatal(TranscribeError::NetworkError(
                "ElevenLabs realtime finalization timed out after 10 seconds with speech \
                 still uncommitted"
                    .to_string(),
            )))
        }
        SessionDecision::Fail(_) => {
            unreachable!("drain timers cannot produce another failure")
        }
    }
}

fn should_attempt_close(exit: &SessionExit) -> bool {
    matches!(exit, SessionExit::Clean)
}

struct CancelSignal {
    receiver: Option<oneshot::Receiver<()>>,
}

impl CancelSignal {
    fn new(receiver: oneshot::Receiver<()>) -> Self {
        Self {
            receiver: Some(receiver),
        }
    }

    async fn requested(&mut self) {
        loop {
            let result = match self.receiver.as_mut() {
                Some(receiver) => receiver.await,
                None => pending().await,
            };
            if result.is_ok() {
                return;
            }
            self.receiver = None;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AsyncDelivery {
    Delivered,
    Cancelled,
    ReceiverClosed,
    DeadlineElapsed,
}

async fn send_websocket_messages<S>(
    write: &mut S,
    messages: Vec<Message>,
    cancel: &mut CancelSignal,
    api_key: &str,
) -> Result<AsyncDelivery, TranscribeError>
where
    S: Sink<Message, Error = WebSocketError> + Unpin,
{
    for message in messages {
        tokio::select! {
            biased;
            _ = cancel.requested() => return Ok(AsyncDelivery::Cancelled),
            result = write.send(message) => {
                result.map_err(|error| websocket_error("send failed", error, api_key))?;
            }
        }
    }
    Ok(AsyncDelivery::Delivered)
}

async fn send_websocket_messages_until<S>(
    write: &mut S,
    messages: Vec<Message>,
    cancel: &mut CancelSignal,
    api_key: &str,
    deadline: tokio::time::Instant,
) -> Result<AsyncDelivery, TranscribeError>
where
    S: Sink<Message, Error = WebSocketError> + Unpin,
{
    for message in messages {
        let deadline_timer = tokio::time::sleep_until(deadline);
        tokio::pin!(deadline_timer);
        tokio::select! {
            biased;
            _ = cancel.requested() => return Ok(AsyncDelivery::Cancelled),
            _ = &mut deadline_timer => return Ok(AsyncDelivery::DeadlineElapsed),
            result = write.send(message) => {
                result.map_err(|error| websocket_error("send failed", error, api_key))?;
            }
        }
    }
    Ok(AsyncDelivery::Delivered)
}

async fn send_audio_messages<S>(
    write: &mut S,
    messages: Vec<String>,
    cancel: &mut CancelSignal,
    api_key: &str,
) -> Result<AsyncDelivery, TranscribeError>
where
    S: Sink<Message, Error = WebSocketError> + Unpin,
{
    send_websocket_messages(
        write,
        messages.into_iter().map(Message::Text).collect(),
        cancel,
        api_key,
    )
    .await
}

async fn emit_streaming_events(
    events_tx: &mpsc::Sender<StreamingEvent>,
    events: Vec<StreamingEvent>,
    cancel: &mut CancelSignal,
) -> AsyncDelivery {
    for event in events {
        tokio::select! {
            biased;
            _ = cancel.requested() => return AsyncDelivery::Cancelled,
            result = events_tx.send(event) => {
                if result.is_err() {
                    return AsyncDelivery::ReceiverClosed;
                }
            }
        }
    }
    AsyncDelivery::Delivered
}

async fn emit_streaming_events_until(
    events_tx: &mpsc::Sender<StreamingEvent>,
    events: Vec<StreamingEvent>,
    cancel: &mut CancelSignal,
    deadline: tokio::time::Instant,
) -> AsyncDelivery {
    for event in events {
        let deadline_timer = tokio::time::sleep_until(deadline);
        tokio::pin!(deadline_timer);
        tokio::select! {
            biased;
            _ = cancel.requested() => return AsyncDelivery::Cancelled,
            _ = &mut deadline_timer => return AsyncDelivery::DeadlineElapsed,
            result = events_tx.send(event) => {
                if result.is_err() {
                    return AsyncDelivery::ReceiverClosed;
                }
            }
        }
    }
    AsyncDelivery::Delivered
}

fn websocket_error(context: &str, error: WebSocketError, api_key: &str) -> TranscribeError {
    TranscribeError::NetworkError(format!(
        "ElevenLabs realtime {context}: {}",
        redact_and_bound(&error.to_string(), api_key)
    ))
}

fn session_start_timeout_error() -> TranscribeError {
    TranscribeError::NetworkError(
        "ElevenLabs realtime session did not start within 10 seconds".to_string(),
    )
}

type RealtimeSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

enum ConnectActivity {
    Connected(Result<Box<RealtimeSocket>, WebSocketError>),
    Audio(Option<Vec<f32>>),
}

enum SessionActivity {
    Incoming(Option<Result<Message, WebSocketError>>),
    Audio(Option<Vec<f32>>),
}

enum FinalizationActivity {
    Incoming(Option<Result<Message, WebSocketError>>),
    Sent(Result<(), WebSocketError>),
}

enum DrainActivity {
    Incoming(Option<Result<Message, WebSocketError>>),
    QuietElapsed,
}

async fn run_streaming_session(
    request: Request<()>,
    api_key: &str,
    type_partials: bool,
    vad_commits: bool,
    mut samples_rx: mpsc::Receiver<Vec<f32>>,
    events_tx: &mpsc::Sender<StreamingEvent>,
    cancel_rx: oneshot::Receiver<()>,
) -> SessionExit {
    let mut lifecycle = RealtimeLifecycle::new().with_vad_commits(vad_commits);
    let mut cancel = CancelSignal::new(cancel_rx);
    let mut pre_session = PreSessionAudioBuffer::default();
    let mut samples_closed = false;
    let start_deadline = tokio::time::Instant::now() + SESSION_START_TIMEOUT;
    let start_timer = tokio::time::sleep_until(start_deadline);
    tokio::pin!(start_timer);
    let connect = tokio_tungstenite::connect_async(request);
    tokio::pin!(connect);

    let websocket = loop {
        let activity = tokio::select! {
            biased;
            _ = cancel.requested() => return cancelled_exit(&lifecycle),
            _ = &mut start_timer => {
                return fatal_exit(&lifecycle, session_start_timeout_error());
            }
            activity = async {
                tokio::select! {
                    result = &mut connect => {
                        ConnectActivity::Connected(
                            result.map(|(socket, _response)| Box::new(socket)),
                        )
                    }
                    chunk = samples_rx.recv(), if !samples_closed => {
                        ConnectActivity::Audio(chunk)
                    }
                }
            } => activity,
        };

        match activity {
            ConnectActivity::Connected(Ok(websocket)) => break websocket,
            ConnectActivity::Connected(Err(error)) => {
                let error = websocket_error("connection failed", error, api_key);
                return fatal_exit(&lifecycle, error);
            }
            ConnectActivity::Audio(Some(chunk)) => {
                if let Err(error) = pre_session.push(chunk) {
                    return fatal_exit(&lifecycle, error);
                }
            }
            ConnectActivity::Audio(None) => samples_closed = true,
        }
    };

    lifecycle.connected();
    let (mut write, mut read) = websocket.split();
    let exit = run_connected_session(
        &mut write,
        &mut read,
        api_key,
        type_partials,
        &mut samples_rx,
        events_tx,
        &mut cancel,
        pre_session,
        samples_closed,
        start_deadline,
        &mut lifecycle,
    )
    .await;

    if should_attempt_close(&exit) {
        let _ =
            tokio::time::timeout(WEBSOCKET_CLOSE_TIMEOUT, write.send(Message::Close(None))).await;
    }
    exit
}

#[allow(
    clippy::too_many_arguments,
    reason = "the session loop owns one instance of each lifecycle resource"
)]
async fn run_connected_session<S, R>(
    write: &mut S,
    read: &mut R,
    api_key: &str,
    type_partials: bool,
    samples_rx: &mut mpsc::Receiver<Vec<f32>>,
    events_tx: &mpsc::Sender<StreamingEvent>,
    cancel: &mut CancelSignal,
    mut pre_session: PreSessionAudioBuffer,
    mut samples_closed: bool,
    start_deadline: tokio::time::Instant,
    lifecycle: &mut RealtimeLifecycle,
) -> SessionExit
where
    S: Sink<Message, Error = WebSocketError> + Unpin,
    R: futures_util::Stream<Item = Result<Message, WebSocketError>> + Unpin,
{
    loop {
        let start_timer = tokio::time::sleep_until(start_deadline);
        tokio::pin!(start_timer);
        let activity = tokio::select! {
            biased;
            _ = cancel.requested() => return cancelled_exit(lifecycle),
            _ = &mut start_timer => {
                return fatal_exit(lifecycle, session_start_timeout_error());
            }
            activity = async {
                tokio::select! {
                    incoming = read.next() => SessionActivity::Incoming(incoming),
                    chunk = samples_rx.recv(), if !samples_closed => {
                        SessionActivity::Audio(chunk)
                    }
                }
            } => activity,
        };

        match activity {
            SessionActivity::Audio(Some(chunk)) => {
                if let Err(error) = pre_session.push(chunk) {
                    return fatal_exit(lifecycle, error);
                }
            }
            SessionActivity::Audio(None) => samples_closed = true,
            SessionActivity::Incoming(incoming) => {
                let message = match incoming {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => {
                        let error = websocket_error("receive failed", error, api_key);
                        return fatal_exit(lifecycle, error);
                    }
                    None => return close_exit(lifecycle),
                };

                match message {
                    Message::Text(payload) => {
                        let parsed = match parse_wire_message(payload.as_ref(), api_key) {
                            Ok(parsed) => parsed,
                            Err(error) => return fatal_exit(lifecycle, error),
                        };
                        match parsed {
                            WireMessage::SessionStarted { .. } => {
                                lifecycle.session_started();
                                break;
                            }
                            WireMessage::ProviderError { kind, message } => {
                                let error = TranscribeError::InferenceFailed(format!(
                                    "ElevenLabs realtime {kind}: {message}"
                                ));
                                return fatal_exit(lifecycle, error);
                            }
                            WireMessage::Unknown => {}
                            _ => {
                                let error = TranscribeError::InferenceFailed(
                                    "ElevenLabs realtime transcript arrived before session_started"
                                        .to_string(),
                                );
                                return fatal_exit(lifecycle, error);
                            }
                        }
                    }
                    Message::Ping(payload) => {
                        match send_websocket_messages_until(
                            write,
                            vec![Message::Pong(payload)],
                            cancel,
                            api_key,
                            start_deadline,
                        )
                        .await
                        {
                            Ok(AsyncDelivery::Delivered) => {}
                            Ok(AsyncDelivery::Cancelled) => return cancelled_exit(lifecycle),
                            Ok(AsyncDelivery::DeadlineElapsed) => {
                                return fatal_exit(lifecycle, session_start_timeout_error());
                            }
                            Ok(AsyncDelivery::ReceiverClosed) => unreachable!(),
                            Err(error) => return fatal_exit(lifecycle, error),
                        }
                    }
                    Message::Close(_) => return close_exit(lifecycle),
                    _ => {}
                }
            }
        }
    }

    let mut encoder = RealtimeAudioEncoder::default();
    for chunk in pre_session.into_chunks() {
        let messages = match encoder.push(&chunk) {
            Ok(messages) => messages,
            Err(error) => return fatal_exit(lifecycle, error),
        };
        match send_audio_messages(write, messages, cancel, api_key).await {
            Ok(AsyncDelivery::Delivered) => {}
            Ok(AsyncDelivery::Cancelled) => return cancelled_exit(lifecycle),
            Ok(AsyncDelivery::ReceiverClosed | AsyncDelivery::DeadlineElapsed) => unreachable!(),
            Err(error) => return fatal_exit(lifecycle, error),
        }
    }

    let mut reconciler = TranscriptReconciler::new(type_partials);
    if !samples_closed {
        loop {
            let activity = tokio::select! {
                biased;
                _ = cancel.requested() => return cancelled_exit(lifecycle),
                activity = async {
                    tokio::select! {
                        incoming = read.next() => SessionActivity::Incoming(incoming),
                        chunk = samples_rx.recv() => SessionActivity::Audio(chunk),
                    }
                } => activity,
            };

            match activity {
                SessionActivity::Audio(Some(chunk)) => {
                    let messages = match encoder.push(&chunk) {
                        Ok(messages) => messages,
                        Err(error) => return fatal_exit(lifecycle, error),
                    };
                    match send_audio_messages(write, messages, cancel, api_key).await {
                        Ok(AsyncDelivery::Delivered) => {}
                        Ok(AsyncDelivery::Cancelled) => return cancelled_exit(lifecycle),
                        Ok(AsyncDelivery::ReceiverClosed | AsyncDelivery::DeadlineElapsed) => {
                            unreachable!()
                        }
                        Err(error) => return fatal_exit(lifecycle, error),
                    }
                }
                SessionActivity::Audio(None) => {
                    samples_closed = true;
                    break;
                }
                SessionActivity::Incoming(incoming) => {
                    let message = match incoming {
                        Some(Ok(message)) => message,
                        Some(Err(error)) => {
                            let error = websocket_error("receive failed", error, api_key);
                            return fatal_exit(lifecycle, error);
                        }
                        None => return close_exit(lifecycle),
                    };

                    match message {
                        Message::Text(payload) => {
                            let processed = match process_incoming_text(
                                payload.as_ref(),
                                api_key,
                                &mut reconciler,
                            ) {
                                Ok(processed) => processed,
                                Err(error) => return fatal_exit(lifecycle, error),
                            };
                            match emit_streaming_events(events_tx, processed.events, cancel).await {
                                AsyncDelivery::Delivered => {}
                                AsyncDelivery::Cancelled => return cancelled_exit(lifecycle),
                                AsyncDelivery::ReceiverClosed => return SessionExit::Clean,
                                AsyncDelivery::DeadlineElapsed => unreachable!(),
                            }
                        }
                        Message::Ping(payload) => {
                            match send_websocket_messages(
                                write,
                                vec![Message::Pong(payload)],
                                cancel,
                                api_key,
                            )
                            .await
                            {
                                Ok(AsyncDelivery::Delivered) => {}
                                Ok(AsyncDelivery::Cancelled) => return cancelled_exit(lifecycle),
                                Ok(
                                    AsyncDelivery::ReceiverClosed | AsyncDelivery::DeadlineElapsed,
                                ) => unreachable!(),
                                Err(error) => return fatal_exit(lifecycle, error),
                            }
                        }
                        Message::Close(_) => return close_exit(lifecycle),
                        _ => {}
                    }
                }
            }
        }
    }

    debug_assert!(samples_closed);
    let exit = finish_after_stop(
        write,
        read,
        api_key,
        encoder,
        &mut reconciler,
        events_tx,
        cancel,
        lifecycle,
    )
    .await;
    if matches!(exit, SessionExit::Fatal(_)) {
        if let Some(tail) = reconciler.take_unconfirmed_tail() {
            tracing::warn!(
                "ElevenLabs realtime finalization failed; appending the last partial unconfirmed"
            );
            if events_tx.try_send(tail).is_err() {
                tracing::warn!("ElevenLabs realtime could not deliver the unconfirmed partial");
            }
        }
    }
    exit
}

/// Send the last audio and the stop's commit, then drain the provider's
/// answers until the final is confirmed or the drain fails.
#[allow(
    clippy::too_many_arguments,
    reason = "finalization owns the session's I/O, transcript and lifecycle state"
)]
async fn finish_after_stop<S, R>(
    write: &mut S,
    read: &mut R,
    api_key: &str,
    mut encoder: RealtimeAudioEncoder,
    reconciler: &mut TranscriptReconciler,
    events_tx: &mpsc::Sender<StreamingEvent>,
    cancel: &mut CancelSignal,
    lifecycle: &mut RealtimeLifecycle,
) -> SessionExit
where
    S: Sink<Message, Error = WebSocketError> + Unpin,
    R: futures_util::Stream<Item = Result<Message, WebSocketError>> + Unpin,
{
    lifecycle.begin_finalization();
    let drain_deadline = tokio::time::Instant::now() + FINAL_DRAIN_TIMEOUT;
    let mut messages = match encoder.finish() {
        Ok(messages) => messages,
        Err(error) => return fatal_exit(lifecycle, error),
    };
    let Some(final_commit) = messages.pop() else {
        return fatal_exit(
            lifecycle,
            TranscribeError::InferenceFailed(
                "ElevenLabs realtime encoder omitted the final commit".to_string(),
            ),
        );
    };
    for tail in messages {
        if let Err(exit) = send_finalization_message(
            write,
            read,
            tail,
            api_key,
            reconciler,
            events_tx,
            cancel,
            drain_deadline,
            lifecycle,
        )
        .await
        {
            return exit;
        }
    }
    if let Err(exit) = send_explicit_commit(
        write,
        read,
        final_commit,
        api_key,
        reconciler,
        events_tx,
        cancel,
        drain_deadline,
        lifecycle,
    )
    .await
    {
        return exit;
    }
    loop {
        let current_quiet_deadline = lifecycle.quiet_deadline();
        let quiet_timer = async move {
            match current_quiet_deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => pending().await,
            }
        };
        tokio::pin!(quiet_timer);
        let hard_timer = tokio::time::sleep_until(drain_deadline);
        tokio::pin!(hard_timer);

        let activity = tokio::select! {
            biased;
            _ = cancel.requested() => return cancelled_exit(lifecycle),
            _ = &mut hard_timer => {
                return drain_timer_exit(lifecycle, SessionSignal::DrainDeadlineElapsed)
                    .expect("final drain deadline always terminates");
            }
            activity = async {
                tokio::select! {
                    biased;
                    incoming = read.next() => DrainActivity::Incoming(incoming),
                    _ = &mut quiet_timer => DrainActivity::QuietElapsed,
                }
            } => activity,
        };

        match activity {
            DrainActivity::QuietElapsed => {
                if let Some(exit) = drain_timer_exit(lifecycle, SessionSignal::QuietPeriodElapsed) {
                    return exit;
                }
            }
            DrainActivity::Incoming(incoming) => {
                let message = match incoming {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => {
                        let error = websocket_error("receive failed", error, api_key);
                        return fatal_exit(lifecycle, error);
                    }
                    None => return close_exit(lifecycle),
                };

                match message {
                    Message::Text(payload) => {
                        if let Err(exit) = deliver_drain_text(
                            payload.as_ref(),
                            api_key,
                            reconciler,
                            events_tx,
                            cancel,
                            drain_deadline,
                            lifecycle,
                        )
                        .await
                        {
                            return exit;
                        }
                    }
                    Message::Ping(payload) => {
                        match send_websocket_messages_until(
                            write,
                            vec![Message::Pong(payload)],
                            cancel,
                            api_key,
                            drain_deadline,
                        )
                        .await
                        {
                            Ok(AsyncDelivery::Delivered) => {}
                            Ok(AsyncDelivery::Cancelled) => return cancelled_exit(lifecycle),
                            Ok(AsyncDelivery::DeadlineElapsed) => {
                                return drain_timer_exit(
                                    lifecycle,
                                    SessionSignal::DrainDeadlineElapsed,
                                )
                                .expect("final drain deadline always terminates");
                            }
                            Ok(AsyncDelivery::ReceiverClosed) => unreachable!(),
                            Err(error) => return fatal_exit(lifecycle, error),
                        }
                    }
                    Message::Close(_) => return close_exit(lifecycle),
                    _ => {}
                }
            }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "finalization must keep the read and write halves under one deadline"
)]
async fn send_finalization_message<S, R>(
    write: &mut S,
    read: &mut R,
    message: String,
    api_key: &str,
    reconciler: &mut TranscriptReconciler,
    events_tx: &mpsc::Sender<StreamingEvent>,
    cancel: &mut CancelSignal,
    deadline: tokio::time::Instant,
    lifecycle: &mut RealtimeLifecycle,
) -> Result<(), SessionExit>
where
    S: Sink<Message, Error = WebSocketError> + Unpin,
    R: futures_util::Stream<Item = Result<Message, WebSocketError>> + Unpin,
{
    let send = write.send(Message::Text(message));
    tokio::pin!(send);

    loop {
        let deadline_timer = tokio::time::sleep_until(deadline);
        tokio::pin!(deadline_timer);
        let activity = tokio::select! {
            biased;
            _ = cancel.requested() => return Err(cancelled_exit(lifecycle)),
            _ = &mut deadline_timer => {
                return Err(
                    drain_timer_exit(lifecycle, SessionSignal::DrainDeadlineElapsed)
                        .expect("finalization deadline always terminates"),
                );
            }
            activity = async {
                tokio::select! {
                    incoming = read.next() => FinalizationActivity::Incoming(incoming),
                    result = &mut send => FinalizationActivity::Sent(result),
                }
            } => activity,
        };

        match activity {
            FinalizationActivity::Sent(Ok(())) => return Ok(()),
            FinalizationActivity::Sent(Err(error)) => {
                return Err(fatal_exit(
                    lifecycle,
                    websocket_error("send failed", error, api_key),
                ));
            }
            FinalizationActivity::Incoming(incoming) => {
                handle_finalization_incoming(
                    incoming, api_key, reconciler, events_tx, cancel, deadline, lifecycle,
                )
                .await?;
            }
        }
    }
}

async fn await_finalization_write<F>(
    operation: F,
    api_key: &str,
    cancel: &mut CancelSignal,
    deadline: tokio::time::Instant,
    lifecycle: &RealtimeLifecycle,
) -> Result<(), SessionExit>
where
    F: Future<Output = Result<(), WebSocketError>>,
{
    tokio::pin!(operation);
    let deadline_timer = tokio::time::sleep_until(deadline);
    tokio::pin!(deadline_timer);
    tokio::select! {
        biased;
        _ = cancel.requested() => Err(cancelled_exit(lifecycle)),
        _ = &mut deadline_timer => {
            Err(
                drain_timer_exit(lifecycle, SessionSignal::DrainDeadlineElapsed)
                    .expect("finalization deadline always terminates"),
            )
        }
        result = &mut operation => {
            result.map_err(|error| {
                fatal_exit(
                    lifecycle,
                    websocket_error("send failed", error, api_key),
                )
            })
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the commit boundary must share lifecycle, I/O, and deadline state"
)]
async fn send_explicit_commit<S, R>(
    write: &mut S,
    read: &mut R,
    message: String,
    api_key: &str,
    reconciler: &mut TranscriptReconciler,
    events_tx: &mpsc::Sender<StreamingEvent>,
    cancel: &mut CancelSignal,
    deadline: tokio::time::Instant,
    lifecycle: &mut RealtimeLifecycle,
) -> Result<(), SessionExit>
where
    S: Sink<Message, Error = WebSocketError> + Unpin,
    R: futures_util::Stream<Item = Result<Message, WebSocketError>> + Unpin,
{
    // Drain a bounded ready backlog before starting the explicit commit.
    // A Ping read here may queue an automatic Pong in tungstenite.
    for _ in 0..MAX_PRE_COMMIT_DRAIN_MESSAGES {
        let Some(incoming) = read.next().now_or_never() else {
            break;
        };
        handle_finalization_incoming(
            incoming, api_key, reconciler, events_tx, cancel, deadline, lifecycle,
        )
        .await?;
    }

    // Flush any automatic control frame queued by the pre-commit drain.
    await_finalization_write(write.flush(), api_key, cancel, deadline, lifecycle).await?;

    // Do not poll inbound messages while feeding or flushing the commit. Any
    // response remains queued until the lifecycle is armed below. The
    // protocol has no request or segment ID, so successful flush is the
    // strongest observable boundary for "received after explicit commit".
    await_finalization_write(
        write.feed(Message::Text(message)),
        api_key,
        cancel,
        deadline,
        lifecycle,
    )
    .await?;
    await_finalization_write(write.flush(), api_key, cancel, deadline, lifecycle).await?;

    lifecycle.explicit_commit_sent(reconciler.segment_open);
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "incoming finalization events share lifecycle and deadline state"
)]
async fn handle_finalization_incoming(
    incoming: Option<Result<Message, WebSocketError>>,
    api_key: &str,
    reconciler: &mut TranscriptReconciler,
    events_tx: &mpsc::Sender<StreamingEvent>,
    cancel: &mut CancelSignal,
    deadline: tokio::time::Instant,
    lifecycle: &mut RealtimeLifecycle,
) -> Result<(), SessionExit> {
    let message = match incoming {
        Some(Ok(message)) => message,
        Some(Err(error)) => {
            return Err(fatal_exit(
                lifecycle,
                websocket_error("receive failed", error, api_key),
            ));
        }
        None => return Err(close_exit(lifecycle)),
    };

    match message {
        Message::Text(payload) => {
            deliver_drain_text(
                payload.as_ref(),
                api_key,
                reconciler,
                events_tx,
                cancel,
                deadline,
                lifecycle,
            )
            .await?;
        }
        // tungstenite queues the matching Pong while reading. The pinned
        // feed/send/flush future writes it without duplicating protocol data.
        Message::Ping(_) => {}
        Message::Close(_) => return Err(close_exit(lifecycle)),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        pin::Pin,
        sync::{Arc, Mutex},
        task::{Context, Poll},
    };

    use super::*;
    use crate::config::ElevenLabsRegion;

    const API_KEY: &str = "elevenlabs-secret-key";

    #[derive(Default)]
    struct CommitOrderState {
        actions: Vec<&'static str>,
        control_pending: bool,
        control_flushed_by_ready: bool,
        commit_pending: bool,
        response_ready: bool,
    }

    struct CommitOrderSink {
        state: Arc<Mutex<CommitOrderState>>,
    }

    impl Sink<Message> for CommitOrderSink {
        type Error = WebSocketError;

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            let mut state = self.state.lock().unwrap();
            if state.control_pending {
                state.actions.push("control_flush_during_feed");
                state.control_pending = false;
                state.control_flushed_by_ready = true;
            }
            Poll::Ready(Ok(()))
        }

        fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            let Message::Text(payload) = item else {
                panic!("expected explicit commit text frame");
            };
            let payload: serde_json::Value = serde_json::from_str(payload.as_ref()).unwrap();
            assert_eq!(payload["commit"], true);

            let mut state = self.state.lock().unwrap();
            state.actions.push("commit_feed");
            state.commit_pending = true;
            if state.control_flushed_by_ready {
                state.response_ready = true;
            }
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            let mut state = self.state.lock().unwrap();
            if state.control_pending {
                state.actions.push("control_flush");
                state.control_pending = false;
            }
            if state.commit_pending {
                state.actions.push("commit_flush");
                state.commit_pending = false;
                state.response_ready = true;
            }
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum EventView {
        Partial(String, SegmentId),
        RevisePartial(usize, String, SegmentId),
        Final(String, SegmentId),
        Replace(usize, String, SegmentId),
        Error(String),
        Ended,
    }

    fn event_views(events: Vec<StreamingEvent>) -> Vec<EventView> {
        events
            .into_iter()
            .map(|event| match event {
                StreamingEvent::Partial { text, segment_id } => {
                    EventView::Partial(text, segment_id)
                }
                StreamingEvent::RevisePartial {
                    backspace,
                    text,
                    segment_id,
                } => EventView::RevisePartial(backspace, text, segment_id),
                StreamingEvent::Final { text, segment_id } => EventView::Final(text, segment_id),
                StreamingEvent::Replace {
                    backspace,
                    text,
                    segment_id,
                } => EventView::Replace(backspace, text, segment_id),
                StreamingEvent::Error(error) => EventView::Error(error.to_string()),
                StreamingEvent::Ended => EventView::Ended,
            })
            .collect()
    }

    fn process_all(
        reconciler: &mut TranscriptReconciler,
        messages: impl IntoIterator<Item = WireMessage>,
    ) -> Vec<EventView> {
        event_views(
            messages
                .into_iter()
                .flat_map(|message| reconciler.process(message))
                .collect(),
        )
    }

    #[test]
    fn accumulates_irregular_chunks_into_100ms_frames_and_flushes_tail() {
        let samples: Vec<f32> = (0..(SAMPLES_PER_FRAME * 2 + 137))
            .map(|index| (index as f32 % 101.0 - 50.0) / 50.0)
            .collect();
        let expected = f32_to_pcm16le(&samples);
        let chunk_sizes = [17, 1_701, 33, 999, 480, 107];
        assert_eq!(chunk_sizes.iter().sum::<usize>(), samples.len());

        let mut accumulator = PcmFrameAccumulator::default();
        let mut frames = Vec::new();
        let mut offset = 0;
        for chunk_size in chunk_sizes {
            frames.extend(accumulator.push(&samples[offset..offset + chunk_size]));
            offset += chunk_size;
        }
        let tail = accumulator.flush().expect("short tail is retained");

        assert_eq!(
            frames.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![BYTES_PER_FRAME, BYTES_PER_FRAME]
        );
        assert_eq!(tail.len(), 137 * size_of::<i16>());
        let reconstructed: Vec<u8> = frames.into_iter().flatten().chain(tail).collect();
        assert_eq!(reconstructed, expected);
        assert!(accumulator.flush().is_none());
    }

    #[test]
    fn serializes_normal_audio_and_final_commit_messages() {
        let audio: serde_json::Value =
            serde_json::from_str(&serialize_audio_chunk(&[0, 1, 2, 3]).unwrap()).unwrap();
        assert_eq!(
            audio,
            serde_json::json!({
                "message_type": "input_audio_chunk",
                "audio_base_64": "AAECAw==",
                "commit": false,
                "sample_rate": 16000
            })
        );

        let commit: serde_json::Value =
            serde_json::from_str(&serialize_final_commit().unwrap()).unwrap();
        assert_eq!(
            commit,
            serde_json::json!({
                "message_type": "input_audio_chunk",
                "audio_base_64": "",
                "commit": true,
                "sample_rate": 16000
            })
        );
    }

    #[test]
    fn constructs_sensitive_regional_requests_without_putting_key_in_uri() {
        let cases = [
            (ElevenLabsRegion::Global, "api.elevenlabs.io"),
            (ElevenLabsRegion::Us, "api.us.elevenlabs.io"),
            (ElevenLabsRegion::Eu, "api.eu.residency.elevenlabs.io"),
            (ElevenLabsRegion::India, "api.in.residency.elevenlabs.io"),
            (
                ElevenLabsRegion::Singapore,
                "api.sg.residency.elevenlabs.io",
            ),
        ];

        for (region, expected_host) in cases {
            let config = ElevenLabsConfig {
                region,
                language_code: Some("en".to_string()),
                commit_strategy: ElevenLabsCommitStrategy::Vad,
                no_verbatim: true,
                vad_silence_threshold_secs: 0.65,
                realtime_model: "scribe_v9_live".to_string(),
                ..ElevenLabsConfig::default()
            };
            let request = realtime_request(&config, API_KEY).unwrap();
            let uri = request.uri().to_string();
            let url = reqwest::Url::parse(&uri).unwrap();
            let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
            let expected_query = BTreeMap::from([
                ("audio_format".to_string(), "pcm_16000".to_string()),
                ("commit_strategy".to_string(), "vad".to_string()),
                ("language_code".to_string(), "en".to_string()),
                ("min_silence_duration_ms".to_string(), "100".to_string()),
                ("min_speech_duration_ms".to_string(), "100".to_string()),
                ("model_id".to_string(), "scribe_v9_live".to_string()),
                ("no_verbatim".to_string(), "true".to_string()),
                ("vad_silence_threshold_secs".to_string(), "0.65".to_string()),
                ("vad_threshold".to_string(), "0.4".to_string()),
            ]);

            assert_eq!(
                (url.scheme(), url.host_str(), url.path(), query),
                (
                    "wss",
                    Some(expected_host),
                    "/v1/speech-to-text/realtime",
                    expected_query
                )
            );
            assert!(!uri.contains(API_KEY));
            let header = request.headers().get(&API_KEY_HEADER).unwrap();
            assert_eq!(header, API_KEY);
            assert!(header.is_sensitive());
        }
    }

    #[test]
    fn manual_realtime_request_omits_vad_tuning() {
        let config = ElevenLabsConfig {
            commit_strategy: ElevenLabsCommitStrategy::Manual,
            no_verbatim: false,
            vad_silence_threshold_secs: 0.65,
            ..ElevenLabsConfig::default()
        };
        let request = realtime_request(&config, API_KEY).unwrap();
        let uri = request.uri().to_string();
        let url = reqwest::Url::parse(&uri).unwrap();
        let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();

        assert_eq!(
            query,
            BTreeMap::from([
                ("audio_format".to_string(), "pcm_16000".to_string()),
                ("commit_strategy".to_string(), "manual".to_string()),
                ("model_id".to_string(), "scribe_v3_live".to_string()),
                ("no_verbatim".to_string(), "false".to_string()),
            ])
        );
        assert!(!uri.contains(API_KEY));
        assert!(request
            .headers()
            .get(&API_KEY_HEADER)
            .unwrap()
            .is_sensitive());
    }

    #[test]
    fn realtime_request_keeps_commit_strategy_and_cleanup_independent() {
        for (strategy, expected_strategy) in [
            (ElevenLabsCommitStrategy::Vad, "vad"),
            (ElevenLabsCommitStrategy::Manual, "manual"),
        ] {
            for no_verbatim in [false, true] {
                let config = ElevenLabsConfig {
                    commit_strategy: strategy,
                    no_verbatim,
                    ..ElevenLabsConfig::default()
                };
                let request = realtime_request(&config, API_KEY).unwrap();
                let url = reqwest::Url::parse(&request.uri().to_string()).unwrap();
                let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();

                assert_eq!(
                    (
                        query.get("commit_strategy").map(String::as_str),
                        query.get("no_verbatim").map(String::as_str),
                    ),
                    (
                        Some(expected_strategy),
                        Some(if no_verbatim { "true" } else { "false" }),
                    ),
                );
            }
        }
    }

    #[test]
    fn omits_absent_language_and_rejects_invalid_header_bytes() {
        let config = ElevenLabsConfig::default();
        let request = realtime_request(&config, API_KEY).unwrap();
        assert!(!request.uri().to_string().contains("language_code"));

        let error = realtime_request(&config, "invalid\nkey").unwrap_err();
        let surfaced = error.to_string();
        assert!(surfaced.contains("cannot be used in an HTTP header"));
        assert!(!surfaced.contains("invalid"));
    }

    #[test]
    fn parses_documented_realtime_message_fixtures() {
        let cases = [
            (
                r#"{
                    "message_type": "session_started",
                    "session_id": "session-123",
                    "config": {
                        "sample_rate": 16000,
                        "audio_format": "pcm_16000",
                        "language_code": "en",
                        "commit_strategy": "vad",
                        "vad_silence_threshold_secs": 1.5,
                        "vad_threshold": 0.4,
                        "min_speech_duration_ms": 100,
                        "min_silence_duration_ms": 100,
                        "model_id": "scribe_v3_live",
                        "enable_logging": true,
                        "include_timestamps": false,
                        "include_language_detection": false,
                        "keyterms": [],
                        "no_verbatim": false
                    }
                }"#,
                WireMessage::SessionStarted {
                    session_id: "session-123".to_string(),
                },
            ),
            (
                r#"{"message_type":"partial_transcript","text":"hello wor"}"#,
                WireMessage::Partial {
                    text: "hello wor".to_string(),
                },
            ),
            (
                r#"{"message_type":"committed_transcript","text":"hello world"}"#,
                WireMessage::Committed {
                    text: "hello world".to_string(),
                },
            ),
            (
                r#"{
                    "message_type": "committed_transcript_with_timestamps",
                    "text": "hello world",
                    "language_code": "en",
                    "words": [{
                        "text": "hello",
                        "start": 0.0,
                        "end": 0.4,
                        "type": "word",
                        "speaker_id": "speaker_0",
                        "logprob": -0.1,
                        "characters": ["h", "e", "l", "l", "o"]
                    }, {
                        "text": " ",
                        "start": 0.4,
                        "end": 0.45,
                        "type": "spacing",
                        "speaker_id": "speaker_0",
                        "logprob": 0.0,
                        "characters": [" "]
                    }, {
                        "text": "world",
                        "start": 0.45,
                        "end": 0.9,
                        "type": "word",
                        "speaker_id": "speaker_0",
                        "logprob": -0.2,
                        "characters": ["w", "o", "r", "l", "d"]
                    }]
                }"#,
                WireMessage::CommittedWithTimestamps {
                    text: "hello world".to_string(),
                },
            ),
            (
                r#"{"message_type":"auth_error","error":"invalid key elevenlabs-secret-key"}"#,
                WireMessage::ProviderError {
                    kind: "auth_error".to_string(),
                    message: "invalid key [REDACTED]".to_string(),
                },
            ),
            (
                r#"{"message_type":"future-elevenlabs-secret-key-event","value":42}"#,
                WireMessage::Unknown,
            ),
        ];

        for (fixture, expected) in cases {
            assert_eq!(parse_wire_message(fixture, API_KEY).unwrap(), expected);
        }
    }

    #[test]
    fn rejects_malformed_realtime_json_without_echoing_payload() {
        let fixture = r#"{"message_type":"partial_transcript","text":"elevenlabs-secret-key""#;
        let error = parse_wire_message(fixture, API_KEY).unwrap_err();
        let surfaced = error.to_string();

        assert!(surfaced.contains("Malformed ElevenLabs realtime message"));
        assert!(!surfaced.contains(API_KEY));
    }

    #[test]
    fn reconciles_repeated_appended_divergent_retracted_and_unicode_partials() {
        struct Case {
            name: &'static str,
            existing: &'static str,
            snapshot: &'static str,
            expected_second: Option<EventView>,
        }
        let cases = [
            Case {
                name: "append",
                existing: "hello",
                snapshot: "hello world",
                expected_second: Some(EventView::Partial(" world".to_string(), 0)),
            },
            Case {
                name: "repeat",
                existing: "hello",
                snapshot: "hello",
                expected_second: None,
            },
            Case {
                name: "punctuation correction",
                existing: "This is difficult.",
                snapshot: "This is difficult because",
                expected_second: Some(EventView::RevisePartial(1, " because".to_string(), 0)),
            },
            Case {
                name: "word correction",
                existing: "hello wurld",
                snapshot: "hello world",
                expected_second: Some(EventView::RevisePartial(4, "orld".to_string(), 0)),
            },
            Case {
                name: "retraction",
                existing: "hello world",
                snapshot: "hello",
                expected_second: Some(EventView::RevisePartial(6, String::new(), 0)),
            },
            Case {
                name: "unicode scalar correction",
                existing: "naïf",
                snapshot: "naïve",
                expected_second: Some(EventView::RevisePartial(1, "ve".to_string(), 0)),
            },
            Case {
                name: "unicode scalar suffix retraction",
                existing: "你好",
                snapshot: "你们",
                expected_second: Some(EventView::RevisePartial(1, "们".to_string(), 0)),
            },
        ];

        for case in cases {
            let mut reconciler = TranscriptReconciler::new(true);
            let events = process_all(
                &mut reconciler,
                [
                    WireMessage::Partial {
                        text: case.existing.to_string(),
                    },
                    WireMessage::Partial {
                        text: case.snapshot.to_string(),
                    },
                ],
            );
            let mut expected = vec![EventView::Partial(case.existing.to_string(), 0)];
            expected.extend(case.expected_second);
            assert_eq!(events, expected, "{}", case.name);
            assert_eq!(reconciler.typed_partial, case.snapshot, "{}", case.name);
            assert_eq!(reconciler.segment_id, 0, "{}", case.name);
        }
    }

    #[test]
    fn revised_partial_remains_provisional_until_matching_commit() {
        let mut reconciler = TranscriptReconciler::new(true);
        let events = process_all(
            &mut reconciler,
            [
                WireMessage::Partial {
                    text: "This is difficult.".to_string(),
                },
                WireMessage::Partial {
                    text: "This is difficult because".to_string(),
                },
                WireMessage::Committed {
                    text: "This is difficult because".to_string(),
                },
                WireMessage::Partial {
                    text: "next".to_string(),
                },
            ],
        );

        assert_eq!(
            events,
            vec![
                EventView::Partial("This is difficult.".to_string(), 0),
                EventView::RevisePartial(1, " because".to_string(), 0),
                EventView::Final(String::new(), 0),
                EventView::Partial(" next".to_string(), 1),
            ]
        );
        assert_eq!(
            (reconciler.typed_partial.as_str(), reconciler.segment_id),
            (" next", 1)
        );
    }

    #[test]
    fn reconciles_exact_extended_corrected_and_unicode_commits() {
        struct Case {
            name: &'static str,
            partial: &'static str,
            committed: &'static str,
            expected_commit: EventView,
        }
        let cases = [
            Case {
                name: "extension",
                partial: "hello",
                committed: "hello world",
                expected_commit: EventView::Final(" world".to_string(), 0),
            },
            Case {
                name: "exact",
                partial: "hello",
                committed: "hello",
                expected_commit: EventView::Final(String::new(), 0),
            },
            Case {
                name: "correction",
                partial: "hello wurld",
                committed: "hello world",
                expected_commit: EventView::Replace(4, "orld".to_string(), 0),
            },
            Case {
                name: "shorter correction",
                partial: "hello world",
                committed: "hello",
                expected_commit: EventView::Replace(6, String::new(), 0),
            },
            Case {
                name: "unicode scalar correction",
                partial: "naïf",
                committed: "naïve",
                expected_commit: EventView::Replace(1, "ve".to_string(), 0),
            },
            Case {
                name: "unicode scalar suffix retraction",
                partial: "你好",
                committed: "你们",
                expected_commit: EventView::Replace(1, "们".to_string(), 0),
            },
        ];

        for case in cases {
            let mut reconciler = TranscriptReconciler::new(true);
            let events = process_all(
                &mut reconciler,
                [
                    WireMessage::Partial {
                        text: case.partial.to_string(),
                    },
                    WireMessage::Committed {
                        text: case.committed.to_string(),
                    },
                ],
            );
            assert_eq!(
                events,
                vec![
                    EventView::Partial(case.partial.to_string(), 0),
                    case.expected_commit
                ],
                "{}",
                case.name
            );
            assert_eq!(reconciler.typed_partial, "", "{}", case.name);
            assert_eq!(reconciler.segment_id, 1, "{}", case.name);
        }
    }

    #[test]
    fn committed_only_mode_ignores_divergent_partials_and_uses_sequential_segments() {
        let mut reconciler = TranscriptReconciler::new(false);
        let events = process_all(
            &mut reconciler,
            [
                WireMessage::Partial {
                    text: "This is difficult.".to_string(),
                },
                WireMessage::Partial {
                    text: "This is difficult because".to_string(),
                },
                WireMessage::Committed {
                    text: "This is difficult because".to_string(),
                },
                WireMessage::Partial {
                    text: "second dra".to_string(),
                },
                WireMessage::Committed {
                    text: "second draft".to_string(),
                },
            ],
        );

        assert_eq!(
            events,
            vec![
                EventView::Final("This is difficult because".to_string(), 0),
                EventView::Final(" second draft".to_string(), 1),
            ]
        );
        assert_eq!(reconciler.segment_id, 2);
    }

    #[test]
    fn partial_mode_clears_state_between_sequential_vad_segments() {
        let mut reconciler = TranscriptReconciler::new(true);
        let events = process_all(
            &mut reconciler,
            [
                WireMessage::Partial {
                    text: "first".to_string(),
                },
                WireMessage::Committed {
                    text: "first".to_string(),
                },
                WireMessage::Partial {
                    text: "second".to_string(),
                },
                WireMessage::Committed {
                    text: "second segment".to_string(),
                },
            ],
        );

        assert_eq!(
            events,
            vec![
                EventView::Partial("first".to_string(), 0),
                EventView::Final(String::new(), 0),
                EventView::Partial(" second".to_string(), 1),
                EventView::Final(" segment".to_string(), 1),
            ]
        );
    }

    #[test]
    fn sequential_segments_are_separated_only_where_a_word_ends() {
        let mut reconciler = TranscriptReconciler::new(false);
        let events = process_all(
            &mut reconciler,
            [
                WireMessage::Committed {
                    text: "first".to_string(),
                },
                WireMessage::Committed {
                    text: String::new(),
                },
                WireMessage::Committed {
                    text: "second ".to_string(),
                },
                WireMessage::Committed {
                    text: "third".to_string(),
                },
                WireMessage::Committed {
                    text: " fourth".to_string(),
                },
            ],
        );

        assert_eq!(
            events,
            vec![
                EventView::Final("first".to_string(), 0),
                EventView::Final(String::new(), 1),
                EventView::Final(" second ".to_string(), 2),
                EventView::Final("third".to_string(), 3),
                EventView::Final(" fourth".to_string(), 4),
            ]
        );
    }

    /// Typed text for a run of messages: every event applied to a cursor.
    fn typed(type_partials: bool, messages: Vec<WireMessage>) -> String {
        let mut reconciler = TranscriptReconciler::new(type_partials);
        let mut cursor: Vec<char> = Vec::new();
        for event in messages.into_iter().flat_map(|m| reconciler.process(m)) {
            match event {
                StreamingEvent::Partial { text, .. } | StreamingEvent::Final { text, .. } => {
                    cursor.extend(text.chars());
                }
                StreamingEvent::RevisePartial {
                    backspace, text, ..
                }
                | StreamingEvent::Replace {
                    backspace, text, ..
                } => {
                    cursor.truncate(cursor.len() - backspace);
                    cursor.extend(text.chars());
                }
                _ => {}
            }
        }
        cursor.into_iter().collect()
    }

    fn committed(text: &str) -> WireMessage {
        WireMessage::Committed {
            text: text.to_string(),
        }
    }

    fn partial(text: &str) -> WireMessage {
        WireMessage::Partial {
            text: text.to_string(),
        }
    }

    #[test]
    fn segments_join_with_a_space_only_between_spaced_scripts() {
        for type_partials in [false, true] {
            assert_eq!(
                typed(type_partials, vec![committed("Hello."), committed("World")]),
                "Hello. World"
            );
            assert_eq!(
                typed(type_partials, vec![committed("one"), committed(", two")]),
                "one, two"
            );
            assert_eq!(
                typed(
                    type_partials,
                    vec![
                        partial("今日は"),
                        committed("今日は"),
                        partial("雨"),
                        committed("雨です。")
                    ]
                ),
                "今日は雨です。"
            );
            assert_eq!(
                typed(type_partials, vec![committed("ok"), committed("ไปไหน")]),
                "okไปไหน"
            );
            assert_eq!(
                typed(
                    type_partials,
                    vec![committed("안녕하세요"), committed("반갑습니다")]
                ),
                "안녕하세요 반갑습니다"
            );
            assert_eq!(
                typed(type_partials, vec![committed("ไปไหน?"), committed("2026")]),
                "ไปไหน?2026"
            );
            assert_eq!(
                typed(type_partials, vec![committed("Done?"), committed("2026")]),
                "Done? 2026"
            );
        }
    }

    /// A partial without a leading space and its commit with one put a
    /// single space at the cursor.
    #[test]
    fn a_commit_that_brings_its_own_space_does_not_double_it() {
        assert_eq!(
            typed(
                true,
                vec![
                    committed("first"),
                    partial("second"),
                    committed(" second"),
                    committed("third"),
                ]
            ),
            "first second third"
        );
    }

    #[test]
    fn ignores_timestamp_followup_after_committed_transcript() {
        let mut reconciler = TranscriptReconciler::new(false);
        let events = process_all(
            &mut reconciler,
            [
                WireMessage::Committed {
                    text: "one transcript".to_string(),
                },
                WireMessage::CommittedWithTimestamps {
                    text: "one transcript".to_string(),
                },
            ],
        );

        assert_eq!(
            events,
            vec![EventView::Final("one transcript".to_string(), 0)]
        );
        assert_eq!(reconciler.segment_id, 1);
    }

    #[test]
    fn production_incoming_path_turns_redacted_provider_errors_fatal() {
        let mut reconciler = TranscriptReconciler::new(false);
        let error = process_incoming_text(
            r#"{"message_type":"auth_error","error":"bad elevenlabs-secret-key"}"#,
            API_KEY,
            &mut reconciler,
        )
        .unwrap_err()
        .to_string();

        assert_eq!(
            error,
            "Transcription failed: ElevenLabs realtime auth_error: bad [REDACTED]"
        );
        assert!(!error.contains(API_KEY));
        assert_eq!(reconciler.segment_id, 0);
    }

    /// serde quotes a string it cannot use, so a response that echoes the
    /// key back where an object belongs must not carry it into the error.
    #[test]
    fn malformed_responses_never_carry_the_api_key_into_the_error() {
        for payload in [
            r#""elevenlabs-secret-key""#,
            r#"{"message_type":["elevenlabs-secret-key"]}"#,
            r#"{"message_type":"partial_transcript","text":{"elevenlabs-secret-key":1}}"#,
        ] {
            let mut reconciler = TranscriptReconciler::new(false);
            let error = process_incoming_text(payload, API_KEY, &mut reconciler)
                .unwrap_err()
                .to_string();
            assert!(!error.contains(API_KEY), "{error}");
            assert!(error.contains("Malformed ElevenLabs"), "{error}");
        }
    }

    #[test]
    fn preserves_pre_session_audio_order_and_rejects_overflow() {
        let mut ordered = PreSessionAudioBuffer::default();
        ordered.push(vec![1.0, 2.0]).unwrap();
        ordered.push(Vec::new()).unwrap();
        ordered.push(vec![3.0]).unwrap();
        ordered.push(vec![4.0, 5.0]).unwrap();

        let flattened: Vec<f32> = ordered.into_chunks().into_iter().flatten().collect();
        assert_eq!(flattened, vec![1.0, 2.0, 3.0, 4.0, 5.0]);

        let mut bounded = PreSessionAudioBuffer::default();
        bounded
            .push(vec![0.0; PRE_SESSION_AUDIO_LIMIT_SAMPLES - 1])
            .unwrap();
        bounded.push(vec![0.0]).unwrap();
        let error = bounded.push(vec![0.0]).unwrap_err().to_string();

        assert!(error.contains("10-second pre-session audio buffer"));
        assert_eq!(bounded.sample_count, PRE_SESSION_AUDIO_LIMIT_SAMPLES);
        assert_eq!(
            bounded.chunks.iter().map(Vec::len).sum::<usize>(),
            PRE_SESSION_AUDIO_LIMIT_SAMPLES
        );
    }

    #[test]
    fn encodes_ordered_frames_short_tail_and_exactly_one_final_commit() {
        let samples: Vec<f32> = (0..SAMPLES_PER_FRAME + 3)
            .map(|index| (index as f32 % 19.0 - 9.0) / 9.0)
            .collect();
        let expected_pcm = f32_to_pcm16le(&samples);
        let mut encoder = RealtimeAudioEncoder::default();
        let mut messages = encoder.push(&samples[..701]).unwrap();
        messages.extend(encoder.push(&samples[701..]).unwrap());
        messages.extend(encoder.finish().unwrap());
        messages.extend(encoder.finish().unwrap());

        let decoded: Vec<(Vec<u8>, bool)> = messages
            .iter()
            .map(|message| {
                let value: serde_json::Value = serde_json::from_str(message).unwrap();
                let audio = base64::engine::general_purpose::STANDARD
                    .decode(value["audio_base_64"].as_str().unwrap())
                    .unwrap();
                (audio, value["commit"].as_bool().unwrap())
            })
            .collect();

        assert_eq!(
            decoded
                .iter()
                .map(|(audio, commit)| (audio.len(), *commit))
                .collect::<Vec<_>>(),
            vec![(BYTES_PER_FRAME, false), (6, false), (0, true)]
        );
        assert_eq!(
            decoded
                .iter()
                .filter(|(_, commit)| !commit)
                .flat_map(|(audio, _)| audio.iter().copied())
                .collect::<Vec<_>>(),
            expected_pcm
        );
        assert_eq!(decoded.iter().filter(|(_, commit)| *commit).count(), 1);
        assert!(encoder
            .push(&[0.0])
            .unwrap_err()
            .to_string()
            .contains("after the final commit"));
    }

    #[test]
    fn parses_and_reconciles_complete_documented_event_sequence() {
        let fixtures = [
            r#"{"message_type":"session_started","session_id":"session-123","config":{"sample_rate":16000}}"#,
            r#"{"message_type":"partial_transcript","text":"hel"}"#,
            r#"{"message_type":"partial_transcript","text":"hello"}"#,
            r#"{"message_type":"committed_transcript","text":"hello world"}"#,
            r#"{"message_type":"committed_transcript_with_timestamps","text":"hello world","language_code":"en","words":[]}"#,
            r#"{"message_type":"partial_transcript","text":"naïf"}"#,
            r#"{"message_type":"committed_transcript","text":"naïve"}"#,
        ];
        let mut reconciler = TranscriptReconciler::new(true);
        let mut events = Vec::new();
        let mut committed_flags = Vec::new();

        for fixture in fixtures {
            let processed = process_incoming_text(fixture, API_KEY, &mut reconciler).unwrap();
            committed_flags.push(processed.committed);
            events.extend(processed.events);
        }

        assert_eq!(
            event_views(events),
            vec![
                EventView::Partial("hel".to_string(), 0),
                EventView::Partial("lo".to_string(), 0),
                EventView::Final(" world".to_string(), 0),
                EventView::Partial(" naïf".to_string(), 1),
                EventView::Replace(1, "ve".to_string(), 1),
            ]
        );
        assert_eq!(
            committed_flags,
            vec![false, false, false, true, false, false, true]
        );
    }

    fn lifecycle_phases() -> Vec<RealtimeLifecycle> {
        let connecting = RealtimeLifecycle::new();
        let mut awaiting = RealtimeLifecycle::new();
        awaiting.connected();
        let mut streaming = RealtimeLifecycle::new();
        streaming.connected();
        streaming.session_started();
        let mut finalizing = RealtimeLifecycle::new();
        finalizing.connected();
        finalizing.session_started();
        finalizing.begin_finalization();
        let mut awaiting_final = RealtimeLifecycle::new();
        awaiting_final.connected();
        awaiting_final.session_started();
        awaiting_final.begin_finalization();
        awaiting_final.explicit_commit_sent(false);
        let mut final_received = RealtimeLifecycle::new();
        final_received.connected();
        final_received.session_started();
        final_received.begin_finalization();
        final_received.explicit_commit_sent(false);
        assert!(final_received.note_committed());

        vec![
            connecting,
            awaiting,
            streaming,
            finalizing,
            awaiting_final,
            final_received,
        ]
    }

    #[test]
    fn cancellation_is_clean_and_provider_errors_are_fatal_in_every_phase() {
        for lifecycle in lifecycle_phases() {
            assert_eq!(
                lifecycle.decide(SessionSignal::Cancelled),
                SessionDecision::EndCleanly
            );
            assert_eq!(
                lifecycle.decide(SessionSignal::FatalError),
                SessionDecision::Fail(SessionFailure::FatalError)
            );
        }
    }

    #[test]
    fn socket_close_requires_a_post_commit_final_response() {
        let phases = lifecycle_phases();
        for lifecycle in &phases[..5] {
            assert_eq!(
                lifecycle.decide(SessionSignal::SocketClosed),
                SessionDecision::Fail(SessionFailure::SocketClosedBeforeFinal)
            );
        }
        assert_eq!(
            phases[5].decide(SessionSignal::SocketClosed),
            SessionDecision::EndCleanly
        );
    }

    #[test]
    fn final_drain_distinguishes_pre_commit_and_post_commit_transcripts() {
        let mut lifecycle = RealtimeLifecycle::new();
        lifecycle.connected();
        lifecycle.session_started();
        lifecycle.begin_finalization();

        assert!(!lifecycle.note_committed());
        assert_eq!(
            lifecycle.decide(SessionSignal::DrainDeadlineElapsed),
            SessionDecision::Fail(SessionFailure::FinalResponseTimeout)
        );

        lifecycle.explicit_commit_sent(false);
        assert!(lifecycle.note_committed());
        assert_eq!(
            lifecycle.decide(SessionSignal::QuietPeriodElapsed),
            SessionDecision::EndCleanly
        );
        assert_eq!(
            lifecycle.decide(SessionSignal::DrainDeadlineElapsed),
            SessionDecision::EndCleanly
        );
    }

    #[tokio::test]
    async fn final_drain_discards_stale_partial_after_a_qualified_commit() {
        let mut lifecycle = RealtimeLifecycle::new();
        lifecycle.connected();
        lifecycle.session_started();
        lifecycle.begin_finalization();
        lifecycle.explicit_commit_sent(false);
        let mut reconciler = TranscriptReconciler::new(true);
        let (events_tx, mut events_rx) = mpsc::channel(4);
        let (_cancel_tx, cancel_rx) = oneshot::channel();
        let mut cancel = CancelSignal::new(cancel_rx);
        let deadline = tokio::time::Instant::now() + FINAL_DRAIN_TIMEOUT;
        for json in [
            r#"{"message_type":"partial_transcript","text":"already committed"}"#,
            r#"{"message_type":"committed_transcript","text":"already committed"}"#,
            r#"{"message_type":"partial_transcript","text":"already committed"}"#,
        ] {
            deliver_drain_text(
                json,
                API_KEY,
                &mut reconciler,
                &events_tx,
                &mut cancel,
                deadline,
                &mut lifecycle,
            )
            .await
            .unwrap();
        }
        drop(events_tx);
        let mut events = Vec::new();
        while let Some(event) = events_rx.recv().await {
            events.push(event);
        }

        assert_eq!(
            event_views(events),
            vec![
                EventView::Partial("already committed".to_string(), 0),
                EventView::Final(String::new(), 0),
            ]
        );
        assert!(lifecycle.quiet_deadline().is_some());
        assert_eq!(
            (reconciler.typed_partial.as_str(), reconciler.segment_id),
            ("", 1)
        );
    }

    #[tokio::test]
    async fn ping_control_output_precedes_commit_and_final_qualification() {
        let state = Arc::new(Mutex::new(CommitOrderState::default()));
        let read_state = Arc::clone(&state);
        let mut ping_sent = false;
        let mut response_sent = false;
        let mut read = futures_util::stream::poll_fn(move |_cx| {
            let mut state = read_state.lock().unwrap();
            if !ping_sent {
                ping_sent = true;
                state.actions.push("ping_read");
                state.control_pending = true;
                return Poll::Ready(Some(Ok(Message::Ping(Vec::new()))));
            }
            if state.response_ready && !response_sent {
                response_sent = true;
                state.actions.push("response_read");
                return Poll::Ready(Some(Ok(Message::Text(
                    r#"{"message_type":"committed_transcript","text":"done"}"#.to_string(),
                ))));
            }
            Poll::Pending
        });
        let mut write = CommitOrderSink {
            state: Arc::clone(&state),
        };
        let (events_tx, mut events_rx) = mpsc::channel(4);
        let (_cancel_tx, cancel_rx) = oneshot::channel();
        let mut cancel = CancelSignal::new(cancel_rx);
        let mut lifecycle = RealtimeLifecycle::new();
        lifecycle.connected();
        lifecycle.session_started();
        lifecycle.begin_finalization();
        let mut reconciler = TranscriptReconciler::new(false);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);

        send_explicit_commit(
            &mut write,
            &mut read,
            serialize_final_commit().unwrap(),
            API_KEY,
            &mut reconciler,
            &events_tx,
            &mut cancel,
            deadline,
            &mut lifecycle,
        )
        .await
        .unwrap();

        assert_eq!(
            lifecycle.phase,
            SessionPhase::DrainingAwaitingFinal {
                first_final_ambiguous: false
            }
        );
        assert_eq!(
            state.lock().unwrap().actions,
            vec!["ping_read", "control_flush", "commit_feed", "commit_flush"]
        );

        let incoming = tokio::time::timeout(Duration::from_millis(50), read.next())
            .await
            .unwrap();
        handle_finalization_incoming(
            incoming,
            API_KEY,
            &mut reconciler,
            &events_tx,
            &mut cancel,
            deadline,
            &mut lifecycle,
        )
        .await
        .unwrap();

        assert!(lifecycle.quiet_deadline().is_some());
        assert!(matches!(
            events_rx.recv().await,
            Some(StreamingEvent::Final { text, .. }) if text == "done"
        ));
        assert_eq!(
            state.lock().unwrap().actions,
            vec![
                "ping_read",
                "control_flush",
                "commit_feed",
                "commit_flush",
                "response_read",
            ]
        );
    }

    /// A VAD commit the server made just before our stop commit answers
    /// first. The stop's own final, for speech after that commit, follows
    /// later than one quiet period, after the server's partial for it.
    #[tokio::test(start_paused = true)]
    async fn stop_final_after_an_in_flight_vad_commit_is_not_dropped() {
        let (server_tx, mut server_rx) = mpsc::unbounded_channel::<Message>();
        let mut read = Box::pin(futures_util::stream::poll_fn(move |cx| {
            server_rx.poll_recv(cx).map(|message| message.map(Ok))
        }));
        let (commit_tx, mut commit_rx) = mpsc::unbounded_channel::<()>();
        let mut write = Box::pin(futures_util::sink::unfold(
            commit_tx,
            |commit_tx, message: Message| async move {
                if matches!(&message, Message::Text(text) if text.contains(r#""commit":true"#)) {
                    let _ = commit_tx.send(());
                }
                Ok::<_, WebSocketError>(commit_tx)
            },
        ));
        let text = |json: &str| Message::Text(json.to_string());
        server_tx
            .send(text(
                r#"{"message_type":"session_started","session_id":"s"}"#,
            ))
            .unwrap();
        let server = tokio::spawn(async move {
            commit_rx.recv().await.unwrap();
            server_tx
                .send(text(
                    r#"{"message_type":"committed_transcript","text":"first"}"#,
                ))
                .unwrap();
            tokio::time::sleep(Duration::from_millis(600)).await;
            server_tx
                .send(text(
                    r#"{"message_type":"partial_transcript","text":"second"}"#,
                ))
                .unwrap();
            tokio::time::sleep(Duration::from_millis(900)).await;
            server_tx
                .send(text(
                    r#"{"message_type":"committed_transcript","text":"second"}"#,
                ))
                .unwrap();
            server_tx
        });

        let (samples_tx, mut samples_rx) = mpsc::channel(1);
        drop(samples_tx);
        let (events_tx, mut events_rx) = mpsc::channel(16);
        let (_cancel_tx, cancel_rx) = oneshot::channel();
        let mut lifecycle = RealtimeLifecycle::new();
        lifecycle.connected();
        let exit = run_connected_session(
            &mut write,
            &mut read,
            API_KEY,
            false,
            &mut samples_rx,
            &events_tx,
            &mut CancelSignal::new(cancel_rx),
            PreSessionAudioBuffer::default(),
            true,
            tokio::time::Instant::now() + SESSION_START_TIMEOUT,
            &mut lifecycle,
        )
        .await;
        let _server_tx = server.await.unwrap();
        drop(events_tx);

        let mut finals = Vec::new();
        while let Some(event) = events_rx.recv().await {
            if let StreamingEvent::Final { text, .. } = event {
                finals.push(text);
            }
        }
        assert!(matches!(exit, SessionExit::Clean), "{exit:?}");
        assert_eq!(finals, ["first", " second"]);
    }

    /// Run a session that is already stopped against a scripted server.
    /// `before_commit` is queued after `session_started`; each `after_commit`
    /// entry is sent that long after the previous one, starting at the
    /// stop's commit. Returns the finals, the exit and how long the drain ran.
    async fn drain_against_script(
        vad_commits: bool,
        before_commit: &[&str],
        after_commit: &[(u64, &str)],
    ) -> (Vec<String>, SessionExit, Duration) {
        let (events, exit, elapsed) = drain_events_against_script(
            false,
            vad_commits,
            DaemonQueue::Draining,
            before_commit,
            after_commit,
        )
        .await;
        let finals = events
            .into_iter()
            .filter_map(|event| match event {
                StreamingEvent::Final { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        (finals, exit, elapsed)
    }

    /// `drain_against_script` with every event the daemon received. With
    /// `daemon_stalled`, the event channel is full for the whole drain.
    /// How the daemon takes events off the session's channel in a drain test.
    #[derive(Clone, Copy, PartialEq)]
    enum DaemonQueue {
        /// Room for every event.
        Draining,
        /// Full from the start: no event reaches the daemon.
        Stalled,
        /// One free slot: the first event fits, later ones wait.
        OneSlot,
    }

    async fn drain_events_against_script(
        type_partials: bool,
        vad_commits: bool,
        queue: DaemonQueue,
        before_commit: &[&str],
        after_commit: &[(u64, &str)],
    ) -> (Vec<StreamingEvent>, SessionExit, Duration) {
        let (server_tx, mut server_rx) = mpsc::unbounded_channel::<Message>();
        let mut read = Box::pin(futures_util::stream::poll_fn(move |cx| {
            server_rx.poll_recv(cx).map(|message| message.map(Ok))
        }));
        let (commit_tx, mut commit_rx) = mpsc::unbounded_channel::<()>();
        let mut write = Box::pin(futures_util::sink::unfold(
            commit_tx,
            |commit_tx, message: Message| async move {
                if matches!(&message, Message::Text(text) if text.contains(r#""commit":true"#)) {
                    let _ = commit_tx.send(());
                }
                Ok::<_, WebSocketError>(commit_tx)
            },
        ));
        let text = |json: &str| Message::Text(json.to_string());
        server_tx
            .send(text(
                r#"{"message_type":"session_started","session_id":"s"}"#,
            ))
            .unwrap();
        for json in before_commit {
            server_tx.send(text(json)).unwrap();
        }
        let script: Vec<(u64, String)> = after_commit
            .iter()
            .map(|(delay, json)| (*delay, json.to_string()))
            .collect();
        let server = tokio::spawn(async move {
            commit_rx.recv().await.unwrap();
            for (delay, json) in script {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                server_tx.send(Message::Text(json)).unwrap();
            }
            server_tx
        });

        let (samples_tx, mut samples_rx) = mpsc::channel(1);
        drop(samples_tx);
        let capacity = if queue == DaemonQueue::Draining {
            16
        } else {
            1
        };
        let (events_tx, mut events_rx) = mpsc::channel(capacity);
        if queue == DaemonQueue::Stalled {
            events_tx.try_send(StreamingEvent::Ended).unwrap();
        }
        let (_cancel_tx, cancel_rx) = oneshot::channel();
        let mut lifecycle = RealtimeLifecycle::new().with_vad_commits(vad_commits);
        lifecycle.connected();
        let started = tokio::time::Instant::now();
        let exit = run_connected_session(
            &mut write,
            &mut read,
            API_KEY,
            type_partials,
            &mut samples_rx,
            &events_tx,
            &mut CancelSignal::new(cancel_rx),
            PreSessionAudioBuffer::default(),
            true,
            tokio::time::Instant::now() + SESSION_START_TIMEOUT,
            &mut lifecycle,
        )
        .await;
        let elapsed = started.elapsed();
        drop(server.await.unwrap());
        drop(events_tx);

        let mut events = Vec::new();
        while let Some(event) = events_rx.recv().await {
            events.push(event);
        }
        if queue == DaemonQueue::Stalled {
            // The event that kept the channel full.
            assert!(matches!(events.remove(0), StreamingEvent::Ended));
        }
        (events, exit, elapsed)
    }

    /// The text at the cursor once the daemon has handled `events` after the
    /// stop, where partials and finals only append.
    fn text_after_stop(events: &[StreamingEvent]) -> String {
        events
            .iter()
            .map(|event| match event {
                StreamingEvent::Partial { text, .. } | StreamingEvent::Final { text, .. } => {
                    text.as_str()
                }
                other => panic!("unexpected event after the stop: {other:?}"),
            })
            .collect()
    }

    const PARTIAL_FIRST: &str = r#"{"message_type":"partial_transcript","text":"first"}"#;
    const COMMITTED_FIRST: &str = r#"{"message_type":"committed_transcript","text":"first"}"#;
    const PARTIAL_SECOND: &str = r#"{"message_type":"partial_transcript","text":"second"}"#;
    const COMMITTED_SECOND: &str = r#"{"message_type":"committed_transcript","text":"second"}"#;

    /// The partial for speech after an in-flight VAD commit can come later
    /// than one quiet period. Once it does, the drain waits for its commit.
    #[tokio::test(start_paused = true)]
    async fn a_late_partial_after_an_in_flight_vad_commit_keeps_the_drain_open() {
        let (finals, exit, _) = drain_against_script(
            true,
            &[PARTIAL_FIRST],
            &[
                (0, COMMITTED_FIRST),
                (1_100, PARTIAL_SECOND),
                (1_500, COMMITTED_SECOND),
            ],
        )
        .await;
        assert!(matches!(exit, SessionExit::Clean), "{exit:?}");
        assert_eq!(finals, ["first", " second"]);
    }

    /// With VAD commits and speech open at the stop, the first final may
    /// answer the VAD commit, so a second final with no partial before it
    /// is still awaited for a while.
    #[tokio::test(start_paused = true)]
    async fn an_ambiguous_first_final_waits_for_a_slow_stop_final() {
        let (finals, exit, _) = drain_against_script(
            true,
            &[PARTIAL_FIRST],
            &[(0, COMMITTED_FIRST), (2_500, COMMITTED_SECOND)],
        )
        .await;
        assert!(matches!(exit, SessionExit::Clean), "{exit:?}");
        assert_eq!(finals, ["first", " second"]);
    }

    /// Where the first final cannot have answered a VAD commit, the drain
    /// still ends one quiet period after it.
    #[tokio::test(start_paused = true)]
    async fn an_unambiguous_final_ends_the_drain_after_one_quiet_period() {
        for (vad_commits, before_commit) in [(false, &[PARTIAL_FIRST][..]), (true, &[][..])] {
            let (finals, exit, elapsed) =
                drain_against_script(vad_commits, before_commit, &[(200, COMMITTED_FIRST)]).await;
            assert!(matches!(exit, SessionExit::Clean), "{exit:?}");
            assert_eq!(finals, ["first"]);
            assert_eq!(
                elapsed,
                Duration::from_millis(200) + POST_FINAL_QUIET_PERIOD
            );
        }
    }

    /// A stale echo of the committed text is not new speech.
    #[tokio::test(start_paused = true)]
    async fn an_echo_of_the_final_does_not_hold_the_drain_open() {
        let (finals, _, elapsed) = drain_against_script(
            false,
            &[PARTIAL_FIRST],
            &[(0, COMMITTED_FIRST), (100, PARTIAL_FIRST)],
        )
        .await;
        assert_eq!(finals, ["first"]);
        assert_eq!(elapsed, POST_FINAL_QUIET_PERIOD);
    }

    fn partial_json(text: &str) -> String {
        format!(r#"{{"message_type":"partial_transcript","text":"{text}"}}"#)
    }

    fn committed_json(text: &str) -> String {
        format!(r#"{{"message_type":"committed_transcript","text":"{text}"}}"#)
    }

    /// An echo that differs from the final only in case and punctuation is
    /// still an echo: the drain ends one quiet period after the final.
    #[tokio::test(start_paused = true)]
    async fn an_echo_in_another_case_does_not_hold_the_drain_open() {
        let (finals, exit, elapsed) = drain_against_script(
            false,
            &[&partial_json("Hello")],
            &[
                (0, &committed_json("Hello.")),
                (100, &partial_json("hello")),
            ],
        )
        .await;
        assert!(matches!(exit, SessionExit::Clean), "{exit:?}");
        assert_eq!(finals, ["Hello."]);
        assert_eq!(elapsed, POST_FINAL_QUIET_PERIOD);
    }

    /// New speech after the final whose commit never comes is not a clean
    /// stop: the session fails at the deadline. Text is already typed, so
    /// the daemon cannot recover from the audio without repeating it; the
    /// unconfirmed partial is appended instead of the words going missing.
    #[tokio::test(start_paused = true)]
    async fn new_speech_still_uncommitted_at_the_deadline_fails_the_session() {
        for type_partials in [false, true] {
            let (events, exit, elapsed) = drain_events_against_script(
                type_partials,
                false,
                DaemonQueue::Draining,
                &[PARTIAL_FIRST],
                &[(0, COMMITTED_FIRST), (100, PARTIAL_SECOND)],
            )
            .await;
            assert!(matches!(exit, SessionExit::Fatal(_)), "{exit:?}");
            assert_eq!(text_after_stop(&events), "first second");
            assert_eq!(elapsed, FINAL_DRAIN_TIMEOUT);
        }
    }

    /// With committed-only typing, speech after a VAD final reaches the
    /// drain only as a partial. If the stop's final never comes, that
    /// partial is appended after the typed final.
    #[tokio::test(start_paused = true)]
    async fn an_uncommitted_partial_after_a_vad_final_is_appended_at_the_deadline() {
        let (events, exit, _) = drain_events_against_script(
            false,
            true,
            DaemonQueue::Draining,
            &[COMMITTED_FIRST, PARTIAL_SECOND],
            &[],
        )
        .await;
        assert!(matches!(exit, SessionExit::Fatal(_)), "{exit:?}");
        assert_eq!(text_after_stop(&events), "first second");
    }

    /// With nothing typed, the daemon recovers the whole recording from its
    /// audio, so the drain appends no unconfirmed partial that would stop it.
    #[tokio::test(start_paused = true)]
    async fn an_uncommitted_partial_with_nothing_typed_is_left_to_recovery() {
        let (events, exit, _) =
            drain_events_against_script(false, false, DaemonQueue::Draining, &[PARTIAL_FIRST], &[])
                .await;
        assert!(matches!(exit, SessionExit::Fatal(_)), "{exit:?}");
        assert_eq!(text_after_stop(&events), "");
    }

    /// A final the daemon never receives because its event channel stays
    /// full is not a clean stop: the drain fails at the deadline.
    #[tokio::test(start_paused = true)]
    async fn a_final_the_daemon_never_receives_fails_the_drain() {
        let (events, exit, elapsed) = drain_events_against_script(
            false,
            false,
            DaemonQueue::Stalled,
            &[PARTIAL_FIRST],
            &[(0, COMMITTED_FIRST)],
        )
        .await;
        assert!(matches!(exit, SessionExit::Fatal(_)), "{exit:?}");
        assert_eq!(text_after_stop(&events), "");
        assert_eq!(elapsed, FINAL_DRAIN_TIMEOUT);
    }

    /// A stop's final that never reaches the daemon fails the drain even
    /// after an earlier VAD final was delivered and confirmed.
    #[tokio::test(start_paused = true)]
    async fn a_later_final_the_daemon_never_receives_fails_the_drain() {
        let (events, exit, _) = drain_events_against_script(
            false,
            true,
            DaemonQueue::OneSlot,
            &[PARTIAL_FIRST],
            &[(0, COMMITTED_FIRST), (100, COMMITTED_SECOND)],
        )
        .await;
        assert!(matches!(exit, SessionExit::Fatal(_)), "{exit:?}");
        assert_eq!(text_after_stop(&events), "first");
    }

    /// A partial that only repeats the typed final is never appended when the
    /// drain fails, so the cursor never gets the final twice.
    #[tokio::test(start_paused = true)]
    async fn an_echo_of_the_final_is_not_appended_at_the_deadline() {
        let echo = r#"{"message_type":"partial_transcript","text":"First."}"#;
        let (events, exit, _) = drain_events_against_script(
            false,
            true,
            DaemonQueue::Draining,
            &[COMMITTED_FIRST, echo],
            &[],
        )
        .await;
        assert!(matches!(exit, SessionExit::Fatal(_)), "{exit:?}");
        assert_eq!(text_after_stop(&events), "first");
    }

    /// A next phrase that begins with the final's first word is new speech,
    /// so the drain waits for its commit.
    #[tokio::test(start_paused = true)]
    async fn a_phrase_that_starts_like_the_final_is_new_speech() {
        let (finals, exit, _) = drain_against_script(
            false,
            &[&partial_json("turn left")],
            &[
                (0, &committed_json("turn left")),
                (100, &partial_json("turn")),
                (1_500, &committed_json("turn right")),
            ],
        )
        .await;
        assert!(matches!(exit, SessionExit::Clean), "{exit:?}");
        assert_eq!(finals, ["turn left", " turn right"]);
    }

    #[test]
    fn only_clean_completion_attempts_a_bounded_websocket_close() {
        let fatal = SessionExit::Fatal(TranscribeError::InferenceFailed("boom".to_string()));
        assert!(should_attempt_close(&SessionExit::Clean));
        assert!(!should_attempt_close(&SessionExit::Cancelled));
        assert!(!should_attempt_close(&fatal));
        assert_eq!(WEBSOCKET_CLOSE_TIMEOUT, Duration::from_secs(1));
    }

    #[tokio::test]
    async fn reserved_epilogue_stays_ordered_when_normal_capacity_is_full() {
        let (events_tx, mut events_rx) = mpsc::channel(4);
        let epilogue = StreamEpilogue::reserve(&events_tx).unwrap();
        events_tx
            .try_send(StreamingEvent::Partial {
                text: "one".to_string(),
                segment_id: 0,
            })
            .unwrap();
        events_tx
            .try_send(StreamingEvent::Partial {
                text: "two".to_string(),
                segment_id: 0,
            })
            .unwrap();
        assert!(events_tx
            .try_send(StreamingEvent::Final {
                text: "full".to_string(),
                segment_id: 0,
            })
            .is_err());

        epilogue.finish(SessionExit::Fatal(TranscribeError::InferenceFailed(
            "boom".to_string(),
        )));

        let events = vec![
            events_rx.recv().await.unwrap(),
            events_rx.recv().await.unwrap(),
            events_rx.recv().await.unwrap(),
            events_rx.recv().await.unwrap(),
        ];
        assert_eq!(
            event_views(events),
            vec![
                EventView::Partial("one".to_string(), 0),
                EventView::Partial("two".to_string(), 0),
                EventView::Error("Transcription failed: boom".to_string()),
                EventView::Ended,
            ]
        );
    }

    #[tokio::test]
    async fn cancellation_epilogue_emits_only_ended_at_saturated_capacity() {
        let (events_tx, mut events_rx) = mpsc::channel(2);
        let epilogue = StreamEpilogue::reserve(&events_tx).unwrap();

        epilogue.finish(SessionExit::Cancelled);

        assert!(matches!(
            events_rx.recv().await,
            Some(StreamingEvent::Ended)
        ));
        assert!(events_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn final_event_delivery_cannot_outlive_hard_deadline() {
        let (events_tx, _events_rx) = mpsc::channel(1);
        events_tx
            .try_send(StreamingEvent::Partial {
                text: "full".to_string(),
                segment_id: 0,
            })
            .unwrap();
        let (_cancel_tx, cancel_rx) = oneshot::channel();
        let mut cancel = CancelSignal::new(cancel_rx);
        let delivery = emit_streaming_events_until(
            &events_tx,
            vec![StreamingEvent::Final {
                text: "blocked".to_string(),
                segment_id: 0,
            }],
            &mut cancel,
            tokio::time::Instant::now() + Duration::from_millis(10),
        )
        .await;

        assert_eq!(delivery, AsyncDelivery::DeadlineElapsed);
    }
}
