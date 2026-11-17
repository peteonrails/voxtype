//! Remote speech-to-text transcription via OpenAI-compatible API
//!
//! Sends audio to a remote whisper.cpp server or OpenAI-compatible endpoint
//! for transcription, enabling use of GPU servers for faster inference.
//!
//! Note: Remote APIs don't support language arrays. When a language array is
//! configured, the first/primary language is used.

use super::Transcriber;
use crate::config::{LanguageConfig, WhisperConfig};
use crate::error::TranscribeError;
use std::io::Cursor;
use std::sync::Mutex;
use std::time::Duration;
use ureq::serde_json;

/// Remote transcriber using OpenAI-compatible Whisper API
#[derive(Debug)]
pub struct RemoteTranscriber {
    /// Base endpoint URL (e.g., "http://192.168.1.100:8080")
    endpoint: String,
    /// Model name to send to server
    model: String,
    /// Language configuration
    language: LanguageConfig,
    /// Whether to translate to English
    translate: bool,
    /// Optional API key for authentication
    api_key: Option<String>,
    /// Optional initial prompt for transcription context
    initial_prompt: Option<String>,
    /// Profile vocabulary selected for the current recording.
    active_vocabulary: Mutex<Vec<String>>,
    /// Send an explicit `language=auto` field instead of omitting it
    send_auto_language: bool,
    /// Request timeout
    timeout: Duration,
}

impl RemoteTranscriber {
    /// Create a new remote transcriber from config
    pub fn new(config: &WhisperConfig) -> Result<Self, TranscribeError> {
        let endpoint = config
            .remote_endpoint
            .as_ref()
            .ok_or_else(|| {
                TranscribeError::ConfigError(
                    "remote_endpoint is required when mode = 'remote'".into(),
                )
            })?
            .clone();

        // Validate endpoint URL format
        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
            return Err(TranscribeError::ConfigError(format!(
                "remote_endpoint must start with http:// or https://, got: {}",
                endpoint
            )));
        }

        // Warn about non-HTTPS for non-localhost endpoints
        if endpoint.starts_with("http://")
            && !endpoint.contains("localhost")
            && !endpoint.contains("127.0.0.1")
            && !endpoint.contains("[::1]")
        {
            tracing::warn!(
                "Remote endpoint uses HTTP without TLS. Audio data will be transmitted unencrypted!"
            );
        }

        // Check for API key in config or environment
        let api_key = config
            .remote_api_key
            .clone()
            .or_else(|| std::env::var("VOXTYPE_WHISPER_API_KEY").ok());

        let model = config
            .remote_model
            .clone()
            .unwrap_or_else(|| "whisper-1".to_string());

        let timeout = Duration::from_secs(config.remote_timeout_secs.unwrap_or(30));

        // Warn if language array is configured (remote APIs don't support arrays)
        if config.language.is_multiple() {
            tracing::warn!(
                "Remote backend doesn't support language arrays. Using primary language '{}' from {:?}",
                config.language.primary(),
                config.language.as_vec()
            );
        }

        tracing::info!(
            "Configured remote transcriber: endpoint={}, model={}, timeout={}s",
            endpoint,
            model,
            timeout.as_secs()
        );

        let initial_prompt = config
            .initial_prompt
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned();

        Ok(Self {
            endpoint,
            model,
            language: config.language.clone(),
            translate: config.translate,
            api_key,
            initial_prompt,
            active_vocabulary: Mutex::new(Vec::new()),
            send_auto_language: config.remote_send_auto_language,
            timeout,
        })
    }

    fn request_url(&self) -> String {
        // Determine the API path based on whether we're doing transcription or translation
        let path = if self.translate {
            "/v1/audio/translations"
        } else {
            "/v1/audio/transcriptions"
        };

        // Reuse a final /v1 path component without treating a hostname,
        // query value or fragment as part of the API path (#603, #722).
        let mut url = match reqwest::Url::parse(&self.endpoint) {
            Ok(url) => url,
            // Preserve the existing HTTP client's error handling for malformed URLs.
            Err(_) => return format!("{}{}", self.endpoint.trim_end_matches('/'), path),
        };
        let trimmed = url.path().trim_end_matches('/');
        let base = trimmed
            .strip_suffix("/v1")
            .unwrap_or(trimmed)
            .trim_end_matches('/');
        let request_path = format!("{base}{path}");
        url.set_path(&request_path);
        url.to_string()
    }

    /// Encode f32 samples to WAV format
    fn encode_wav(&self, samples: &[f32]) -> Result<Vec<u8>, TranscribeError> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut buffer = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut buffer, spec).map_err(|e| {
            TranscribeError::AudioFormat(format!("Failed to create WAV writer: {}", e))
        })?;

        // Convert f32 [-1.0, 1.0] to i16
        for &sample in samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let scaled = (clamped * i16::MAX as f32) as i16;
            writer.write_sample(scaled).map_err(|e| {
                TranscribeError::AudioFormat(format!("Failed to write sample: {}", e))
            })?;
        }

        writer
            .finalize()
            .map_err(|e| TranscribeError::AudioFormat(format!("Failed to finalize WAV: {}", e)))?;

        Ok(buffer.into_inner())
    }

    /// Build the multipart form body for the API request
    fn build_multipart_body(&self, wav_data: &[u8]) -> (String, Vec<u8>) {
        let boundary = format!(
            "----VoxtypeBoundary{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        let mut body = Vec::new();

        // Add file field
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: audio/wav\r\n\r\n");
        body.extend_from_slice(wav_data);
        body.extend_from_slice(b"\r\n");

        // Add model field
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"model\"\r\n\r\n");
        body.extend_from_slice(self.model.as_bytes());
        body.extend_from_slice(b"\r\n");

        // Add language field (omitted in auto-detect mode unless the server
        // needs an explicit "auto", e.g. whisper.cpp server which otherwise
        // falls back to its own -l default)
        // For language arrays, use the primary language since remote APIs don't support arrays
        if !self.language.is_auto() || self.send_auto_language {
            body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            body.extend_from_slice(b"Content-Disposition: form-data; name=\"language\"\r\n\r\n");
            body.extend_from_slice(self.language.primary().as_bytes());
            body.extend_from_slice(b"\r\n");
        }

        // Preserve the configured prompt and append active profile terms.
        let vocabulary = self
            .active_vocabulary
            .lock()
            .map(|terms| terms.clone())
            .unwrap_or_default();
        let prompt = super::prompt_with_vocabulary(self.initial_prompt.as_deref(), &vocabulary);
        if let Some(ref prompt) = prompt {
            body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            body.extend_from_slice(b"Content-Disposition: form-data; name=\"prompt\"\r\n\r\n");
            body.extend_from_slice(prompt.as_bytes());
            body.extend_from_slice(b"\r\n");
        }

        // Add response_format field
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"response_format\"\r\n\r\n");
        body.extend_from_slice(b"json\r\n");

        // End boundary
        body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

        (boundary, body)
    }
}

impl Transcriber for RemoteTranscriber {
    fn set_vocabulary(&self, terms: &[String]) {
        if let Ok(mut active) = self.active_vocabulary.lock() {
            active.clear();
            active.extend_from_slice(terms);
        }
    }

    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".into()));
        }

        let duration_secs = samples.len() as f32 / 16000.0;
        tracing::debug!(
            "Sending {:.2}s of audio to remote server ({} samples)",
            duration_secs,
            samples.len()
        );

        let start = std::time::Instant::now();

        // Encode audio to WAV
        let wav_data = self.encode_wav(samples)?;
        tracing::debug!("Encoded WAV: {} bytes", wav_data.len());

        // Build multipart form
        let (boundary, body) = self.build_multipart_body(&wav_data);

        let url = self.request_url();

        // Build request
        let mut request = ureq::post(&url).timeout(self.timeout).set(
            "Content-Type",
            &format!("multipart/form-data; boundary={}", boundary),
        );

        // Add authorization if API key is configured
        if let Some(ref key) = self.api_key {
            request = request.set("Authorization", &format!("Bearer {}", key));
        }

        // Send request
        let response = request.send_bytes(&body).map_err(|e| match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                TranscribeError::RemoteError(format!("Server returned {}: {}", code, body))
            }
            ureq::Error::Transport(t) => {
                TranscribeError::NetworkError(format!("Request failed: {}", t))
            }
        })?;

        // Parse JSON response
        let json: serde_json::Value = response.into_json().map_err(|e| {
            TranscribeError::RemoteError(format!("Failed to parse response: {}", e))
        })?;

        // Extract text from response
        let text = json
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TranscribeError::RemoteError(format!("Response missing 'text' field: {}", json))
            })?
            .trim()
            .to_string();

        tracing::info!(
            "Remote transcription completed in {:.2}s: {:?}",
            start.elapsed().as_secs_f32(),
            if text.chars().count() > 50 {
                format!("{}...", text.chars().take(50).collect::<String>())
            } else {
                text.clone()
            }
        );

        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configure_fixture_stream(stream: &std::net::TcpStream, timeout: Duration) {
        // Accepted sockets can inherit the listener's nonblocking mode on BSD.
        stream.set_nonblocking(false).unwrap();
        stream.set_read_timeout(Some(timeout)).unwrap();
        stream.set_write_timeout(Some(timeout)).unwrap();
    }

    #[test]
    fn fixture_stream_reads_delayed_fragments_after_nonblocking_accept() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::sync::mpsc;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        // Emulate BSD accepted-socket inheritance even on Linux.
        stream.set_nonblocking(true).unwrap();
        configure_fixture_stream(&stream, Duration::from_secs(2));
        client
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte)?;
                header.push(byte[0]);
                assert!(header.len() < 16_384);
            }
            let mut body = [0; 4];
            stream.read_exact(&mut body)?;
            Ok::<_, std::io::Error>((header, body))
        });
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let writer_result = (|| -> std::io::Result<()> {
            for fragment in [
                &b"POST / HTTP/1.1\r\nContent-Len"[..],
                &b"gth: 4\r\n\r"[..],
                &b"\n"[..],
                &b"\0\xff"[..],
                &b"\x01\x80"[..],
            ] {
                std::thread::sleep(Duration::from_millis(50));
                client.write_all(fragment)?;
            }
            Ok(())
        })();
        let read_result = reader.join().unwrap();
        let (header, body) = read_result.unwrap();
        writer_result.unwrap();
        assert_eq!(header, b"POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\n");
        assert_eq!(body, [0, 255, 1, 128]);
    }

    #[test]
    fn fixture_stream_idle_read_is_bounded() {
        use std::io::Read;
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        let timeout = Duration::from_millis(100);
        configure_fixture_stream(&stream, timeout);
        assert_eq!(stream.read_timeout().unwrap(), Some(timeout));
        assert_eq!(stream.write_timeout().unwrap(), Some(timeout));
        let start = std::time::Instant::now();
        let error = stream.read_exact(&mut [0]).unwrap_err();
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ));
        assert!(start.elapsed() >= timeout);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    // The listener is loopback-only; release verification runs these tests in
    // the private no-egress NAS container, never against a provider or desktop.
    fn capture_request(endpoint_suffix: &str, translate: bool) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline, "no fixture request");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            configure_fixture_stream(&stream, Duration::from_secs(5));
            let mut request = Vec::new();
            let header_end = loop {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 16_384);
                if request.ends_with(b"\r\n\r\n") {
                    break request.len();
                }
            };
            let headers = String::from_utf8(request.clone()).unwrap();
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            assert!(length < 16_384);
            request.resize(header_end + length, 0);
            stream.read_exact(&mut request[header_end..]).unwrap();
            let response = r#"{"text":" fixture transcript "}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        let config = WhisperConfig {
            remote_endpoint: Some(format!("http://{address}{endpoint_suffix}")),
            remote_model: Some("fixture-model".into()),
            remote_api_key: Some("fixture-not-a-secret".into()),
            language: LanguageConfig::Single("fr".into()),
            initial_prompt: Some("Technical discussion.".into()),
            remote_timeout_secs: Some(5),
            translate,
            ..Default::default()
        };
        let transcriber = RemoteTranscriber::new(&config).unwrap();
        transcriber.set_vocabulary(&["Voxtype".into(), "PipeWire".into()]);
        let result = transcriber.transcribe(&[0.0, 0.0]);
        let request = server.join().unwrap();
        assert_eq!(result.unwrap(), "fixture transcript");
        let headers = request
            .split("\r\n\r\n")
            .next()
            .unwrap()
            .to_ascii_lowercase();
        assert!(headers.contains("authorization: bearer fixture-not-a-secret\r\n"));
        assert!(headers.contains("content-type: multipart/form-data; boundary="));
        for field in [
            "name=\"model\"\r\n\r\nfixture-model\r\n",
            "name=\"language\"\r\n\r\nfr\r\n",
            "Technical discussion.\nVocabulary: Voxtype, PipeWire",
            "name=\"response_format\"\r\n\r\njson\r\n",
            "filename=\"audio.wav\"",
            "RIFF",
        ] {
            assert!(request.contains(field), "missing multipart field: {field}");
        }
        request.lines().next().unwrap().to_string()
    }

    #[test]
    fn endpoint_compatibility_openrouter_transcription() {
        assert_eq!(
            capture_request("/api/v1", false),
            "POST /api/v1/audio/transcriptions HTTP/1.1"
        );
    }

    #[test]
    fn endpoint_compatibility_versioned_translation() {
        assert_eq!(
            capture_request("/v1/", true),
            "POST /v1/audio/translations HTTP/1.1"
        );
    }

    #[test]
    fn endpoint_compatibility_query_stays_after_request_path() {
        assert_eq!(
            capture_request("/api/v1?route=/v1", false),
            "POST /api/v1/audio/transcriptions?route=/v1 HTTP/1.1"
        );
    }

    #[test]
    fn endpoint_compatibility_url_component_boundaries() {
        for (endpoint, expected) in [
            ("https://v1", "https://v1/v1/audio/transcriptions"),
            (
                "https://example.test/api/v1?route=/v1",
                "https://example.test/api/v1/audio/transcriptions?route=/v1",
            ),
            (
                "https://example.test/api/v1#section/v1",
                "https://example.test/api/v1/audio/transcriptions#section/v1",
            ),
        ] {
            let config = WhisperConfig {
                remote_endpoint: Some(endpoint.into()),
                ..Default::default()
            };
            assert_eq!(
                RemoteTranscriber::new(&config).unwrap().request_url(),
                expected,
                "{endpoint}"
            );
        }
    }

    #[test]
    fn endpoint_compatibility_http_base_matrix() {
        for (suffix, prefix) in [
            ("", ""),
            ("/", ""),
            ("/api", "/api"),
            ("/api/", "/api"),
            ("/v1", ""),
            ("/v1/", ""),
            ("/api/v1", "/api"),
            ("/api/v1/", "/api"),
        ] {
            for translate in [false, true] {
                let task = if translate {
                    "translations"
                } else {
                    "transcriptions"
                };
                assert_eq!(
                    capture_request(suffix, translate),
                    format!("POST {prefix}/v1/audio/{task} HTTP/1.1"),
                    "{suffix}"
                );
            }
        }
    }

    #[test]
    fn endpoint_compatibility_https_path_matrix() {
        for (suffix, prefix) in [
            ("", ""),
            ("/", ""),
            ("/api", "/api"),
            ("/api/", "/api"),
            ("/v1", ""),
            ("/v1/", ""),
            ("/api/v1", "/api"),
            ("/api/v1/", "/api"),
            ("/v10", "/v10"),
            ("/api/v1/resources", "/api/v1/resources"),
            ("/custom/inference", "/custom/inference"),
            ("/api/audio/transcriptions", "/api/audio/transcriptions"),
            ("/api%2Ftenant/v1", "/api%2Ftenant"),
        ] {
            for translate in [false, true] {
                let config = WhisperConfig {
                    remote_endpoint: Some(format!("https://example.test{suffix}")),
                    translate,
                    ..Default::default()
                };
                let task = if translate {
                    "translations"
                } else {
                    "transcriptions"
                };
                assert_eq!(
                    RemoteTranscriber::new(&config).unwrap().request_url(),
                    format!("https://example.test{prefix}/v1/audio/{task}"),
                    "{suffix}"
                );
            }
        }
    }

    #[test]
    fn test_encode_wav_basic() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("http://localhost:8080".to_string()),
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();

        // Create a simple sine wave
        let samples: Vec<f32> = (0..16000)
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin() * 0.5)
            .collect();

        let wav = transcriber.encode_wav(&samples).unwrap();

        // WAV header is 44 bytes, then 16000 samples * 2 bytes = 32000 bytes
        assert_eq!(wav.len(), 44 + 32000);

        // Check WAV magic
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn test_config_validation_missing_endpoint() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: None, // Missing!
            ..Default::default()
        };

        let result = RemoteTranscriber::new(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("remote_endpoint"));
    }

    #[test]
    fn test_endpoint_normalization_bare_host() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("https://api.openai.com".to_string()),
            ..Default::default()
        };
        let transcriber = RemoteTranscriber::new(&config).unwrap();
        assert_eq!(
            transcriber.request_url(),
            "https://api.openai.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn test_endpoint_normalization_trailing_slash() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("https://api.openai.com/".to_string()),
            ..Default::default()
        };
        let transcriber = RemoteTranscriber::new(&config).unwrap();
        assert_eq!(
            transcriber.request_url(),
            "https://api.openai.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn test_endpoint_normalization_trailing_v1() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("https://api.openai.com/v1".to_string()),
            ..Default::default()
        };
        let transcriber = RemoteTranscriber::new(&config).unwrap();
        assert_eq!(
            transcriber.request_url(),
            "https://api.openai.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn test_endpoint_normalization_trailing_v1_with_slash() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("https://api.openai.com/v1/".to_string()),
            ..Default::default()
        };
        let transcriber = RemoteTranscriber::new(&config).unwrap();
        assert_eq!(
            transcriber.request_url(),
            "https://api.openai.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn test_config_validation_invalid_url() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("not-a-url".to_string()),
            ..Default::default()
        };

        let result = RemoteTranscriber::new(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("http://"));
    }

    #[test]
    fn test_multipart_body_structure() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("http://localhost:8080".to_string()),
            remote_model: Some("large-v3".to_string()),
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        let wav_data = vec![0u8; 100]; // Dummy data

        let (boundary, body) = transcriber.build_multipart_body(&wav_data);

        let body_str = String::from_utf8_lossy(&body);

        // Verify boundary is used
        assert!(body_str.contains(&boundary));

        // Verify required fields
        assert!(body_str.contains("name=\"file\""));
        assert!(body_str.contains("filename=\"audio.wav\""));
        assert!(body_str.contains("name=\"model\""));
        assert!(body_str.contains("large-v3"));
        assert!(body_str.contains("name=\"language\""));
        assert!(body_str.contains("name=\"response_format\""));
        assert!(body_str.contains("json"));
    }

    #[test]
    fn test_multipart_body_omits_language_when_auto() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("http://localhost:8080".to_string()),
            language: LanguageConfig::Single("auto".to_string()),
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        let wav_data = vec![0u8; 100];

        let (_boundary, body) = transcriber.build_multipart_body(&wav_data);
        let body_str = String::from_utf8_lossy(&body);

        assert!(!body_str.contains("name=\"language\""));
    }

    #[test]
    fn test_multipart_body_sends_auto_language_when_enabled() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("http://localhost:8080".to_string()),
            language: LanguageConfig::Single("auto".to_string()),
            remote_send_auto_language: true,
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        let wav_data = vec![0u8; 100];

        let (_boundary, body) = transcriber.build_multipart_body(&wav_data);
        let body_str = String::from_utf8_lossy(&body);

        assert!(body_str.contains("name=\"language\"\r\n\r\nauto\r\n"));
    }

    #[test]
    fn test_multipart_body_includes_prompt() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("http://localhost:8080".to_string()),
            initial_prompt: Some("Technical discussion about Rust and Kubernetes.".to_string()),
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        let wav_data = vec![0u8; 100];

        let (_boundary, body) = transcriber.build_multipart_body(&wav_data);
        let body_str = String::from_utf8_lossy(&body);

        assert!(body_str.contains("name=\"prompt\""));
        assert!(body_str.contains("Technical discussion about Rust and Kubernetes."));
    }

    #[test]
    fn multipart_prompt_preserves_configured_prompt_and_adds_profile_vocabulary() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("http://localhost:8080".to_string()),
            initial_prompt: Some("Technical discussion.".to_string()),
            ..Default::default()
        };
        let transcriber = RemoteTranscriber::new(&config).unwrap();
        transcriber.set_vocabulary(&["Voxtype".into(), "PipeWire".into()]);

        let (_, body) = transcriber.build_multipart_body(&[0u8; 100]);
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("Technical discussion.\nVocabulary: Voxtype, PipeWire"));
    }

    #[test]
    fn test_multipart_body_excludes_empty_prompt() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("http://localhost:8080".to_string()),
            initial_prompt: Some("".to_string()),
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        let wav_data = vec![0u8; 100];

        let (_boundary, body) = transcriber.build_multipart_body(&wav_data);
        let body_str = String::from_utf8_lossy(&body);

        assert!(!body_str.contains("name=\"prompt\""));
    }

    #[test]
    fn test_multipart_body_excludes_prompt_when_none() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("http://localhost:8080".to_string()),
            initial_prompt: None,
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        let wav_data = vec![0u8; 100];

        let (_boundary, body) = transcriber.build_multipart_body(&wav_data);
        let body_str = String::from_utf8_lossy(&body);

        assert!(!body_str.contains("name=\"prompt\""));
    }

    #[test]
    fn test_translate_false_uses_transcriptions_endpoint() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            translate: false,
            remote_endpoint: Some("http://localhost:8080".to_string()),
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();

        // Verify translate flag is stored correctly
        assert!(!transcriber.translate);

        assert_eq!(
            transcriber.request_url(),
            "http://localhost:8080/v1/audio/transcriptions"
        );
    }

    #[test]
    fn test_translate_true_uses_translations_endpoint() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            translate: true,
            remote_endpoint: Some("http://localhost:8080".to_string()),
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();

        // Verify translate flag is stored correctly
        assert!(transcriber.translate);

        assert_eq!(
            transcriber.request_url(),
            "http://localhost:8080/v1/audio/translations"
        );
    }

    #[test]
    fn test_api_key_from_config() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("http://localhost:8080".to_string()),
            remote_api_key: Some("sk-test-key-123".to_string()),
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        assert_eq!(transcriber.api_key, Some("sk-test-key-123".to_string()));
    }

    #[test]
    fn test_custom_timeout() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("http://localhost:8080".to_string()),
            remote_timeout_secs: Some(60),
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        assert_eq!(transcriber.timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_default_timeout() {
        let config = WhisperConfig {
            mode: Some(crate::config::WhisperMode::Remote),
            remote_endpoint: Some("http://localhost:8080".to_string()),
            ..Default::default()
        };

        let transcriber = RemoteTranscriber::new(&config).unwrap();
        assert_eq!(transcriber.timeout, Duration::from_secs(30));
    }
}
