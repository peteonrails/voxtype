//! Remote speech-to-text transcription via OpenAI-compatible API
//!
//! Sends audio to a remote whisper.cpp server or OpenAI-compatible endpoint
//! for transcription, enabling use of GPU servers for faster inference.
//!
//! Note: Remote APIs don't support language arrays. When a language array is
//! configured, the first/primary language is used.
//!
//! Meeting chunks go through [`Transcriber::transcribe_timed`], which asks
//! for `response_format=verbose_json` to get the server's segment
//! timestamps. Not every OpenAI-compatible server has it, so the first
//! timed request doubles as a probe and the answer is kept: a server that
//! rejects the format, or ignores it and sends no `segments`, gets plain
//! `json` from then on and one segment per chunk, as before.

use super::audio::encode_wav_s16le;
use super::{TimedSegment, Transcriber};
use crate::config::{LanguageConfig, WhisperConfig};
use crate::error::TranscribeError;
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
    /// Whether the server returns segments for `verbose_json`: unknown
    /// until the first timed request, then kept for the transcriber's life.
    verbose_json: Mutex<Option<bool>>,
}

/// What came back for a request.
enum Reply {
    Json(serde_json::Value),
    /// A 4xx answer, or a body that is not JSON: the request itself was not
    /// acceptable, which for `verbose_json` means the format is unsupported.
    Rejected(String),
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
            verbose_json: Mutex::new(None),
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

    /// Build the multipart form body for the API request
    #[cfg(test)]
    fn build_multipart_body(&self, wav_data: &[u8]) -> (String, Vec<u8>) {
        self.build_multipart_body_as(wav_data, "json")
    }

    /// The multipart body with the given `response_format`.
    fn build_multipart_body_as(&self, wav_data: &[u8], format: &str) -> (String, Vec<u8>) {
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
        body.extend_from_slice(format.as_bytes());
        body.extend_from_slice(b"\r\n");

        // End boundary
        body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

        (boundary, body)
    }
}

impl RemoteTranscriber {
    /// Send `samples` asking for `format`. Transport failures and 5xx
    /// answers are errors; a 4xx answer or a non-JSON body is
    /// [`Reply::Rejected`].
    fn request(&self, samples: &[f32], format: &str) -> Result<Reply, TranscribeError> {
        let wav_data = encode_wav_s16le(samples)?;
        tracing::debug!("Encoded WAV: {} bytes", wav_data.len());
        let (boundary, body) = self.build_multipart_body_as(&wav_data, format);
        let url = self.request_url();

        let mut request = ureq::post(&url).timeout(self.timeout).set(
            "Content-Type",
            &format!("multipart/form-data; boundary={}", boundary),
        );
        if let Some(ref key) = self.api_key {
            request = request.set("Authorization", &format!("Bearer {}", key));
        }

        let response = match request.send_bytes(&body) {
            Ok(response) => response,
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                let message = format!("Server returned {}: {}", code, body);
                return if (400..500).contains(&code) {
                    Ok(Reply::Rejected(message))
                } else {
                    Err(TranscribeError::RemoteError(message))
                };
            }
            Err(ureq::Error::Transport(t)) => {
                return Err(TranscribeError::NetworkError(format!(
                    "Request failed: {}",
                    t
                )))
            }
        };
        let text = response
            .into_string()
            .map_err(|e| TranscribeError::RemoteError(format!("Failed to read response: {}", e)))?;
        Ok(match serde_json::from_str(&text) {
            Ok(json) => Reply::Json(json),
            Err(e) => Reply::Rejected(format!("Failed to parse response: {}", e)),
        })
    }

    /// The `text` field of a JSON reply.
    fn reply_text(json: &serde_json::Value) -> Result<String, TranscribeError> {
        Ok(json
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TranscribeError::RemoteError(format!("Response missing 'text' field: {}", json))
            })?
            .trim()
            .to_string())
    }

    /// A plain `json` request, the way dictation always asks.
    fn transcribe_json(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        match self.request(samples, "json")? {
            Reply::Json(json) => Self::reply_text(&json),
            Reply::Rejected(message) => Err(TranscribeError::RemoteError(message)),
        }
    }

    fn verbose_supported(&self) -> Option<bool> {
        self.verbose_json.lock().ok().and_then(|g| *g)
    }

    fn remember_verbose(&self, supported: bool) {
        if let Ok(mut g) = self.verbose_json.lock() {
            if g.is_none() {
                tracing::info!(
                    "Remote server {} segment timestamps (verbose_json)",
                    if supported {
                        "returns"
                    } else {
                        "does not return"
                    }
                );
            }
            *g = Some(supported);
        }
    }
}

/// The text of a whole chunk as one segment spanning it, the way engines
/// without timestamps report.
fn one_segment(text: String, duration_secs: f32) -> Vec<TimedSegment> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![TimedSegment {
            text,
            start_secs: 0.0,
            end_secs: duration_secs,
        }]
    }
}

/// The `segments` of a `verbose_json` reply, or `None` when there are none
/// to read (the server ignored the format). Text is trimmed, times clamped
/// to the audio, and segments without words dropped.
fn verbose_segments(json: &serde_json::Value, duration_secs: f32) -> Option<Vec<TimedSegment>> {
    let raw = json.get("segments")?.as_array()?;
    let mut out = Vec::new();
    for seg in raw {
        let text = seg.get("text")?.as_str()?.trim();
        let start = seg.get("start")?.as_f64()? as f32;
        let end = seg.get("end")?.as_f64()? as f32;
        if !text.chars().any(char::is_alphanumeric) {
            continue;
        }
        let start = start.max(0.0).min(duration_secs);
        out.push(TimedSegment {
            text: text.to_string(),
            start_secs: start,
            end_secs: end.clamp(start, duration_secs),
        });
    }
    Some(out)
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

        let text = self.transcribe_json(samples)?;

        super::log_transcription("Remote transcription", start, &text);

        Ok(text)
    }

    /// Ask for the server's segment timestamps (`verbose_json`), once per
    /// call while the server is known or presumed to have them. See the
    /// module docs for how a server without them is detected and remembered.
    fn transcribe_timed(&self, samples: &[f32]) -> Result<Vec<TimedSegment>, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".into()));
        }
        let duration_secs = samples.len() as f32 / 16000.0;
        if self.verbose_supported() == Some(false) {
            return Ok(one_segment(self.transcribe_json(samples)?, duration_secs));
        }

        let start = std::time::Instant::now();
        match self.request(samples, "verbose_json")? {
            Reply::Json(json) => match verbose_segments(&json, duration_secs) {
                Some(segments) => {
                    self.remember_verbose(true);
                    tracing::info!(
                        "Remote timed transcription of {:.2}s completed in {:.2}s: {} segment(s)",
                        duration_secs,
                        start.elapsed().as_secs_f32(),
                        segments.len()
                    );
                    Ok(segments)
                }
                None => {
                    // Answered, just without segments: use the text, and ask
                    // for plain json from now on.
                    self.remember_verbose(false);
                    Ok(one_segment(Self::reply_text(&json)?, duration_secs))
                }
            },
            Reply::Rejected(message) => {
                tracing::debug!("verbose_json rejected: {}", message);
                // Only a server that then takes plain json has shown the
                // format is the problem; a bad key fails both and is not
                // remembered as "no segments".
                let text = self.transcribe_json(samples)?;
                self.remember_verbose(false);
                Ok(one_segment(text, duration_secs))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How long the loopback fixtures and their client wait on each other.
    /// A passing request is answered in milliseconds, so only a genuine
    /// hang reaches it; it is generous so a loaded CI machine never turns a
    /// slow round trip into a failure.
    const FIXTURE_TIMEOUT: Duration = Duration::from_secs(60);

    /// Read one HTTP request (headers, then a Content-Length body) from a
    /// fixture connection. An error means the client went away; the fixture
    /// drops that connection rather than panicking, and the client's own
    /// error is what the test reports.
    fn read_request(stream: &mut std::net::TcpStream, max: usize) -> std::io::Result<Vec<u8>> {
        use std::io::{Error, ErrorKind, Read};
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(FIXTURE_TIMEOUT))?;
        stream.set_write_timeout(Some(FIXTURE_TIMEOUT))?;
        let mut request = Vec::new();
        let header_end = loop {
            let mut byte = [0];
            stream.read_exact(&mut byte)?;
            request.push(byte[0]);
            if request.len() >= max {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "request headers too long",
                ));
            }
            if request.ends_with(b"\r\n\r\n") {
                break request.len();
            }
        };
        let headers = String::from_utf8_lossy(&request).into_owned();
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())?
            })
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "no Content-Length"))?;
        request.resize(header_end + length, 0);
        stream.read_exact(&mut request[header_end..])?;
        Ok(request)
    }

    /// 1.2.2's regression test for a fixture socket accepted in nonblocking
    /// mode (BSD sockets inherit the listener's): read_request must switch
    /// it back to blocking and read a request that arrives in fragments.
    #[test]
    fn fixture_reads_a_fragmented_request_after_a_nonblocking_accept() {
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        // Emulate BSD accepted-socket inheritance even on Linux.
        stream.set_nonblocking(true).unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let reader = std::thread::spawn(move || read_request(&mut stream, 16_384));
        for fragment in [
            &b"POST / HTTP/1.1\r\nContent-Len"[..],
            &b"gth: 4\r\n\r"[..],
            &b"\n"[..],
            &b"\0\xff"[..],
            &b"\x01\x80"[..],
        ] {
            std::thread::sleep(Duration::from_millis(50));
            client.write_all(fragment).unwrap();
        }
        let request = reader.join().unwrap().unwrap();
        assert_eq!(
            request,
            b"POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\n\0\xff\x01\x80"
        );
    }

    // The listener is loopback-only; release verification runs these tests in
    // the private no-egress NAS container, never against a provider or desktop.
    fn capture_request(endpoint_suffix: &str, translate: bool) -> String {
        use std::io::Write;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + FIXTURE_TIMEOUT;
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
            let request = read_request(&mut stream, 16_384).expect("fixture request");
            assert!(request.len() < 16_384);
            let response = r#"{"text":" fixture transcript "}"#;
            // The client may already have gone; its error is the failure.
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response);
            String::from_utf8_lossy(&request).into_owned()
        });
        let config = WhisperConfig {
            remote_endpoint: Some(format!("http://{address}{endpoint_suffix}")),
            remote_model: Some("fixture-model".into()),
            remote_api_key: Some("fixture-not-a-secret".into()),
            language: LanguageConfig::Single("fr".into()),
            initial_prompt: Some("Technical discussion.".into()),
            remote_timeout_secs: Some(FIXTURE_TIMEOUT.as_secs()),
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

    /// A loopback server that answers each request with the next scripted
    /// (status, body) and reports the `response_format` each one asked for.
    /// No provider is ever contacted.
    fn scripted(
        replies: Vec<(u16, &'static str)>,
    ) -> (WhisperConfig, std::thread::JoinHandle<Vec<String>>) {
        use std::io::Write;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let mut formats = Vec::new();
            for (status, body) in replies {
                let deadline = std::time::Instant::now() + FIXTURE_TIMEOUT;
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() > deadline {
                                return formats;
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("fixture accept failed: {e}"),
                    }
                };
                // A client that hung up mid-request gets no reply and is not
                // recorded; the test fails on the client's error instead.
                let Ok(request) = read_request(&mut stream, 16_384) else {
                    continue;
                };
                let text = String::from_utf8_lossy(&request);
                let marker = "name=\"response_format\"\r\n\r\n";
                let at = text.find(marker).unwrap() + marker.len();
                formats.push(text[at..].split("\r\n").next().unwrap().to_string());
                let reason = if status == 200 { "OK" } else { "Error" };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
            formats
        });
        let config = WhisperConfig {
            remote_endpoint: Some(format!("http://{address}")),
            remote_timeout_secs: Some(FIXTURE_TIMEOUT.as_secs()),
            ..Default::default()
        };
        (config, server)
    }

    const VERBOSE: &str = r#"{"text":"We ship. Agreed.","segments":[
        {"id":0,"start":0.0,"end":4.5,"text":" We ship."},
        {"id":1,"start":4.5,"end":13.0,"text":" Agreed."},
        {"id":2,"start":13.0,"end":14.0,"text":" ..."},
        {"id":3,"start":28.0,"end":31.0,"text":" Bye."}]}"#;
    const PLAIN: &str = r#"{"text":" We ship. Agreed. "}"#;

    fn spans(segments: &[TimedSegment]) -> Vec<(&str, f32, f32)> {
        segments
            .iter()
            .map(|s| (s.text.as_str(), s.start_secs, s.end_secs))
            .collect()
    }

    #[test]
    fn verbose_json_segments_are_used_and_remembered() {
        let (config, server) = scripted(vec![(200, VERBOSE), (200, VERBOSE)]);
        let t = RemoteTranscriber::new(&config).unwrap();
        let audio = vec![0.0f32; 30 * 16000];
        let got = t.transcribe_timed(&audio).unwrap();
        assert_eq!(
            spans(&got),
            vec![
                ("We ship.", 0.0, 4.5),
                ("Agreed.", 4.5, 13.0),
                // Clamped to the 30 s of audio; "..." has no words.
                ("Bye.", 28.0, 30.0),
            ]
        );
        assert_eq!(t.transcribe_timed(&audio).unwrap().len(), 3);
        assert_eq!(server.join().unwrap(), vec!["verbose_json", "verbose_json"]);
        assert_eq!(t.verbose_supported(), Some(true));
    }

    #[test]
    fn a_server_that_rejects_verbose_json_gets_plain_json_from_then_on() {
        let (config, server) = scripted(vec![
            (400, r#"{"error":"unsupported response_format"}"#),
            (200, PLAIN),
            (200, PLAIN),
        ]);
        let t = RemoteTranscriber::new(&config).unwrap();
        let audio = vec![0.0f32; 16000 * 10];
        assert_eq!(
            spans(&t.transcribe_timed(&audio).unwrap()),
            vec![("We ship. Agreed.", 0.0, 10.0)]
        );
        assert_eq!(t.transcribe_timed(&audio).unwrap().len(), 1);
        assert_eq!(server.join().unwrap(), vec!["verbose_json", "json", "json"]);
    }

    #[test]
    fn a_server_that_ignores_verbose_json_is_not_asked_again() {
        let (config, server) = scripted(vec![(200, PLAIN), (200, PLAIN)]);
        let t = RemoteTranscriber::new(&config).unwrap();
        let audio = vec![0.0f32; 16000 * 10];
        assert_eq!(t.transcribe_timed(&audio).unwrap().len(), 1);
        assert_eq!(t.transcribe_timed(&audio).unwrap().len(), 1);
        // One request each: the ignored format cost nothing extra.
        assert_eq!(server.join().unwrap(), vec!["verbose_json", "json"]);
    }

    #[test]
    fn failures_that_are_not_about_the_format_are_not_remembered() {
        let (config, server) = scripted(vec![
            (500, r#"{"error":"busy"}"#),
            (401, r#"{"error":"bad key"}"#),
            (401, r#"{"error":"bad key"}"#),
            (200, VERBOSE),
        ]);
        let t = RemoteTranscriber::new(&config).unwrap();
        let audio = vec![0.0f32; 16000 * 30];
        assert!(t.transcribe_timed(&audio).is_err(), "5xx is an error");
        assert_eq!(t.verbose_supported(), None);
        assert!(t.transcribe_timed(&audio).is_err(), "bad key fails both");
        assert_eq!(t.verbose_supported(), None);
        assert_eq!(t.transcribe_timed(&audio).unwrap().len(), 3);
        assert_eq!(
            server.join().unwrap(),
            vec!["verbose_json", "verbose_json", "json", "verbose_json"]
        );
    }

    #[test]
    fn dictation_still_asks_for_plain_json() {
        let (config, server) = scripted(vec![(200, PLAIN)]);
        let t = RemoteTranscriber::new(&config).unwrap();
        assert_eq!(t.transcribe(&[0.0; 16000]).unwrap(), "We ship. Agreed.");
        assert_eq!(server.join().unwrap(), vec!["json"]);
    }

    /// A meeting chunk through a remote server with segments: one request,
    /// a line per segment, each fitted to its speech like local Whisper's,
    /// and a segment over silence dropped.
    #[test]
    fn remote_meeting_chunks_get_fitted_lines() {
        use crate::meeting::chunk::{ChunkConfig, ChunkProcessor};
        use crate::meeting::data::AudioSource;

        let (config, server) = scripted(vec![(200, VERBOSE)]);
        let t = std::sync::Arc::new(RemoteTranscriber::new(&config).unwrap());
        let mut audio = vec![0.0f32; 30 * 16000];
        for (start, end) in [(2.0f32, 4.0f32), (10.0, 12.5)] {
            let span = (start * 16000.0) as usize..(end * 16000.0) as usize;
            for (i, sample) in audio[span.clone()].iter_mut().enumerate() {
                let t = (span.start + i) as f32 / 16000.0;
                *sample = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            }
        }
        let mut processor = ChunkProcessor::new(ChunkConfig::default(), t);
        let mut buffer = processor.new_buffer(1, AudioSource::Loopback, 30_000);
        buffer.add_samples(&audio);
        let segments = processor.process_chunk(buffer).unwrap().segments;
        // Fitted to 2-4 s and 10-12.5 s plus the 200 ms pad (within one
        // 30 ms analysis frame), offset by 30 s; "Bye." at 28-30 s is over
        // silence and dropped.
        let texts: Vec<&str> = segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["We ship.", "Agreed."], "{segments:?}");
        for (seg, (start, end)) in segments.iter().zip([(31_800, 34_200), (39_800, 42_700)]) {
            assert!(
                seg.start_ms.abs_diff(start) <= 30 && seg.end_ms.abs_diff(end) <= 30,
                "{segments:?}"
            );
        }
        assert_eq!(server.join().unwrap(), vec!["verbose_json"]);
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
