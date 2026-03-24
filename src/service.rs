//! Local OpenAI-compatible HTTP transcription service.
//!
//! Runs in-process with the daemon and exposes:
//! - `GET /healthz`
//! - `POST /v1/audio/transcriptions`
//! - `POST /v1/audio/translations` (alias to transcriptions)

use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use std::collections::BTreeSet;
use std::io::Cursor;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

use crate::config::{Config, LanguageConfig, ServiceConfig, TranscriptionEngine};
use crate::error::{TranscribeError, VoxtypeError};
use crate::meeting::VoiceActivityDetector;
use crate::transcribe::{Transcriber, TranscriptionResult, TranscriptionSegment};

const SERVICE_SAMPLE_RATE: usize = 16_000;
const LONG_FORM_CHUNK_SECS: usize = 30;
const LONG_FORM_CHUNK_THRESHOLD_SECS: usize = 90;
const LONG_FORM_VAD_THRESHOLD: f32 = 0.01;

#[derive(Clone)]
struct AppState {
    transcriber: Arc<dyn Transcriber>,
    request_timeout: Duration,
    allowed_languages: Arc<Vec<String>>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct TranscriptionResponse {
    text: String,
}

#[derive(Serialize)]
struct VerboseTranscriptionResponse {
    text: String,
    language: String,
    duration: f64,
    segments: Vec<VerboseSegment>,
}

#[derive(Serialize)]
struct VerboseSegment {
    id: usize,
    start: f64,
    end: f64,
    text: String,
}

#[derive(Serialize)]
struct ApiErrorResponse {
    error: ApiErrorBody,
}

#[derive(Serialize)]
struct ApiErrorBody {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
    error_type: &'static str,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            error_type: "invalid_request_error",
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
            error_type: "server_error",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorResponse {
            error: ApiErrorBody {
                message: self.message,
                error_type: self.error_type.to_string(),
            },
        };
        (self.status, Json(body)).into_response()
    }
}

/// Running local service handle.
pub struct ServiceHandle {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl ServiceHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Err(e) = self.task.await {
            tracing::warn!("Service task join error: {}", e);
        }
    }
}

/// Start local OpenAI-compatible STT service sharing an existing transcriber.
///
/// When `shared` is provided the service reuses it instead of loading a
/// second copy of the model into VRAM.  Falls back to creating its own
/// transcriber when `shared` is `None`.
pub async fn start(
    config: &Config,
    config_path: Option<PathBuf>,
    shared: Option<Arc<dyn Transcriber>>,
) -> Result<ServiceHandle, VoxtypeError> {
    let service_cfg = config.service.clone();

    let transcriber = if let Some(t) = shared {
        tracing::info!("Service reusing daemon transcriber (shared VRAM)");
        t
    } else {
        let mut transcriber_config = config.clone();
        transcriber_config.whisper.language =
            default_language_for_service(&service_cfg, &config.whisper.language);

        tokio::task::spawn_blocking(move || {
            match transcriber_config.engine {
                TranscriptionEngine::Whisper => {
                    crate::transcribe::create_transcriber_with_config_path(
                        &transcriber_config.whisper,
                        config_path,
                    )
                    .map(Arc::from)
                }
                _ => crate::transcribe::create_transcriber(&transcriber_config).map(Arc::from),
            }
        })
        .await
        .map_err(|e| {
            VoxtypeError::Config(format!(
                "Service transcriber initialization task failed: {}",
                e
            ))
        })??
    };

    start_with_transcriber(service_cfg, transcriber).await
}

fn default_language_for_service(
    service_cfg: &ServiceConfig,
    fallback: &LanguageConfig,
) -> LanguageConfig {
    let normalized = normalize_languages(&service_cfg.allowed_languages);
    if normalized.is_empty() {
        fallback.clone()
    } else if normalized.len() == 1 {
        LanguageConfig::Single(normalized[0].clone())
    } else {
        LanguageConfig::Multiple(normalized)
    }
}

async fn start_with_transcriber(
    service_cfg: ServiceConfig,
    transcriber: Arc<dyn Transcriber>,
) -> Result<ServiceHandle, VoxtypeError> {
    let bind_addr = format!("{}:{}", service_cfg.host, service_cfg.port);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|e| {
            VoxtypeError::Config(format!(
                "Failed to bind service listener on {}: {}",
                bind_addr, e
            ))
        })?;
    let local_addr = listener.local_addr().map_err(|e| {
        VoxtypeError::Config(format!("Failed to read service local address: {}", e))
    })?;

    let state = AppState {
        transcriber,
        request_timeout: Duration::from_millis(service_cfg.request_timeout_ms),
        allowed_languages: Arc::new(normalize_languages(&service_cfg.allowed_languages)),
    };

    let app = build_router(state, service_cfg.max_upload_bytes);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app.into_make_service())
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
        {
            tracing::error!("Service HTTP server failed: {}", e);
        }
    });

    Ok(ServiceHandle {
        addr: local_addr,
        shutdown_tx: Some(shutdown_tx),
        task,
    })
}

fn build_router(state: AppState, max_upload_bytes: usize) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/audio/transcriptions", post(transcribe_handler))
        .route("/v1/audio/translations", post(transcribe_handler))
        .layer(DefaultBodyLimit::max(max_upload_bytes))
        .with_state(state)
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn transcribe_handler(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    let mut audio_data: Option<Vec<u8>> = None;
    let mut language: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut response_format = "json".to_string();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("Invalid multipart request: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                let bytes = field.bytes().await.map_err(|e| {
                    ApiError::bad_request(format!("Failed to read file field: {}", e))
                })?;
                audio_data = Some(bytes.to_vec());
            }
            "language" => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| ApiError::bad_request(format!("Invalid language field: {}", e)))?;
                language = Some(value);
            }
            "prompt" => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| ApiError::bad_request(format!("Invalid prompt field: {}", e)))?;
                prompt = Some(value);
            }
            "response_format" => {
                response_format = field.text().await.map_err(|e| {
                    ApiError::bad_request(format!("Invalid response_format field: {}", e))
                })?;
            }
            _ => {
                // Ignore non-essential fields (model, temperature, etc.).
            }
        }
    }

    let audio_data = audio_data
        .ok_or_else(|| ApiError::bad_request("Missing required multipart field: file"))?;

    let samples = decode_wav_to_mono_16k(&audio_data).map_err(ApiError::bad_request)?;
    if samples.is_empty() {
        return Err(ApiError::bad_request("Audio payload contains no samples"));
    }

    let language_override =
        normalize_language_override(language.as_deref(), &state.allowed_languages)?;
    let prompt_override = prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let format = response_format.trim().to_lowercase();
    if format != "json" && format != "verbose_json" && format != "text" {
        return Err(ApiError::bad_request(format!(
            "Unsupported response_format '{}'; expected json, verbose_json, or text",
            response_format
        )));
    }

    let use_segments = format == "verbose_json";
    let transcriber = state.transcriber.clone();
    let timeout = state.request_timeout;

    if use_segments {
        let mut task = tokio::task::spawn_blocking(move || {
            transcribe_segments_adaptive(
                transcriber,
                &samples,
                language_override.as_deref(),
                prompt_override.as_deref(),
            )
        });

        let result = tokio::select! {
            join = &mut task => join,
            _ = tokio::time::sleep(timeout) => {
                task.abort();
                return Err(ApiError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!("Transcription timed out after {}ms", timeout.as_millis()),
                    error_type: "timeout_error",
                });
            }
        };

        let tr = match result {
            Ok(Ok(tr)) => tr,
            Ok(Err(e)) => return Err(map_transcription_error(e)),
            Err(e) => return Err(ApiError::internal(format!("Transcription task failed: {}", e))),
        };

        let verbose = VerboseTranscriptionResponse {
            text: tr.text,
            language: tr.language,
            duration: tr.duration,
            segments: tr
                .segments
                .into_iter()
                .enumerate()
                .map(|(id, seg)| VerboseSegment {
                    id,
                    start: seg.start,
                    end: seg.end,
                    text: seg.text,
                })
                .collect(),
        };

        Ok(Json(verbose).into_response())
    } else {
        let mut task = tokio::task::spawn_blocking(move || {
            transcribe_text_adaptive(
                transcriber,
                &samples,
                language_override.as_deref(),
                prompt_override.as_deref(),
            )
        });

        let result = tokio::select! {
            join = &mut task => join,
            _ = tokio::time::sleep(timeout) => {
                task.abort();
                return Err(ApiError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!("Transcription timed out after {}ms", timeout.as_millis()),
                    error_type: "timeout_error",
                });
            }
        };

        let text = match result {
            Ok(Ok(text)) => text,
            Ok(Err(e)) => return Err(map_transcription_error(e)),
            Err(e) => return Err(ApiError::internal(format!("Transcription task failed: {}", e))),
        };

        if format == "text" {
            let mut response = text.into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            );
            return Ok(response);
        }

        Ok(Json(TranscriptionResponse { text }).into_response())
    }
}

fn transcribe_text_adaptive(
    transcriber: Arc<dyn Transcriber>,
    samples: &[f32],
    language_override: Option<&str>,
    prompt_override: Option<&str>,
) -> Result<String, TranscribeError> {
    if should_chunk_long_form(samples) {
        return Ok(
            transcribe_segments_adaptive(transcriber, samples, language_override, prompt_override)?
                .text,
        );
    }

    transcriber.transcribe_with_options(samples, language_override, prompt_override)
}

fn transcribe_segments_adaptive(
    transcriber: Arc<dyn Transcriber>,
    samples: &[f32],
    language_override: Option<&str>,
    prompt_override: Option<&str>,
) -> Result<TranscriptionResult, TranscribeError> {
    if !should_chunk_long_form(samples) {
        return transcriber.transcribe_segments(samples, language_override, prompt_override);
    }

    let vad = VoiceActivityDetector::new(LONG_FORM_VAD_THRESHOLD, SERVICE_SAMPLE_RATE as u32);
    let chunk_len = LONG_FORM_CHUNK_SECS * SERVICE_SAMPLE_RATE;
    let total_duration = samples.len() as f64 / SERVICE_SAMPLE_RATE as f64;
    let mut combined_text = String::new();
    let mut combined_segments = Vec::new();
    let mut detected_languages = BTreeSet::new();

    tracing::info!(
        "Long-form service request ({:.2}s) chunked into {}s windows",
        total_duration,
        LONG_FORM_CHUNK_SECS
    );

    for (chunk_index, chunk_samples) in samples.chunks(chunk_len).enumerate() {
        if !vad.contains_speech(chunk_samples) {
            tracing::debug!(
                "Skipping silent long-form chunk {} ({:.2}s)",
                chunk_index,
                chunk_samples.len() as f64 / SERVICE_SAMPLE_RATE as f64
            );
            continue;
        }

        let chunk_result =
            transcriber.transcribe_segments(chunk_samples, language_override, prompt_override)?;

        let detected_language = chunk_result.language.trim().to_lowercase();
        if !detected_language.is_empty() && detected_language != "auto" {
            detected_languages.insert(detected_language);
        }

        push_text_piece(&mut combined_text, &chunk_result.text);

        let chunk_offset = (chunk_index * chunk_len) as f64 / SERVICE_SAMPLE_RATE as f64;
        let chunk_duration = chunk_samples.len() as f64 / SERVICE_SAMPLE_RATE as f64;

        if chunk_result.segments.is_empty() {
            let text = chunk_result.text.trim();
            if !text.is_empty() {
                combined_segments.push(TranscriptionSegment {
                    start: chunk_offset,
                    end: chunk_offset + chunk_duration,
                    text: text.to_string(),
                });
            }
            continue;
        }

        for segment in chunk_result.segments {
            let text = segment.text.trim();
            if text.is_empty() {
                continue;
            }
            combined_segments.push(TranscriptionSegment {
                start: chunk_offset + segment.start,
                end: chunk_offset + segment.end,
                text: text.to_string(),
            });
        }
    }

    if combined_segments.is_empty() {
        tracing::warn!(
            "Long-form chunking found no speech chunks; falling back to single-pass transcription"
        );
        return transcriber.transcribe_segments(samples, language_override, prompt_override);
    }

    if combined_text.trim().is_empty() {
        combined_text = combined_segments
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }

    Ok(TranscriptionResult {
        text: combined_text.trim().to_string(),
        language: summarize_detected_languages(language_override, &detected_languages),
        duration: total_duration,
        segments: combined_segments,
    })
}

fn should_chunk_long_form(samples: &[f32]) -> bool {
    samples.len() > LONG_FORM_CHUNK_THRESHOLD_SECS * SERVICE_SAMPLE_RATE
}

fn summarize_detected_languages(
    language_override: Option<&str>,
    detected_languages: &BTreeSet<String>,
) -> String {
    if let Some(language) = language_override {
        let trimmed = language.trim();
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("auto") {
            return trimmed.to_lowercase();
        }
    }

    match detected_languages.len() {
        0 => language_override.unwrap_or("auto").trim().to_lowercase(),
        1 => detected_languages.iter().next().cloned().unwrap_or_else(|| "auto".to_string()),
        _ => "mixed".to_string(),
    }
}

fn push_text_piece(buffer: &mut String, piece: &str) {
    let trimmed = piece.trim();
    if trimmed.is_empty() {
        return;
    }

    if !buffer.is_empty() {
        buffer.push(' ');
    }
    buffer.push_str(trimmed);
}

fn map_transcription_error(err: TranscribeError) -> ApiError {
    match err {
        TranscribeError::AudioFormat(msg) | TranscribeError::ConfigError(msg) => {
            ApiError::bad_request(msg)
        }
        TranscribeError::ModelNotFound(msg)
        | TranscribeError::InitFailed(msg)
        | TranscribeError::InferenceFailed(msg)
        | TranscribeError::NetworkError(msg)
        | TranscribeError::RemoteError(msg) => ApiError {
            status: StatusCode::BAD_GATEWAY,
            message: msg,
            error_type: "upstream_error",
        },
    }
}

fn normalize_languages(languages: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for lang in languages {
        let normalized = lang.trim().to_lowercase();
        if !normalized.is_empty() && !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    out
}

fn normalize_language_override(
    language: Option<&str>,
    allowed_languages: &[String],
) -> Result<Option<String>, ApiError> {
    let Some(raw) = language else {
        return Ok(None);
    };

    let normalized = raw.trim().to_lowercase();
    if normalized.is_empty() {
        return Ok(None);
    }

    if normalized == "auto" {
        return Ok(Some(normalized));
    }

    if !allowed_languages.is_empty() && !allowed_languages.contains(&normalized) {
        return Err(ApiError::bad_request(format!(
            "Language '{}' is not allowed; allowed languages: {}",
            normalized,
            allowed_languages.join(", ")
        )));
    }

    Ok(Some(normalized))
}

fn decode_wav_to_mono_16k(wav_bytes: &[u8]) -> Result<Vec<f32>, String> {
    let cursor = Cursor::new(wav_bytes);
    let mut reader =
        hound::WavReader::new(cursor).map_err(|e| format!("Invalid WAV payload: {}", e))?;
    let spec = reader.spec();

    let channels = spec.channels as usize;
    if channels == 0 {
        return Err("WAV payload has zero channels".to_string());
    }
    let sample_rate = spec.sample_rate;

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.map(|v| v.clamp(-1.0, 1.0)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to decode float WAV samples: {}", e))?,
        hound::SampleFormat::Int => {
            if spec.bits_per_sample <= 8 {
                reader
                    .samples::<i8>()
                    .map(|s| s.map(|v| v as f32 / i8::MAX as f32))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("Failed to decode 8-bit WAV samples: {}", e))?
            } else if spec.bits_per_sample <= 16 {
                reader
                    .samples::<i16>()
                    .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("Failed to decode 16-bit WAV samples: {}", e))?
            } else {
                let max_val =
                    ((1_i64 << (spec.bits_per_sample.saturating_sub(1) as u32)) - 1) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / max_val))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| {
                        format!(
                            "Failed to decode {}-bit WAV samples: {}",
                            spec.bits_per_sample, e
                        )
                    })?
            }
        }
    };

    let mono = if channels == 1 {
        interleaved
    } else {
        let frame_count = interleaved.len() / channels;
        let mut downmixed = Vec::with_capacity(frame_count);
        for i in 0..frame_count {
            let mut sum = 0.0f32;
            for ch in 0..channels {
                sum += interleaved[i * channels + ch];
            }
            downmixed.push((sum / channels as f32).clamp(-1.0, 1.0));
        }
        downmixed
    };

    if sample_rate == 16000 {
        return Ok(mono);
    }

    Ok(resample_linear(&mono, sample_rate, 16000))
}

fn resample_linear(samples: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if samples.is_empty() || source_rate == target_rate {
        return samples.to_vec();
    }

    let ratio = target_rate as f64 / source_rate as f64;
    let output_len = (samples.len() as f64 * ratio).ceil() as usize;
    let mut out = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let source_pos = i as f64 / ratio;
        let idx = source_pos.floor() as usize;
        let frac = (source_pos - idx as f64) as f32;

        let value = if idx + 1 < samples.len() {
            samples[idx] * (1.0 - frac) + samples[idx + 1] * frac
        } else {
            samples.get(idx).copied().unwrap_or(0.0)
        };
        out.push(value.clamp(-1.0, 1.0));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{WhisperConfig, WhisperMode};
    use crate::transcribe::remote::RemoteTranscriber;
    use crate::transcribe::{TranscriptionResult, TranscriptionSegment};
    use std::sync::Mutex;

    struct MockTranscriber {
        text: String,
        calls: Mutex<Vec<(Option<String>, Option<String>)>>,
    }

    impl MockTranscriber {
        fn new(text: &str) -> Self {
            Self {
                text: text.to_string(),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl Transcriber for MockTranscriber {
        fn transcribe(&self, _samples: &[f32]) -> Result<String, TranscribeError> {
            Ok(self.text.clone())
        }

        fn transcribe_with_options(
            &self,
            _samples: &[f32],
            language_override: Option<&str>,
            prompt_override: Option<&str>,
        ) -> Result<String, TranscribeError> {
            self.calls.lock().unwrap().push((
                language_override.map(ToOwned::to_owned),
                prompt_override.map(ToOwned::to_owned),
            ));
            Ok(self.text.clone())
        }

        fn transcribe_segments(
            &self,
            samples: &[f32],
            language_override: Option<&str>,
            prompt_override: Option<&str>,
        ) -> Result<TranscriptionResult, TranscribeError> {
            self.calls.lock().unwrap().push((
                language_override.map(ToOwned::to_owned),
                prompt_override.map(ToOwned::to_owned),
            ));
            let duration = samples.len() as f64 / 16000.0;
            Ok(TranscriptionResult {
                text: self.text.clone(),
                language: language_override.unwrap_or("en").to_string(),
                duration,
                segments: vec![
                    TranscriptionSegment {
                        start: 0.0,
                        end: duration / 2.0,
                        text: "hello from".to_string(),
                    },
                    TranscriptionSegment {
                        start: duration / 2.0,
                        end: duration,
                        text: "local service".to_string(),
                    },
                ],
            })
        }
    }

    struct ChunkCountingTranscriber {
        calls: Mutex<Vec<usize>>,
    }

    impl ChunkCountingTranscriber {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl Transcriber for ChunkCountingTranscriber {
        fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
            self.transcribe_with_options(samples, None, None)
        }

        fn transcribe_with_options(
            &self,
            samples: &[f32],
            _language_override: Option<&str>,
            _prompt_override: Option<&str>,
        ) -> Result<String, TranscribeError> {
            let mut calls = self.calls.lock().unwrap();
            calls.push(samples.len());
            Ok(format!("chunk {}", calls.len()))
        }

        fn transcribe_segments(
            &self,
            samples: &[f32],
            _language_override: Option<&str>,
            _prompt_override: Option<&str>,
        ) -> Result<TranscriptionResult, TranscribeError> {
            let mut calls = self.calls.lock().unwrap();
            calls.push(samples.len());
            let call_index = calls.len();
            let duration = samples.len() as f64 / SERVICE_SAMPLE_RATE as f64;
            Ok(TranscriptionResult {
                text: format!("chunk {}", call_index),
                language: "de".to_string(),
                duration,
                segments: vec![TranscriptionSegment {
                    start: 0.0,
                    end: duration,
                    text: format!("segment {}", call_index),
                }],
            })
        }
    }

    struct LanguageTrackingTranscriber {
        languages_seen: Mutex<Vec<Option<String>>>,
        returned_languages: Vec<String>,
    }

    impl LanguageTrackingTranscriber {
        fn new(returned_languages: &[&str]) -> Self {
            Self {
                languages_seen: Mutex::new(Vec::new()),
                returned_languages: returned_languages.iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    impl Transcriber for LanguageTrackingTranscriber {
        fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
            self.transcribe_with_options(samples, None, None)
        }

        fn transcribe_with_options(
            &self,
            samples: &[f32],
            language_override: Option<&str>,
            prompt_override: Option<&str>,
        ) -> Result<String, TranscribeError> {
            Ok(self
                .transcribe_segments(samples, language_override, prompt_override)?
                .text)
        }

        fn transcribe_segments(
            &self,
            samples: &[f32],
            language_override: Option<&str>,
            _prompt_override: Option<&str>,
        ) -> Result<TranscriptionResult, TranscribeError> {
            let mut seen = self.languages_seen.lock().unwrap();
            seen.push(language_override.map(ToOwned::to_owned));
            let call_index = seen.len();
            let duration = samples.len() as f64 / SERVICE_SAMPLE_RATE as f64;
            let language = self
                .returned_languages
                .get(call_index - 1)
                .cloned()
                .unwrap_or_else(|| "auto".to_string());

            Ok(TranscriptionResult {
                text: format!("chunk {}", call_index),
                language,
                duration,
                segments: vec![TranscriptionSegment {
                    start: 0.0,
                    end: duration,
                    text: format!("segment {}", call_index),
                }],
            })
        }
    }

    async fn spawn_test_server(
        transcriber: Arc<dyn Transcriber>,
        allowed_languages: Vec<String>,
    ) -> ServiceHandle {
        let service_cfg = ServiceConfig {
            enabled: true,
            host: "127.0.0.1".to_string(),
            port: 0,
            max_upload_bytes: 2_000_000,
            request_timeout_ms: 5000,
            allowed_languages,
        };

        start_with_transcriber(service_cfg, transcriber).await.unwrap()
    }

    fn sine_samples(sample_rate: u32, duration_secs: f32, freq_hz: f32) -> Vec<f32> {
        let sample_count = (sample_rate as f32 * duration_secs) as usize;
        (0..sample_count)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * freq_hz * t).sin() * 0.2
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_client_can_call_local_service() {
        let mock = Arc::new(MockTranscriber::new("hello from local service"));
        let handle =
            spawn_test_server(mock.clone(), vec!["en".to_string(), "de".to_string()]).await;
        let endpoint = format!("http://{}", handle.addr());

        let cfg = WhisperConfig {
            mode: Some(WhisperMode::Remote),
            remote_endpoint: Some(endpoint),
            remote_model: Some("whisper-1".to_string()),
            language: LanguageConfig::Single("en".to_string()),
            ..Default::default()
        };

        let client = RemoteTranscriber::new(&cfg).unwrap();
        let samples = sine_samples(16000, 0.3, 440.0);
        let text = tokio::task::spawn_blocking(move || client.transcribe(&samples))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text, "hello from local service");

        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0.as_deref(), Some("en"));

        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn language_outside_allowed_set_is_rejected() {
        let mock = Arc::new(MockTranscriber::new("ignored"));
        let handle = spawn_test_server(mock, vec!["en".to_string(), "de".to_string()]).await;
        let endpoint = format!("http://{}", handle.addr());

        let cfg = WhisperConfig {
            mode: Some(WhisperMode::Remote),
            remote_endpoint: Some(endpoint),
            remote_model: Some("whisper-1".to_string()),
            language: LanguageConfig::Single("fr".to_string()),
            ..Default::default()
        };

        let client = RemoteTranscriber::new(&cfg).unwrap();
        let samples = sine_samples(16000, 0.2, 440.0);
        let err = tokio::task::spawn_blocking(move || client.transcribe(&samples))
            .await
            .unwrap()
            .unwrap_err()
            .to_string();
        assert!(err.contains("Language 'fr' is not allowed"));

        handle.shutdown().await;
    }

    fn make_wav_bytes(samples: &[f32]) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
            for &s in samples {
                writer.write_sample((s * i16::MAX as f32) as i16).unwrap();
            }
            writer.finalize().unwrap();
        }
        cursor.into_inner()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn verbose_json_returns_segments() {
        use std::io::{Read, Write};

        let mock = Arc::new(MockTranscriber::new("hello from local service"));
        let handle =
            spawn_test_server(mock.clone(), vec!["en".to_string(), "de".to_string()]).await;
        let addr = handle.addr();

        let samples = sine_samples(16000, 0.3, 440.0);
        let wav_bytes = make_wav_bytes(&samples);

        let boundary = "----TestBoundary1234";
        let mut body = Vec::new();

        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"response_format\"\r\n\r\n",
        );
        body.extend_from_slice(b"verbose_json\r\n");

        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: audio/wav\r\n\r\n");
        body.extend_from_slice(&wav_bytes);
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

        // Use raw TCP to avoid needing reqwest
        let request = format!(
            "POST /v1/audio/transcriptions HTTP/1.1\r\n\
             Host: {}\r\n\
             Content-Type: multipart/form-data; boundary={}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            addr, boundary, body.len()
        );

        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();

        // Parse HTTP response body (after blank line)
        let body_start = response.find("\r\n\r\n").unwrap() + 4;
        let response_body = &response[body_start..];

        // Handle chunked transfer encoding
        let json_str = if response.contains("transfer-encoding: chunked") {
            // Parse chunked body: size\r\ndata\r\n...0\r\n
            let mut decoded = String::new();
            let mut remaining = response_body;
            loop {
                let size_end = remaining.find("\r\n").unwrap_or(0);
                let chunk_size =
                    usize::from_str_radix(remaining[..size_end].trim(), 16).unwrap_or(0);
                if chunk_size == 0 {
                    break;
                }
                let chunk_start = size_end + 2;
                decoded.push_str(&remaining[chunk_start..chunk_start + chunk_size]);
                remaining = &remaining[chunk_start + chunk_size + 2..];
            }
            decoded
        } else {
            response_body.to_string()
        };

        let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(json["text"], "hello from local service");
        assert!(json["duration"].as_f64().unwrap() > 0.0);
        assert!(json["language"].as_str().is_some());

        let segments = json["segments"].as_array().unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0]["id"], 0);
        assert_eq!(segments[0]["text"], "hello from");
        assert!(segments[0]["start"].as_f64().unwrap() >= 0.0);
        assert_eq!(segments[1]["id"], 1);
        assert_eq!(segments[1]["text"], "local service");

        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);

        handle.shutdown().await;
    }

    #[test]
    fn decode_wav_resamples_to_16k() {
        let samples = sine_samples(8000, 0.5, 440.0);

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
            for sample in samples {
                let v = (sample * i16::MAX as f32) as i16;
                writer.write_sample(v).unwrap();
            }
            writer.finalize().unwrap();
        }

        let decoded = decode_wav_to_mono_16k(&cursor.into_inner()).unwrap();
        assert!(decoded.len() > 7000);
        assert!(decoded.len() < 9000);
    }

    #[test]
    fn adaptive_long_form_chunking_splits_large_inputs() {
        let counter = Arc::new(ChunkCountingTranscriber::new());
        let transcriber: Arc<dyn Transcriber> = counter.clone();
        let samples = sine_samples(SERVICE_SAMPLE_RATE as u32, 95.0, 440.0);

        let result =
            transcribe_segments_adaptive(transcriber.clone(), &samples, Some("de"), None).unwrap();

        let calls = counter.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert!(calls.iter().all(|&len| len <= 30 * SERVICE_SAMPLE_RATE));

        assert_eq!(result.language, "de");
        assert_eq!(result.segments.len(), 4);
        assert_eq!(result.segments[0].start, 0.0);
        assert_eq!(result.segments[1].start, 30.0);
        assert_eq!(result.segments[2].start, 60.0);
        assert_eq!(result.segments[3].start, 90.0);
    }

    #[test]
    fn adaptive_long_form_chunking_reports_mixed_language_when_chunks_differ() {
        let detector = Arc::new(LanguageTrackingTranscriber::new(&["de", "en", "de", "en"]));
        let transcriber: Arc<dyn Transcriber> = detector.clone();
        let samples = sine_samples(SERVICE_SAMPLE_RATE as u32, 95.0, 440.0);

        let result =
            transcribe_segments_adaptive(transcriber, &samples, Some("auto"), None).unwrap();

        let seen = detector.languages_seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        assert_eq!(seen[0].as_deref(), Some("auto"));
        assert_eq!(seen[1].as_deref(), Some("auto"));
        assert_eq!(seen[2].as_deref(), Some("auto"));
        assert_eq!(seen[3].as_deref(), Some("auto"));
        assert_eq!(result.language, "mixed");
    }
}
