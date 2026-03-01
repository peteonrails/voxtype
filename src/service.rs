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
use std::io::Cursor;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

use crate::config::{Config, LanguageConfig, ServiceConfig, TranscriptionEngine};
use crate::error::{TranscribeError, VoxtypeError};
use crate::transcribe::Transcriber;

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

/// Start local OpenAI-compatible STT service with model-backed transcriber.
pub async fn start(
    config: &Config,
    config_path: Option<PathBuf>,
) -> Result<ServiceHandle, VoxtypeError> {
    let service_cfg = config.service.clone();

    let mut transcriber_config = config.clone();
    transcriber_config.whisper.language =
        default_language_for_service(&service_cfg, &config.whisper.language);

    let transcriber = tokio::task::spawn_blocking(move || {
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
    })??;

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

    let transcriber = state.transcriber.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        transcriber.transcribe_with_options(
            &samples,
            language_override.as_deref(),
            prompt_override.as_deref(),
        )
    });

    let result = tokio::select! {
        join = &mut task => join,
        _ = tokio::time::sleep(state.request_timeout) => {
            task.abort();
            return Err(ApiError {
                status: StatusCode::REQUEST_TIMEOUT,
                message: format!("Transcription timed out after {}ms", state.request_timeout.as_millis()),
                error_type: "timeout_error",
            });
        }
    };

    let text = match result {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => return Err(map_transcription_error(e)),
        Err(e) => {
            return Err(ApiError::internal(format!(
                "Transcription task failed: {}",
                e
            )))
        }
    };

    let format = response_format.trim().to_lowercase();
    if format == "text" {
        let mut response = text.into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        return Ok(response);
    }

    if format != "json" && format != "verbose_json" {
        return Err(ApiError::bad_request(format!(
            "Unsupported response_format '{}'; expected json, verbose_json, or text",
            response_format
        )));
    }

    Ok(Json(TranscriptionResponse { text }).into_response())
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
}
