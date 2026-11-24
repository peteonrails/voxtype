//! ElevenLabs Scribe batch transcription: one multipart upload of the
//! finished recording.

use std::time::Duration;

use reqwest::StatusCode;

use super::{
    api_origin, f32_to_pcm16le, redact_and_bound, sensitive_api_key_header, ElevenLabsTranscriber,
    API_KEY_HEADER, SAMPLE_RATE,
};
use crate::{config::ElevenLabsConfig, error::TranscribeError};

const BATCH_FILE_FORMAT: &str = "pcm_s16le_16";
const BATCH_FILE_NAME: &str = "voxtype.pcm";
const BATCH_FILE_MIME: &str = "application/octet-stream";
const MIN_BATCH_SAMPLES: usize = SAMPLE_RATE as usize / 10;
const BATCH_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

impl ElevenLabsTranscriber {
    fn batch_client(&self) -> Result<&reqwest::Client, TranscribeError> {
        if let Some(client) = self.batch_client.get() {
            return Ok(client);
        }

        let client = build_batch_client()?;
        let _ = self.batch_client.set(client);
        Ok(self
            .batch_client
            .get()
            .expect("ElevenLabs batch client was just initialized"))
    }

    pub(super) async fn batch_transcribe(
        &self,
        samples: &[f32],
    ) -> Result<String, TranscribeError> {
        validate_batch_input(samples)?;

        let client = self.batch_client()?;
        let upload = BatchUpload::new(samples, &self.config);
        let request = build_batch_request(client, &self.config, &self.api_key, upload)?;
        let response = client.execute(request).await.map_err(|error| {
            TranscribeError::NetworkError(format!(
                "ElevenLabs batch request failed: {}",
                redact_and_bound(&error.to_string(), &self.api_key)
            ))
        })?;
        let status = response.status();
        let body = response.text().await.map_err(|error| {
            TranscribeError::NetworkError(format!(
                "Could not read ElevenLabs batch response: {}",
                redact_and_bound(&error.to_string(), &self.api_key)
            ))
        })?;

        if !status.is_success() {
            return Err(map_batch_status(status, &body, &self.api_key));
        }

        parse_batch_response(&body)
    }
}

pub(super) fn validate_batch_input(samples: &[f32]) -> Result<(), TranscribeError> {
    if samples.is_empty() {
        return Err(TranscribeError::AudioFormat(
            "Empty audio buffer".to_string(),
        ));
    }
    if samples.len() < MIN_BATCH_SAMPLES {
        return Err(TranscribeError::AudioFormat(format!(
            "ElevenLabs batch transcription requires at least 100 ms of audio ({MIN_BATCH_SAMPLES} samples)"
        )));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct BatchUpload {
    pcm16le: Vec<u8>,
    language_code: Option<String>,
    model_id: String,
    file_format: &'static str,
    no_verbatim: bool,
    tag_audio_events: bool,
    diarize: bool,
    webhook: bool,
}

impl BatchUpload {
    fn new(samples: &[f32], config: &ElevenLabsConfig) -> Self {
        Self {
            pcm16le: f32_to_pcm16le(samples),
            language_code: config.language_code.clone(),
            model_id: config.model.clone(),
            file_format: BATCH_FILE_FORMAT,
            no_verbatim: config.no_verbatim,
            tag_audio_events: false,
            diarize: false,
            webhook: false,
        }
    }

    fn into_multipart(self) -> Result<reqwest::multipart::Form, TranscribeError> {
        let file = reqwest::multipart::Part::bytes(self.pcm16le)
            .file_name(BATCH_FILE_NAME)
            .mime_str(BATCH_FILE_MIME)
            .map_err(|error| {
                TranscribeError::InferenceFailed(format!(
                    "Could not construct ElevenLabs PCM upload: {error}"
                ))
            })?;

        let mut form = reqwest::multipart::Form::new()
            .part("file", file)
            .text("file_format", self.file_format)
            .text("model_id", self.model_id);
        if let Some(language_code) = self.language_code {
            form = form.text("language_code", language_code);
        }

        Ok(form
            .text("tag_audio_events", self.tag_audio_events.to_string())
            .text("no_verbatim", self.no_verbatim.to_string())
            .text("diarize", self.diarize.to_string())
            .text("webhook", self.webhook.to_string()))
    }
}

fn build_batch_client() -> Result<reqwest::Client, TranscribeError> {
    reqwest::Client::builder()
        // The custom xi-api-key header is not covered by reqwest's
        // cross-origin redirect sanitizer. Never forward it away from the
        // fixed regional production endpoint.
        .redirect(reqwest::redirect::Policy::none())
        .timeout(BATCH_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| {
            TranscribeError::InferenceFailed(format!(
                "Could not initialize ElevenLabs HTTP client: {error}"
            ))
        })
}

fn batch_url(config: &ElevenLabsConfig) -> Result<reqwest::Url, TranscribeError> {
    let endpoint = format!("https://{}/v1/speech-to-text", api_origin(config.region));
    reqwest::Url::parse(&endpoint).map_err(|error| {
        TranscribeError::ConfigError(format!("Invalid ElevenLabs batch endpoint: {error}"))
    })
}

fn build_batch_request(
    client: &reqwest::Client,
    config: &ElevenLabsConfig,
    api_key: &str,
    upload: BatchUpload,
) -> Result<reqwest::Request, TranscribeError> {
    client
        .post(batch_url(config)?)
        .header(API_KEY_HEADER, sensitive_api_key_header(api_key)?)
        .multipart(upload.into_multipart()?)
        .build()
        .map_err(|error| {
            TranscribeError::InferenceFailed(format!(
                "Could not construct ElevenLabs batch request: {error}"
            ))
        })
}

fn parse_batch_response(body: &str) -> Result<String, TranscribeError> {
    let response: serde_json::Value = serde_json::from_str(body).map_err(|error| {
        TranscribeError::InferenceFailed(format!(
            "Could not parse ElevenLabs batch response: {error}"
        ))
    })?;

    response
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            TranscribeError::InferenceFailed("ElevenLabs batch response missing text".to_string())
        })
}

struct BatchErrorInfo {
    provider_status: Option<String>,
    message: String,
}

fn batch_error_info(body: &str) -> BatchErrorInfo {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return BatchErrorInfo {
            provider_status: None,
            message: body.to_string(),
        };
    };

    let provider_status = value
        .pointer("/detail/status")
        .or_else(|| value.get("status"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let message = value
        .pointer("/detail/message")
        .or_else(|| value.get("message"))
        .or_else(|| value.get("error"))
        .or_else(|| value.get("detail"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(body)
        .to_string();

    BatchErrorInfo {
        provider_status,
        message,
    }
}

fn map_batch_status(status: StatusCode, body: &str, api_key: &str) -> TranscribeError {
    let error_info = batch_error_info(body);
    let provider_status = error_info.provider_status.as_deref();
    let message_hint = error_info.message.to_ascii_lowercase();
    let unaccepted_terms = provider_status == Some("unaccepted_terms")
        || message_hint.contains("terms of service")
        || message_hint.contains("terms have not been accepted")
        || message_hint.contains("accept the terms");
    let authentication_failure = status == StatusCode::UNAUTHORIZED
        || matches!(
            provider_status,
            Some("auth_error" | "invalid_api_key" | "authentication_error")
        )
        || (status == StatusCode::FORBIDDEN
            && (message_hint.contains("api key")
                || message_hint.contains("api-key")
                || message_hint.contains("authentication")));

    let message = if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
        && unaccepted_terms
    {
        "ElevenLabs Scribe terms have not been accepted: accept them in the ElevenLabs dashboard"
            .to_string()
    } else if authentication_failure {
        "ElevenLabs authentication failed: check [elevenlabs] api_key or ELEVENLABS_API_KEY"
            .to_string()
    } else if status == StatusCode::FORBIDDEN {
        "ElevenLabs access denied: check API key permissions and Scribe access".to_string()
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        "ElevenLabs rate limit exceeded: wait and try again".to_string()
    } else {
        let detail = redact_and_bound(error_info.message.trim(), api_key);
        if detail.is_empty() {
            format!("ElevenLabs batch request failed (HTTP {status})")
        } else {
            format!("ElevenLabs batch request failed (HTTP {status}): {detail}")
        }
    };

    TranscribeError::InferenceFailed(message)
}

#[cfg(test)]
mod tests {
    use super::super::MAX_PROVIDER_ERROR_CHARS;
    use super::*;
    use crate::config::{ElevenLabsMode, ElevenLabsRegion};
    use crate::transcribe::Transcriber;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const API_KEY: &str = "elevenlabs-secret-key";

    fn batch_config(region: ElevenLabsRegion) -> ElevenLabsConfig {
        ElevenLabsConfig {
            api_key: Some(API_KEY.to_string()),
            region,
            language_code: Some("en".to_string()),
            mode: ElevenLabsMode::Batch,
            ..ElevenLabsConfig::default()
        }
    }

    fn batch_transcriber() -> ElevenLabsTranscriber {
        ElevenLabsTranscriber::new(&batch_config(ElevenLabsRegion::Global))
            .expect("valid test transcriber")
    }

    #[test]
    fn rejects_empty_and_short_batch_input_before_requesting() {
        let transcriber = batch_transcriber();

        let empty_error = transcriber.transcribe(&[]).unwrap_err().to_string();
        assert!(empty_error.contains("Empty audio buffer"));

        let short_error = transcriber
            .transcribe(&vec![0.0; MIN_BATCH_SAMPLES - 1])
            .unwrap_err()
            .to_string();
        assert!(short_error.contains("at least 100 ms"));
        assert!(short_error.contains("1600 samples"));
        assert!(transcriber.batch_client.get().is_none());
    }

    #[test]
    fn builds_pcm_batch_upload_with_fixed_fields_and_both_cleanup_values() {
        let samples = [-2.0, -0.5, 0.0, 0.5, 2.0];
        let expected_pcm: Vec<u8> = [-32767i16, -16384, 0, 16384, 32767]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();

        for no_verbatim in [false, true] {
            let config = ElevenLabsConfig {
                no_verbatim,
                ..batch_config(ElevenLabsRegion::Global)
            };
            assert_eq!(
                BatchUpload::new(&samples, &config),
                BatchUpload {
                    pcm16le: expected_pcm.clone(),
                    language_code: Some("en".to_string()),
                    model_id: "scribe_v3".to_string(),
                    file_format: "pcm_s16le_16",
                    no_verbatim,
                    tag_audio_events: false,
                    diarize: false,
                    webhook: false,
                }
            );
        }
    }

    #[tokio::test]
    async fn multipart_wire_body_carries_the_cleanup_value_and_the_configured_model() {
        async fn capture_multipart(config: &ElevenLabsConfig) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    let Some(header_end) =
                        request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .expect("multipart request has a content length");
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
                request
            });

            let form = BatchUpload::new(&[0.0; MIN_BATCH_SAMPLES], config)
                .into_multipart()
                .unwrap();
            let response = reqwest::Client::new()
                .post(format!("http://{address}/upload"))
                .multipart(form)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            String::from_utf8(server.await.unwrap()).unwrap()
        }

        for (no_verbatim, model) in [(false, "scribe_v3"), (true, "scribe_v9_future")] {
            let config = ElevenLabsConfig {
                no_verbatim,
                model: model.to_string(),
                ..batch_config(ElevenLabsRegion::Global)
            };
            let request = capture_multipart(&config).await;
            for (name, value) in [
                ("no_verbatim", no_verbatim.to_string()),
                ("model_id", model.to_string()),
            ] {
                let expected_field = format!("name=\"{name}\"\r\n\r\n{value}\r\n");
                assert!(request.contains(&expected_field), "{request}");
            }
            assert!(!request.contains(API_KEY));
        }
    }

    #[test]
    fn builds_sensitive_regional_batch_requests() {
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
        let client = build_batch_client().unwrap();

        for (region, expected_host) in cases {
            let config = batch_config(region);
            let upload = BatchUpload::new(&[0.0; MIN_BATCH_SAMPLES], &config);
            let request = build_batch_request(&client, &config, API_KEY, upload).unwrap();
            let uri = request.url().as_str();

            assert_eq!(request.method(), reqwest::Method::POST);
            assert_eq!(request.url().scheme(), "https");
            assert_eq!(request.url().host_str(), Some(expected_host));
            assert_eq!(request.url().path(), "/v1/speech-to-text");
            assert!(request.url().query().is_none());
            assert!(!uri.contains(API_KEY));
            let header = request.headers().get(&API_KEY_HEADER).unwrap();
            assert_eq!(header, API_KEY);
            assert!(header.is_sensitive());
            assert!(request
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("multipart/form-data; boundary="));
        }
    }

    #[test]
    fn lazily_builds_and_reuses_batch_client() {
        let transcriber = batch_transcriber();
        assert!(transcriber.batch_client.get().is_none());

        let first = transcriber.batch_client().unwrap() as *const reqwest::Client;
        let second = transcriber.batch_client().unwrap() as *const reqwest::Client;

        assert_eq!(first, second);
        assert!(transcriber.batch_client.get().is_some());
        assert_eq!(BATCH_REQUEST_TIMEOUT, Duration::from_secs(120));
    }

    #[test]
    fn parses_documented_batch_response() {
        let fixture = r#"{
            "language_code": "en",
            "language_probability": 0.98,
            "text": "Hello world!",
            "words": [{
                "end": 0.5,
                "logprob": -0.124,
                "speaker_id": "speaker_1",
                "start": 0,
                "text": "Hello",
                "type": "word"
            }]
        }"#;

        assert_eq!(parse_batch_response(fixture).unwrap(), "Hello world!");
    }

    #[test]
    fn rejects_malformed_or_textless_batch_responses() {
        let malformed = parse_batch_response(r#"{"text":"unfinished""#)
            .unwrap_err()
            .to_string();
        let missing = parse_batch_response(r#"{"language_code":"en","words":[]}"#)
            .unwrap_err()
            .to_string();

        assert!(malformed.contains("Could not parse ElevenLabs batch response"));
        assert!(missing.contains("missing text"));
        assert!(!malformed.contains("unfinished"));
    }

    #[test]
    fn maps_batch_http_statuses_to_remediation() {
        let cases = [
            (
                StatusCode::UNAUTHORIZED,
                r#"{"detail":{"status":"invalid_api_key","message":"Invalid xi-api-key"}}"#,
                "Transcription failed: ElevenLabs authentication failed: check [elevenlabs] api_key or ELEVENLABS_API_KEY",
            ),
            (
                StatusCode::FORBIDDEN,
                r#"{"detail":{"status":"unaccepted_terms","message":"Forbidden"}}"#,
                "Transcription failed: ElevenLabs Scribe terms have not been accepted: accept them in the ElevenLabs dashboard",
            ),
            (
                StatusCode::FORBIDDEN,
                r#"{"detail":{"status":"forbidden","message":"API key terminated"}}"#,
                "Transcription failed: ElevenLabs authentication failed: check [elevenlabs] api_key or ELEVENLABS_API_KEY",
            ),
            (
                StatusCode::TOO_MANY_REQUESTS,
                r#"{"detail":{"status":"rate_limited","message":"Too many requests"}}"#,
                "Transcription failed: ElevenLabs rate limit exceeded: wait and try again",
            ),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"detail":{"status":"server_error","message":"temporary provider failure"}}"#,
                "Transcription failed: ElevenLabs batch request failed (HTTP 500 Internal Server Error): temporary provider failure",
            ),
        ];

        for (status, body, expected) in cases {
            assert_eq!(
                map_batch_status(status, body, API_KEY).to_string(),
                expected
            );
        }
    }

    #[test]
    fn redacts_and_bounds_api_key_in_batch_provider_errors() {
        let body = serde_json::json!({
            "detail": {
                "status": "server_error",
                "message": format!("failed for {API_KEY}: {}", "x".repeat(600)),
            }
        })
        .to_string();
        let surfaced = map_batch_status(StatusCode::BAD_GATEWAY, &body, API_KEY).to_string();

        assert!(!surfaced.contains(API_KEY));
        assert!(surfaced.contains("[REDACTED]"));
        assert!(surfaced.ends_with("..."));
        assert!(surfaced.chars().count() <= MAX_PROVIDER_ERROR_CHARS + 100);
    }
}
