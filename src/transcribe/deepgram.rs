//! Deepgram pre-recorded speech-to-text backend.

use super::audio::{encode_wav_s16le, SAMPLE_RATE};
use super::Transcriber;
use crate::config::DeepgramConfig;
use crate::error::TranscribeError;
use serde::Deserialize;
use std::net::IpAddr;
use std::time::Duration;

#[derive(Debug)]
pub struct DeepgramTranscriber {
    config: DeepgramConfig,
    api_key: String,
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

impl DeepgramTranscriber {
    pub fn new(config: DeepgramConfig) -> Result<Self, TranscribeError> {
        validate_config(&config)?;

        let api_key = std::env::var("DEEPGRAM_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
            .or_else(|| config.api_key.clone().filter(|key| !key.trim().is_empty()))
            .ok_or_else(|| {
                TranscribeError::ConfigError(
                    "Deepgram API key required: set DEEPGRAM_API_KEY or [deepgram] api_key".into(),
                )
            })?;

        tracing::info!(
            "Deepgram backend configured: endpoint={}, model={}, language={}, smart_format={}, mip_opt_out={}, timeout={}s",
            config.endpoint,
            config.model,
            config.language,
            config.smart_format,
            config.mip_opt_out,
            config.timeout_secs,
        );

        Ok(Self { config, api_key })
    }

    fn request(&self) -> ureq::Request {
        let mut request = ureq::post(&self.config.endpoint)
            .timeout(Duration::from_secs(self.config.timeout_secs))
            .set("Authorization", &format!("Token {}", self.api_key))
            .set("Content-Type", "audio/wav")
            .query("model", &self.config.model)
            .query(
                "smart_format",
                if self.config.smart_format {
                    "true"
                } else {
                    "false"
                },
            )
            .query(
                "mip_opt_out",
                if self.config.mip_opt_out {
                    "true"
                } else {
                    "false"
                },
            );

        request = match self.config.language.as_str() {
            "auto" => request.query("detect_language", "true"),
            language => request.query("language", language),
        };
        request
    }

    fn parse_response(body: &str) -> Result<String, TranscribeError> {
        let response: DeepgramResponse = serde_json::from_str(body).map_err(|e| {
            TranscribeError::RemoteError(format!("Deepgram returned malformed JSON: {}", e))
        })?;

        let transcript = response
            .results
            .channels
            .first()
            .and_then(|channel| channel.alternatives.first())
            .map(|alternative| alternative.transcript.trim().to_string())
            .ok_or_else(|| {
                TranscribeError::RemoteError(
                    "Deepgram response contained no channel alternatives".into(),
                )
            })?;

        if transcript.is_empty() {
            return Err(TranscribeError::RemoteError(
                "Deepgram returned an empty transcript".into(),
            ));
        }

        Ok(transcript)
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

    fn last_detected_language(&self) -> Option<String> {
        match self.config.language.as_str() {
            "auto" | "multi" => None,
            language => Some(language.to_string()),
        }
    }
}

fn validate_config(config: &DeepgramConfig) -> Result<(), TranscribeError> {
    let endpoint = reqwest::Url::parse(&config.endpoint).map_err(|e| {
        TranscribeError::ConfigError(format!("Invalid Deepgram endpoint URL: {}", e))
    })?;
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(TranscribeError::ConfigError(
            "Deepgram endpoint must not contain credentials".into(),
        ));
    }
    let is_loopback = endpoint.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && is_loopback) {
        return Err(TranscribeError::ConfigError(
            "Deepgram endpoint must use HTTPS (HTTP is allowed only for localhost tests)".into(),
        ));
    }
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

fn map_request_error(error: ureq::Error) -> TranscribeError {
    match error {
        ureq::Error::Status(code, response) => {
            let detail = response.into_string().unwrap_or_default();
            let summary = match code {
                401 | 403 => "authentication failed",
                402 => "account has insufficient credit",
                429 => "rate limit exceeded",
                _ => "request failed",
            };
            if detail.is_empty() {
                TranscribeError::RemoteError(format!("Deepgram {} (HTTP {})", summary, code))
            } else {
                TranscribeError::RemoteError(format!(
                    "Deepgram {} (HTTP {}): {}",
                    summary,
                    code,
                    truncate_detail(&detail)
                ))
            }
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
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn config_with_key() -> DeepgramConfig {
        DeepgramConfig {
            api_key: Some("config-key".into()),
            ..Default::default()
        }
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
        std::env::set_var("DEEPGRAM_API_KEY", "environment-key");
        let transcriber = DeepgramTranscriber::new(config_with_key()).unwrap();
        std::env::remove_var("DEEPGRAM_API_KEY");
        assert_eq!(transcriber.api_key, "environment-key");
    }

    #[test]
    fn config_key_is_fallback() {
        let _guard = env_lock();
        std::env::remove_var("DEEPGRAM_API_KEY");
        let transcriber = DeepgramTranscriber::new(config_with_key()).unwrap();
        assert_eq!(transcriber.api_key, "config-key");
    }

    #[test]
    fn missing_key_is_rejected() {
        let _guard = env_lock();
        std::env::remove_var("DEEPGRAM_API_KEY");
        let error = DeepgramTranscriber::new(DeepgramConfig::default()).unwrap_err();
        assert!(error.to_string().contains("DEEPGRAM_API_KEY"));
    }

    #[test]
    fn endpoint_requires_https_except_localhost() {
        let mut config = config_with_key();
        config.endpoint = "http://example.com/v1/listen".into();
        let error = DeepgramTranscriber::new(config).unwrap_err();
        assert!(error.to_string().contains("HTTPS"));

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
    fn parses_transcript_and_rejects_empty_transcript() {
        let body =
            r#"{"results":{"channels":[{"alternatives":[{"transcript":" hello world "}]}]}}"#;
        assert_eq!(
            DeepgramTranscriber::parse_response(body).unwrap(),
            "hello world"
        );

        let empty = r#"{"results":{"channels":[{"alternatives":[{"transcript":""}]}]}}"#;
        assert!(DeepgramTranscriber::parse_response(empty)
            .unwrap_err()
            .to_string()
            .contains("empty transcript"));
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
        let transcriber = DeepgramTranscriber::new(DeepgramConfig {
            endpoint,
            api_key: Some("secret-test-key".into()),
            language: "auto".into(),
            ..Default::default()
        })
        .unwrap();

        let transcript = transcriber.transcribe(&[0.0; 160]).unwrap();
        assert_eq!(transcript, "test");
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/listen?"));
        assert!(request.contains("model=nova-3"));
        assert!(request.contains("smart_format=true"));
        assert!(request.contains("mip_opt_out=true"));
        assert!(request.contains("detect_language=true"));
        assert!(!request.contains("&language="));
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
        let transcriber = DeepgramTranscriber::new(DeepgramConfig {
            endpoint,
            api_key: Some("key".into()),
            ..Default::default()
        })
        .unwrap();
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
        let transcriber = DeepgramTranscriber::new(DeepgramConfig {
            endpoint,
            api_key: Some("key".into()),
            timeout_secs: 1,
            ..Default::default()
        })
        .unwrap();

        let error = transcriber.transcribe(&[0.0; 16]).unwrap_err();
        server.join().unwrap();
        assert!(matches!(error, TranscribeError::NetworkError(_)));
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
