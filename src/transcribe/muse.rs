//! Muse Voice Transcribe via Meta Model API.
//!
//! Endpoints per https://dev.meta.ai/docs/speech-to-text.md :
//! * Live: `wss://api.meta.ai/v1/asr/realtime?sessionId=...` — auth in handshake
//! * Batch: `POST https://api.meta.ai/v1/asr/transcribe?sessionId=...` — Bearer header

use super::streaming::{StreamHandle, StreamingEvent, StreamingTranscriber};
use super::Transcriber;
use crate::config::MuseConfig;
use crate::error::TranscribeError;
use futures_util::{SinkExt, StreamExt};
use std::io::Cursor;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const WS_TIMEOUT: Duration = Duration::from_secs(15);
// Voxtype's audio pipeline is 16kHz mono (`[audio] sample_rate`), and the
// Meta API supports PCM_16KHZ for live + 16k WAV for batch — so declare 16k
// rather than upsampling. Declaring PCM_24KHZ while sending 16k worth of
// samples/sec makes the server report "Ingress audio slower than real-time".
const SAMPLE_RATE: u32 = 16_000;
const AUDIO_ENCODING: &str = "PCM_16KHZ";

/// Muse transcriber (Meta Model API).
#[derive(Debug)]
pub struct MuseTranscriber {
    endpoint: String,
    ws_endpoint: String,
    model: String,
    language: Option<String>,
    api_key: String,
    timeout: Duration,
    streaming: bool,
    interim_results: bool,
    diarization: bool,
    keywords: Vec<String>,
}

impl MuseTranscriber {
    pub fn new(config: &MuseConfig) -> Result<Self, TranscribeError> {
        let api_key = config.resolve_api_key().ok_or_else(|| {
            TranscribeError::ConfigError(
                "Muse API key required: set [muse] api_key in config.toml or \
                 VOXTYPE_MUSE_API_KEY / MUSE_API_KEY / META_API_KEY env var"
                    .into(),
            )
        })?;

        let endpoint = config.effective_endpoint();
        if !endpoint.starts_with("http://")
            && !endpoint.starts_with("https://")
            && !endpoint.starts_with("ws://")
            && !endpoint.starts_with("wss://")
        {
            return Err(TranscribeError::ConfigError(format!(
                "muse endpoint must start with http(s):// or ws(s)://, got: {}",
                endpoint
            )));
        }

        let ws_endpoint = config.effective_ws_endpoint();

        tracing::info!(
            "Configured Muse transcriber: endpoint={}, ws={}, model={}, streaming={}, diarization={}, polish={}",
            endpoint,
            ws_endpoint,
            config.model,
            config.streaming,
            config.diarization,
            config.polish_command.is_some(),
        );

        Ok(Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            ws_endpoint: ws_endpoint.trim_end_matches('/').to_string(),
            model: config.model.clone(),
            language: config.language.clone().filter(|s| !s.trim().is_empty()),
            api_key,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            streaming: config.streaming,
            interim_results: config.interim_results,
            diarization: config.diarization,
            keywords: config
                .keywords
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        })
    }

    fn encode_wav(&self, samples: &[f32]) -> Result<Vec<u8>, TranscribeError> {
        // Docs: WAV mono 16-bit at 16k or 24k. Voxtype's pipeline is 16k, so
        // write an honest 16k header — no resampling needed.
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut buffer, spec).map_err(|e| {
            TranscribeError::AudioFormat(format!("Failed to create WAV writer: {}", e))
        })?;
        for &s in samples {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(v).map_err(|e| {
                TranscribeError::AudioFormat(format!("Failed to write sample: {}", e))
            })?;
        }
        writer
            .finalize()
            .map_err(|e| TranscribeError::AudioFormat(format!("Failed to finalize WAV: {}", e)))?;
        Ok(buffer.into_inner())
    }

    fn build_multipart_body(&self, wav_data: &[u8]) -> (String, Vec<u8>) {
        // Docs: multipart with `request` (JSON) and `audio` (WAV)
        // `request` = {"mode":"PUSH_TO_TALK","model":"...","audioEncoding":"WAV", ...}
        let boundary = format!(
            "----VoxtypeBoundary{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let mode = if self.diarization {
            "DIARIZATION"
        } else {
            "PUSH_TO_TALK"
        };
        let mut request_obj = serde_json::json!({
            "model": self.model,
            "mode": mode,
            "audioEncoding": "WAV",
        });
        if let Some(ref lang) = self.language {
            // Docs use languageBias: ["English", "French"] — map single hint there.
            request_obj["languageBias"] = serde_json::json!([lang]);
        }
        if !self.keywords.is_empty() {
            request_obj["keywords"] = serde_json::json!(self.keywords);
        }

        let request_json = request_obj.to_string();
        let mut body = Vec::new();
        // request part
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"request\"\r\n");
        body.extend_from_slice(b"Content-Type: application/json\r\n\r\n");
        body.extend_from_slice(request_json.as_bytes());
        body.extend_from_slice(b"\r\n");
        // audio part
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"audio\"; filename=\"audio.wav\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: audio/wav\r\n\r\n");
        body.extend_from_slice(wav_data);
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
        (boundary, body)
    }

    fn init_frame(&self) -> String {
        // Docs example:
        // {
        //   "authorization": {"accessToken": "Bearer <key>"},
        //   "audioEncoding": "PCM_16KHZ",
        //   "model": "muse-voice-transcribe-1.0",
        //   "mode": "PUSH_TO_TALK",
        //   "partialMode": "CUMULATIVE",
        //   "emitAudioProgress": false,
        // }
        let mode = if self.diarization {
            "DIARIZATION"
        } else {
            "PUSH_TO_TALK"
        };
        let mut obj = serde_json::json!({
            "authorization": {"accessToken": format!("Bearer {}", self.api_key)},
            "audioEncoding": AUDIO_ENCODING,
            "model": self.model,
            "mode": mode,
            "partialMode": "CUMULATIVE",
            "emitAudioProgress": false,
        });
        if let Some(lang) = &self.language {
            obj["languageBias"] = serde_json::json!([lang]);
        }
        if !self.keywords.is_empty() {
            obj["keywords"] = serde_json::json!(self.keywords);
        }
        obj.to_string()
    }
}

impl Transcriber for MuseTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".into()));
        }
        let duration = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!(
            "Muse: sending {:.2}s of audio to {} (model {})",
            duration,
            self.endpoint,
            self.model
        );
        let start = std::time::Instant::now();
        let wav = self.encode_wav(samples)?;
        let (boundary, body) = self.build_multipart_body(&wav);
        // Docs: POST https://api.meta.ai/v1/asr/transcribe?sessionId=...
        let url = format!("{}/asr/transcribe", self.endpoint);
        let req = ureq::post(&url)
            .timeout(self.timeout)
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={}", boundary),
            )
            .set("Authorization", &format!("Bearer {}", self.api_key));
        let resp = req.send_bytes(&body).map_err(|e| match e {
            ureq::Error::Status(code, r) => {
                let b = r.into_string().unwrap_or_default();
                TranscribeError::RemoteError(format!("Muse API {}: {}", code, b))
            }
            ureq::Error::Transport(t) => {
                TranscribeError::NetworkError(format!("Muse request failed: {}", t))
            }
        })?;
        let json: serde_json::Value = resp.into_json().map_err(|e| {
            TranscribeError::RemoteError(format!("Failed to parse Muse response: {}", e))
        })?;
        // Docs response: { sessionId, transcript, audioDurationMs, turns: [...] }
        // Fallback to `text` for older OpenAI-compat shape.
        let text = json
            .get("transcript")
            .and_then(|v| v.as_str())
            .or_else(|| json.get("text").and_then(|v| v.as_str()))
            .ok_or_else(|| {
                TranscribeError::RemoteError(format!(
                    "Muse response missing 'transcript': {}",
                    json
                ))
            })?
            .trim()
            .to_string();
        tracing::info!(
            "Muse transcription completed in {:.2}s: {:?}",
            start.elapsed().as_secs_f32(),
            if text.chars().count() > 80 {
                format!("{}...", text.chars().take(80).collect::<String>())
            } else {
                text.clone()
            }
        );
        Ok(text)
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        if self.streaming {
            Some(self)
        } else {
            None
        }
    }
}

// --- Streaming (native WebSocket) ---

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
fn redact_api_key(frame: &str) -> String {
    // Redact both `api_key` and `authorization.accessToken`
    match serde_json::from_str::<serde_json::Value>(frame) {
        Ok(mut v) => {
            if let Some(o) = v.as_object_mut() {
                if o.contains_key("api_key") {
                    o.insert("api_key".into(), serde_json::Value::String("***".into()));
                }
                if let Some(auth) = o.get_mut("authorization").and_then(|a| a.as_object_mut()) {
                    if auth.contains_key("accessToken") {
                        auth.insert(
                            "accessToken".into(),
                            serde_json::Value::String("Bearer ***".into()),
                        );
                    }
                }
            }
            v.to_string()
        }
        Err(_) => frame.to_string(),
    }
}
fn is_special_token(text: &str) -> bool {
    let t = text.trim();
    t.starts_with('<') && t.ends_with('>') && t.len() > 2
}

/// Length of the longest common prefix in Unicode scalars, so a revision
/// backspaces only the divergent tail instead of the whole segment.
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

// Server messages per docs: type = speechStart | transcript | speechEnd | speechComplete | speaker | error
#[derive(Debug, serde::Deserialize, Default)]
#[allow(non_snake_case, dead_code)]
struct MuseServerMessage {
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    transcript: Option<String>,
    #[serde(default)]
    text: Option<String>, // fallback for older shape
    #[serde(default)]
    speaker: Option<String>,
    #[serde(default, alias = "label")]
    label: Option<String>,
    #[serde(default)]
    turnId: Option<u64>,
    #[serde(default, alias = "is_final")]
    r#final: Option<bool>,
    #[serde(default)]
    audioProcessedMs: Option<u64>,
    #[serde(default)]
    sessionId: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
}

impl MuseServerMessage {
    fn effective_text(&self) -> Option<String> {
        let t = self.transcript.clone().or_else(|| self.text.clone())?;
        if t.trim().is_empty() || is_special_token(&t) {
            return None;
        }
        Some(t)
    }
    #[allow(dead_code)]
    fn is_finished(&self) -> bool {
        matches!(self.r#type.as_deref(), Some("speechComplete"))
    }
    fn is_error(&self) -> Option<String> {
        if matches!(self.r#type.as_deref(), Some("error")) {
            return Some(
                self.message
                    .clone()
                    .or_else(|| self.error.clone())
                    .or_else(|| self.error_message.clone())
                    .unwrap_or_else(|| "unknown error".into()),
            );
        }
        self.error.clone().or_else(|| self.error_message.clone())
    }
    fn is_final_flag(&self) -> bool {
        // Docs: `transcript` with `final:true` is interim final, `speechComplete` is the per-turn final
        if self.r#type.as_deref() == Some("speechComplete") {
            return true;
        }
        self.r#final.unwrap_or(false)
    }
}

struct MuseReconciler {
    typed_partial: String,
    interim_results: bool,
}
impl MuseReconciler {
    fn new(interim_results: bool) -> Self {
        Self {
            typed_partial: String::new(),
            interim_results,
        }
    }
    fn process(&mut self, msg: &MuseServerMessage) -> Option<StreamingEvent> {
        if let Some(err) = msg.is_error() {
            return Some(StreamingEvent::Error(TranscribeError::InferenceFailed(
                format!("Muse server error: {}", err),
            )));
        }
        // Ignore non-transcript events except speechComplete which carries final
        let is_transcript = matches!(msg.r#type.as_deref(), Some("transcript"));
        let is_complete = matches!(msg.r#type.as_deref(), Some("speechComplete"));
        if !is_transcript && !is_complete {
            return None;
        }
        let text = msg.effective_text()?;
        if is_complete || msg.is_final_flag() {
            // speechComplete is always final for the turn
            let ev = if text.starts_with(&self.typed_partial) {
                let delta = text[self.typed_partial.len()..].to_string();
                self.typed_partial.clear();
                if delta.is_empty() {
                    // Final confirmed what we already typed as partial — emit as Final delta
                    // But docs say speechComplete may differ (punct/casing), so if empty we skip
                    return None;
                }
                StreamingEvent::Final {
                    text: delta,
                    segment_id: msg.turnId.unwrap_or(0),
                }
            } else {
                // Tail revision: the final differs from what partials typed.
                // Emit Replace (not Final) so the cursor text is corrected
                // instead of duplicated. Common-prefix keeps the churn small:
                // "hello world" -> "hello world." becomes backspace 0 + ".".
                let common = common_prefix_len(&self.typed_partial, &text);
                let backspace = self.typed_partial.chars().count() - common;
                let revised: String = text.chars().skip(common).collect();
                self.typed_partial.clear();
                StreamingEvent::Replace {
                    backspace,
                    text: revised,
                    segment_id: msg.turnId.unwrap_or(0),
                }
            };
            Some(ev)
        } else {
            if !self.interim_results || text.is_empty() {
                return None;
            }
            if text.starts_with(&self.typed_partial) {
                let delta = text[self.typed_partial.len()..].to_string();
                if delta.is_empty() {
                    return None;
                }
                self.typed_partial = text;
                Some(StreamingEvent::Partial {
                    text: delta,
                    segment_id: msg.turnId.unwrap_or(0),
                })
            } else {
                None
            }
        }
    }
}

impl StreamingTranscriber for MuseTranscriber {
    fn start_stream(
        &self,
        mut samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        // Per docs, WS URL is wss://api.meta.ai/v1/asr/realtime?sessionId=... (optional)
        let mut ws_url = self.ws_endpoint.clone();
        if !ws_url.contains("sessionId=") {
            let sid = format!("voxtype-{}", uuid::Uuid::new_v4());
            if ws_url.contains('?') {
                ws_url = format!("{}&sessionId={}", ws_url, sid);
            } else {
                ws_url = format!("{}?sessionId={}", ws_url, sid);
            }
        }
        let init_frame = self.init_frame();
        let interim_results = self.interim_results;

        let (events_tx, events_rx) = mpsc::channel::<StreamingEvent>(64);
        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();

        tracing::info!(
            "Muse streaming: connecting to {} (model {})",
            redact_api_key(&ws_url),
            self.model
        );
        tracing::debug!("Muse streaming init frame: {}", redact_api_key(&init_frame));

        let task = tokio::spawn(async move {
            let (ws_stream, _) =
                match tokio::time::timeout(WS_TIMEOUT, tokio_tungstenite::connect_async(&ws_url))
                    .await
                {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => {
                        let _ = events_tx
                            .send(StreamingEvent::Error(TranscribeError::NetworkError(
                                format!("Muse WS connect failed: {}", e),
                            )))
                            .await;
                        let _ = events_tx.send(StreamingEvent::Ended).await;
                        return Ok(());
                    }
                    Err(_) => {
                        let _ = events_tx
                            .send(StreamingEvent::Error(TranscribeError::NetworkError(
                                "Muse WS connect timeout".into(),
                            )))
                            .await;
                        let _ = events_tx.send(StreamingEvent::Ended).await;
                        return Ok(());
                    }
                };

            let (mut ws_sink, mut ws_stream) = ws_stream.split();

            // Send handshake as first text frame
            if let Err(e) = ws_sink.send(Message::Text(init_frame)).await {
                let _ = events_tx
                    .send(StreamingEvent::Error(TranscribeError::NetworkError(
                        format!("Muse WS handshake send failed: {}", e),
                    )))
                    .await;
                let _ = events_tx.send(StreamingEvent::Ended).await;
                return Ok(());
            }

            // Wait for handshake ack (must contain sessionId)
            let handshake_ack = match tokio::time::timeout(WS_TIMEOUT, ws_stream.next()).await {
                Ok(Some(Ok(Message::Text(t)))) => t,
                Ok(Some(Ok(Message::Binary(_)))) => {
                    let _ = events_tx
                        .send(StreamingEvent::Error(TranscribeError::NetworkError(
                            "Muse handshake: unexpected binary".into(),
                        )))
                        .await;
                    let _ = events_tx.send(StreamingEvent::Ended).await;
                    return Ok(());
                }
                Ok(Some(Ok(_))) | Ok(None) => {
                    let _ = events_tx
                        .send(StreamingEvent::Error(TranscribeError::NetworkError(
                            "Muse handshake: no ack".into(),
                        )))
                        .await;
                    let _ = events_tx.send(StreamingEvent::Ended).await;
                    return Ok(());
                }
                Ok(Some(Err(e))) => {
                    let _ = events_tx
                        .send(StreamingEvent::Error(TranscribeError::NetworkError(
                            format!("Muse handshake recv failed: {}", e),
                        )))
                        .await;
                    let _ = events_tx.send(StreamingEvent::Ended).await;
                    return Ok(());
                }
                Err(_) => {
                    let _ = events_tx
                        .send(StreamingEvent::Error(TranscribeError::NetworkError(
                            "Muse handshake timeout".into(),
                        )))
                        .await;
                    let _ = events_tx.send(StreamingEvent::Ended).await;
                    return Ok(());
                }
            };
            tracing::debug!("Muse handshake ack: {}", handshake_ack);
            // If server sent error instead of sessionId
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&handshake_ack) {
                if v.get("sessionId").is_none()
                    && v.get("type").and_then(|t| t.as_str()) == Some("error")
                {
                    let msg = v
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("handshake failed");
                    let _ = events_tx
                        .send(StreamingEvent::Error(TranscribeError::InferenceFailed(
                            format!("Muse handshake error: {}", msg),
                        )))
                        .await;
                    let _ = events_tx.send(StreamingEvent::Ended).await;
                    return Ok(());
                }
            }

            let mut reconciler = MuseReconciler::new(interim_results);
            let mut eof_sent = false;

            loop {
                tokio::select! {
                    biased;

                    _ = &mut cancel_rx => {
                        tracing::debug!("Muse streaming cancelled");
                        let _ = ws_sink.close().await;
                        break;
                    }

                    chunk = async {
                        if eof_sent {
                            // Already sent endStream — park this branch forever
                            std::future::pending::<Option<Vec<f32>>>().await
                        } else {
                            samples_rx.recv().await
                        }
                    } => {
                        match chunk {
                            Some(samples) if !samples.is_empty() => {
                                // Docs: PCM_16KHZ mono int16 LE, real-time paced.
                                let bytes = f32_to_s16le_bytes(&samples);
                                if let Err(e) = ws_sink.send(Message::Binary(bytes)).await {
                                    let _ = events_tx.send(StreamingEvent::Error(TranscribeError::NetworkError(format!("Muse WS send failed: {}", e)))).await;
                                    break;
                                }
                            }
                            Some(_) => {},
                            None => {
                                eof_sent = true;
                                tracing::debug!("Muse streaming: sending endStream");
                                let _ = ws_sink.send(Message::Text(r#"{"type":"endStream"}"#.into())).await;
                                // Don't break — drain server events until close
                            }
                        }
                    }

                    msg = ws_stream.next() => {
                        match msg {
                            Some(Ok(Message::Text(txt))) => {
                                tracing::trace!("Muse WS recv: {}", txt);
                                let parsed: Result<MuseServerMessage, _> = serde_json::from_str(&txt);
                                let m = match parsed {
                                    Ok(v) => v,
                                    Err(_) => {
                                        tracing::trace!("Muse WS non-JSON text: {}", txt);
                                        continue;
                                    }
                                };
                                if let Some(err) = m.is_error() {
                                    let _ = events_tx.send(StreamingEvent::Error(TranscribeError::InferenceFailed(err))).await;
                                    let _ = events_tx.send(StreamingEvent::Ended).await;
                                    return Ok(());
                                }
                                if let Some(ev) = reconciler.process(&m) {
                                    let is_err = matches!(ev, StreamingEvent::Error(_));
                                    let _ = events_tx.send(ev).await;
                                    if is_err {
                                        let _ = events_tx.send(StreamingEvent::Ended).await;
                                        return Ok(());
                                    }
                                }
                                if m.r#type.as_deref() == Some("speechComplete") {
                                    // keep reading for more turns until server closes after endStream
                                }
                            }
                            Some(Ok(Message::Binary(bin))) => {
                                if let Ok(txt) = String::from_utf8(bin) {
                                    if let Ok(m) = serde_json::from_str::<MuseServerMessage>(&txt) {
                                        if let Some(ev) = reconciler.process(&m) {
                                            let _ = events_tx.send(ev).await;
                                        }
                                    }
                                }
                            }
                            Some(Ok(Message::Close(_))) | None => {
                                break;
                            }
                            Some(Ok(_)) => {},
                            Some(Err(e)) => {
                                let _ = events_tx.send(StreamingEvent::Error(TranscribeError::NetworkError(format!("Muse WS recv error: {}", e)))).await;
                                break;
                            }
                        }
                    }
                }
            }

            let _ = ws_sink.close().await;
            let _ = events_tx.send(StreamingEvent::Ended).await;
            Ok::<(), TranscribeError>(())
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

    fn cfg_with_key() -> MuseConfig {
        MuseConfig {
            api_key: Some("sk_test".into()),
            ..Default::default()
        }
    }

    #[test]
    fn new_requires_api_key() {
        let cfg = MuseConfig {
            api_key: None,
            ..Default::default()
        };
        unsafe {
            std::env::remove_var("VOXTYPE_MUSE_API_KEY");
            std::env::remove_var("MUSE_API_KEY");
            std::env::remove_var("META_API_KEY");
            std::env::remove_var("META_MODEL_API_KEY");
        }
        let r = MuseTranscriber::new(&cfg);
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("API key"));
    }

    #[test]
    fn encode_wav_produces_header() {
        let t = MuseTranscriber::new(&cfg_with_key()).unwrap();
        let samples = vec![0.0f32; 1600];
        let wav = t.encode_wav(&samples).unwrap();
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn multipart_contains_request_and_audio() {
        let t = MuseTranscriber::new(&cfg_with_key()).unwrap();
        let (boundary, body) = t.build_multipart_body(&[0u8; 10]);
        let s = String::from_utf8_lossy(&body);
        assert!(s.contains(&boundary));
        assert!(s.contains("name=\"request\""));
        assert!(s.contains("name=\"audio\""));
        assert!(s.contains("muse-voice-transcribe-1.0"));
    }

    #[test]
    fn ws_endpoint_defaults_to_asr_realtime() {
        let cfg = MuseConfig {
            api_key: Some("k".into()),
            ..Default::default()
        };
        assert_eq!(
            cfg.effective_ws_endpoint(),
            "wss://api.meta.ai/v1/asr/realtime"
        );
        let cfg2 = MuseConfig {
            api_key: Some("k".into()),
            endpoint: Some("https://example.com/api".into()),
            ..Default::default()
        };
        assert_eq!(
            cfg2.effective_ws_endpoint(),
            "wss://example.com/api/asr/realtime"
        );
        let cfg3 = MuseConfig {
            api_key: Some("k".into()),
            ws_endpoint: Some("wss://custom/ws".into()),
            ..Default::default()
        };
        assert_eq!(cfg3.effective_ws_endpoint(), "wss://custom/ws");
    }

    #[test]
    fn reconciler_emits_partial_then_final_via_speech_complete() {
        let mut r = MuseReconciler::new(true);
        let ev = r.process(&MuseServerMessage {
            r#type: Some("transcript".into()),
            transcript: Some("hello".into()),
            r#final: Some(false),
            ..Default::default()
        });
        assert!(matches!(ev, Some(StreamingEvent::Partial { text, .. }) if text == "hello"));
        let ev2 = r.process(&MuseServerMessage {
            r#type: Some("transcript".into()),
            transcript: Some("hello world".into()),
            r#final: Some(false),
            ..Default::default()
        });
        assert!(matches!(ev2, Some(StreamingEvent::Partial { text, .. }) if text == " world"));
        let ev3 = r.process(&MuseServerMessage {
            r#type: Some("speechComplete".into()),
            transcript: Some("hello world.".into()),
            ..Default::default()
        });
        assert!(matches!(ev3, Some(StreamingEvent::Final { text, .. }) if text == "."));
    }

    #[test]
    fn reconciler_emits_replace_on_tail_revision() {
        let mut r = MuseReconciler::new(true);
        // Partials typed "hello world" at the cursor...
        for t in ["hello", "hello world"] {
            let _ = r.process(&MuseServerMessage {
                r#type: Some("transcript".into()),
                transcript: Some(t.into()),
                r#final: Some(false),
                ..Default::default()
            });
        }
        // ...then the final differs (casing + punctuation).
        let ev = r.process(&MuseServerMessage {
            r#type: Some("speechComplete".into()),
            transcript: Some("Hello world.".into()),
            ..Default::default()
        });
        // Common prefix is empty ("h" vs "H"), so the whole typed tail
        // is backspaced and the corrected text committed — no duplication.
        assert!(
            matches!(ev, Some(StreamingEvent::Replace { backspace: 11, text, .. }) if text == "Hello world.")
        );
        // A pure append ("hello world" -> "hello world.") stays a Final
        // delta — Replace with backspace 0 would be equivalent per the
        // StreamingEvent docs, but the reconciler only reaches for Replace
        // when the tail actually diverges.
        let mut r2 = MuseReconciler::new(true);
        let _ = r2.process(&MuseServerMessage {
            r#type: Some("transcript".into()),
            transcript: Some("hello world".into()),
            r#final: Some(false),
            ..Default::default()
        });
        let ev2 = r2.process(&MuseServerMessage {
            r#type: Some("speechComplete".into()),
            transcript: Some("hello world.".into()),
            ..Default::default()
        });
        assert!(matches!(ev2, Some(StreamingEvent::Final { text, .. }) if text == "."));
    }

    #[test]
    fn polish_fields_deserialize() {
        let cfg: MuseConfig = toml::from_str(
            r#"
            polish_command = "muse exec 'clean this: $(cat)'"
            polish_timeout_ms = 5000
        "#,
        )
        .unwrap();
        assert_eq!(
            cfg.polish_command.as_deref(),
            Some("muse exec 'clean this: $(cat)'")
        );
        assert_eq!(cfg.polish_timeout_ms, 5000);
        assert_eq!(MuseConfig::default().polish_timeout_ms, 20000);
        assert!(MuseConfig::default().polish_command.is_none());
    }

    #[test]
    fn init_frame_contains_auth_and_model() {
        let t = MuseTranscriber::new(&cfg_with_key()).unwrap();
        let f = t.init_frame();
        assert!(f.contains("accessToken"));
        assert!(f.contains("Bearer sk_test"));
        assert!(f.contains("PCM_16KHZ"));
        assert!(f.contains("muse-voice-transcribe-1.0"));
    }

    #[test]
    fn init_frame_maps_keywords_array() {
        let mut cfg = cfg_with_key();
        cfg.keywords = Some(vec!["Omarchy".into(), " Hyprland ".into(), "".into()]);
        let t = MuseTranscriber::new(&cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&t.init_frame()).unwrap();
        assert_eq!(v["keywords"], serde_json::json!(["Omarchy", "Hyprland"]));
        // Unset or all-blank means no keywords key at all.
        let t2 = MuseTranscriber::new(&cfg_with_key()).unwrap();
        let v2: serde_json::Value = serde_json::from_str(&t2.init_frame()).unwrap();
        assert!(v2.get("keywords").is_none());
    }
}
