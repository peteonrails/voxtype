//! Grok Speech-to-Text (`POST https://api.x.ai/v1/stt`). Batch after PTT.

use super::xai_oauth;
use super::Transcriber;
use crate::config::XaiConfig;
use crate::error::TranscribeError;
use std::io::Cursor;
use std::time::Duration;
use ureq::serde_json;

const DEFAULT_ENDPOINT: &str = "https://api.x.ai/v1/stt";

pub struct XaiTranscriber {
    endpoint: String,
    language: Option<String>,
    format: bool,
    timeout: Duration,
    api_key: Option<String>,
}

impl XaiTranscriber {
    pub fn new(config: &XaiConfig) -> Result<Self, TranscribeError> {
        let api_key = explicit_api_key(config.api_key.as_deref());
        if api_key.is_none() && !xai_oauth::is_logged_in() {
            return Err(TranscribeError::ConfigError(
                "xAI engine needs credentials. Set [xai] api_key, VOXTYPE_XAI_API_KEY / XAI_API_KEY, \
                 or run: voxtype setup xai --login"
                    .into(),
            ));
        }

        let endpoint = config
            .endpoint
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_ENDPOINT)
            .trim_end_matches('/')
            .to_string();
        if !endpoint.starts_with("https://") {
            return Err(TranscribeError::ConfigError(format!(
                "xAI endpoint must be https, got: {endpoint}"
            )));
        }

        let language = config
            .language
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("auto"))
            .map(str::to_string);

        tracing::info!(
            "Configured xAI STT: endpoint={endpoint}, language={}, auth={}",
            language.as_deref().unwrap_or("auto"),
            if api_key.is_some() {
                "api_key"
            } else {
                "oauth"
            }
        );

        Ok(Self {
            endpoint,
            language,
            format: config.format,
            timeout: Duration::from_secs(config.timeout_secs.unwrap_or(120).max(5)),
            api_key,
        })
    }

    fn bearer(&self) -> Result<String, TranscribeError> {
        if let Some(ref k) = self.api_key {
            return Ok(k.clone());
        }
        xai_oauth::access_token().map_err(|e| TranscribeError::ConfigError(e.to_string()))
    }

    fn encode_wav(samples: &[f32]) -> Result<Vec<u8>, TranscribeError> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut buffer, spec).map_err(|e| {
            TranscribeError::AudioFormat(format!("Failed to create WAV writer: {e}"))
        })?;
        for &sample in samples {
            let scaled = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(scaled).map_err(|e| {
                TranscribeError::AudioFormat(format!("Failed to write sample: {e}"))
            })?;
        }
        writer
            .finalize()
            .map_err(|e| TranscribeError::AudioFormat(format!("Failed to finalize WAV: {e}")))?;
        Ok(buffer.into_inner())
    }

    fn build_multipart(&self, wav: &[u8]) -> (String, Vec<u8>) {
        let boundary = format!(
            "----VoxtypeXai{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let mut body = Vec::new();
        if let Some(ref lang) = self.language {
            push_text_field(&mut body, &boundary, "language", lang);
            if self.format {
                push_text_field(&mut body, &boundary, "format", "true");
            }
        }
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: audio/wav\r\n\r\n");
        body.extend_from_slice(wav);
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        (boundary, body)
    }

    fn post(
        &self,
        bearer: &str,
        boundary: &str,
        body: &[u8],
    ) -> Result<ureq::Response, Box<ureq::Error>> {
        ureq::post(&self.endpoint)
            .timeout(self.timeout)
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .set("Authorization", &format!("Bearer {bearer}"))
            .set("User-Agent", "voxtype")
            .send_bytes(body)
            .map_err(Box::new)
    }
}

fn explicit_api_key(from_config: Option<&str>) -> Option<String> {
    if let Some(k) = from_config.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(k.to_string());
    }
    for var in ["VOXTYPE_XAI_API_KEY", "XAI_API_KEY"] {
        if let Ok(k) = std::env::var(var) {
            let k = k.trim().to_string();
            if !k.is_empty() {
                return Some(k);
            }
        }
    }
    None
}

fn push_text_field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(value.as_bytes());
    body.extend_from_slice(b"\r\n");
}

impl Transcriber for XaiTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".into()));
        }
        let start = std::time::Instant::now();
        let wav = Self::encode_wav(samples)?;
        let (boundary, body) = self.build_multipart(&wav);
        let mut bearer = self.bearer()?;
        let mut response = self.post(&bearer, &boundary, &body);
        if let Err(e) = &response {
            if let ureq::Error::Status(code, _) = e.as_ref() {
                if *code == 401 && self.api_key.is_none() {
                    if let Ok(tok) = xai_oauth::force_refresh() {
                        bearer = tok;
                        response = self.post(&bearer, &boundary, &body);
                    }
                }
            }
        }
        let response = response.map_err(|e| match *e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                TranscribeError::RemoteError(format!("xAI STT HTTP {code}: {body}"))
            }
            ureq::Error::Transport(t) => {
                TranscribeError::NetworkError(format!("xAI STT request failed: {t}"))
            }
        })?;
        let json: serde_json::Value = response.into_json().map_err(|e| {
            TranscribeError::RemoteError(format!("Failed to parse xAI STT response: {e}"))
        })?;
        let text = json
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TranscribeError::RemoteError(format!("xAI STT response missing 'text': {json}"))
            })?
            .trim()
            .to_string();
        tracing::info!(
            "xAI transcription completed in {:.2}s ({} chars)",
            start.elapsed().as_secs_f32(),
            text.chars().count()
        );
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_has_riff_header() {
        let wav = XaiTranscriber::encode_wav(&[0.1_f32; 160]).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn autodetect_omits_format_field() {
        let t = XaiTranscriber {
            endpoint: DEFAULT_ENDPOINT.into(),
            language: None,
            format: true,
            timeout: Duration::from_secs(5),
            api_key: Some("test".into()),
        };
        let (_b, body) = t.build_multipart(b"WAV");
        let s = String::from_utf8_lossy(&body);
        assert!(!s.contains("name=\"format\""));
        assert!(!s.contains("name=\"language\""));
        assert!(s.contains("name=\"file\""));
    }

    #[test]
    fn language_includes_format_before_file() {
        let t = XaiTranscriber {
            endpoint: DEFAULT_ENDPOINT.into(),
            language: Some("en".into()),
            format: true,
            timeout: Duration::from_secs(5),
            api_key: Some("test".into()),
        };
        let (_b, body) = t.build_multipart(b"WAV");
        let s = String::from_utf8_lossy(&body);
        let lang = s.find("name=\"language\"").unwrap();
        let format = s.find("name=\"format\"").unwrap();
        let file = s.find("name=\"file\"").unwrap();
        assert!(lang < format && format < file, "{s}");
    }

    #[test]
    fn rejects_http_endpoint() {
        let cfg = XaiConfig {
            api_key: Some("xai-test".into()),
            endpoint: Some("http://127.0.0.1/stt".into()),
            ..XaiConfig::default()
        };
        let msg = match XaiTranscriber::new(&cfg) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("http endpoint should fail"),
        };
        assert!(msg.contains("https"), "{msg}");
    }

    #[test]
    fn rejects_missing_credentials() {
        let prev_data = std::env::var_os("VOXTYPE_DATA_DIR");
        let prev_a = std::env::var_os("VOXTYPE_XAI_API_KEY");
        let prev_b = std::env::var_os("XAI_API_KEY");
        let dir = std::env::temp_dir().join(format!("voxtype-xai-nocred-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("VOXTYPE_DATA_DIR", &dir);
        std::env::remove_var("VOXTYPE_XAI_API_KEY");
        std::env::remove_var("XAI_API_KEY");
        let err = match XaiTranscriber::new(&XaiConfig::default()) {
            Err(e) => e,
            Ok(_) => panic!("missing credentials should fail"),
        };
        match prev_data {
            Some(v) => std::env::set_var("VOXTYPE_DATA_DIR", v),
            None => std::env::remove_var("VOXTYPE_DATA_DIR"),
        }
        if let Some(v) = prev_a {
            std::env::set_var("VOXTYPE_XAI_API_KEY", v);
        }
        if let Some(v) = prev_b {
            std::env::set_var("XAI_API_KEY", v);
        }
        let msg = err.to_string();
        assert!(
            msg.contains("credentials") || msg.contains("API key"),
            "{msg}"
        );
    }
}
