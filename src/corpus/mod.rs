//! Corpus capture for post-processing training.
//!
//! Writes push-to-talk sessions to a flat directory as
//! `(audio.wav, raw.txt, [processed.txt,] [post.txt,] meta.json)` tuples.

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CorpusError {
    #[error("Corpus IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Corpus WAV encode error: {0}")]
    Wav(#[from] hound::Error),

    #[error("Corpus metadata serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Corpus capture configuration. `path` is the directory that artifacts will
/// be written to; callers are expected to resolve `"auto"` and any tilde
/// expansion before constructing this, but both absolute and relative paths
/// are accepted as-is.
#[derive(Debug, Clone)]
pub struct CorpusConfig {
    pub path: PathBuf,
}

/// Writes corpus artifacts to a base directory.
pub struct CorpusWriter {
    base_dir: PathBuf,
}

impl CorpusWriter {
    /// Open (creating the directory if missing) a corpus writer at the given path.
    pub fn open(config: CorpusConfig) -> Result<Self, CorpusError> {
        std::fs::create_dir_all(&config.path)?;
        Ok(Self { base_dir: config.path })
    }

    pub fn base_dir(&self) -> &std::path::Path {
        &self.base_dir
    }
}

/// Format a timestamp + 4-hex suffix into a filesystem-safe stem.
/// Example: `2026-04-20T14-32-05_a7f3`
fn session_stem(dt: DateTime<Local>, hex: &str) -> String {
    // RFC 3339 second-precision with `:` → `-` for filesystem friendliness.
    let ts = dt.format("%Y-%m-%dT%H-%M-%S").to_string();
    format!("{ts}_{hex}")
}

/// Encode f32 samples as int16 mono PCM WAV into an already-opened file.
/// The file is consumed so that callers can atomically claim the path via
/// `OpenOptions::create_new` before writing.
fn write_wav(file: std::fs::File, samples: &[f32], sample_rate: u32) -> Result<(), CorpusError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(std::io::BufWriter::new(file), spec)?;
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let sample_i16 = (clamped * i16::MAX as f32) as i16;
        writer.write_sample(sample_i16)?;
    }
    writer.finalize()?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextStages {
    pub raw: bool,
    pub processed: bool,
    pub post: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSidecar {
    pub id: String,
    pub recorded_at: DateTime<Local>,
    pub duration_secs: f32,
    pub sample_rate: u32,
    pub engine: String,
    pub model: String,
    pub language: Option<String>,
    pub profile: Option<String>,
    pub post_process_command: Option<String>,
    pub voxtype_version: String,
    pub text_stages: TextStages,
}

/// A complete recording session ready to be persisted.
pub struct CorpusSession {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub raw_text: String,
    pub processed_text: String,
    pub post_text: Option<String>,
    pub engine: String,
    pub model: String,
    pub language: Option<String>,
    pub profile: Option<String>,
    pub post_process_command: Option<String>,
    pub duration_secs: f32,
    pub recorded_at: DateTime<Local>,
}

impl CorpusWriter {
    /// Persist a session. Returns the session id (filename stem) on success.
    ///
    /// Synchronous — callers should run this on `tokio::task::spawn_blocking`.
    pub fn save(&self, session: CorpusSession) -> Result<String, CorpusError> {
        // Atomically claim a unique stem by creating its `.wav` file with
        // `create_new` (fails on AlreadyExists). Retry up to 3 times with a
        // fresh hex suffix when two saves in the same clock second collide.
        let (stem, wav_file) = {
            let mut last_err: Option<std::io::Error> = None;
            let mut claimed: Option<(String, std::fs::File)> = None;
            for _ in 0..3 {
                let candidate = session_stem(session.recorded_at, &random_hex4());
                let wav_path = self.base_dir.join(format!("{candidate}.wav"));
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&wav_path)
                {
                    Ok(f) => {
                        claimed = Some((candidate, f));
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        last_err = Some(e);
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            match claimed {
                Some(pair) => pair,
                None => {
                    tracing::warn!("Corpus: 3 filename collisions; skipping session");
                    return Err(CorpusError::Io(last_err.unwrap_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::AlreadyExists, "stem collision")
                    })));
                }
            }
        };

        write_wav(wav_file, &session.samples, session.sample_rate)?;

        let raw_path = self.base_dir.join(format!("{stem}.raw.txt"));
        std::fs::write(&raw_path, &session.raw_text)?;

        let processed_written = session.processed_text != session.raw_text;
        if processed_written {
            let p = self.base_dir.join(format!("{stem}.processed.txt"));
            std::fs::write(&p, &session.processed_text)?;
        }

        let post_written = session.post_text.is_some();
        if let Some(ref post) = session.post_text {
            let p = self.base_dir.join(format!("{stem}.post.txt"));
            std::fs::write(&p, post)?;
        }

        let sidecar = SessionSidecar {
            id: stem.clone(),
            recorded_at: session.recorded_at,
            duration_secs: session.duration_secs,
            sample_rate: session.sample_rate,
            engine: session.engine,
            model: session.model,
            language: session.language,
            profile: session.profile,
            post_process_command: session.post_process_command,
            voxtype_version: env!("CARGO_PKG_VERSION").to_string(),
            text_stages: TextStages {
                raw: true,
                processed: processed_written,
                post: post_written,
            },
        };
        let json_path = self.base_dir.join(format!("{stem}.json"));
        let json = serde_json::to_string_pretty(&sidecar)?;
        std::fs::write(&json_path, json)?;

        Ok(stem)
    }
}

/// Generate a 4-character random hex suffix (uses uuid v4 for entropy).
fn random_hex4() -> String {
    let u = uuid::Uuid::new_v4();
    // Take the first 4 hex chars of the simple representation.
    u.simple().to_string()[..4].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_base_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("nested").join("corpus");
        let cfg = CorpusConfig { path: target.clone() };
        let writer = CorpusWriter::open(cfg).expect("open should succeed");
        assert!(target.exists());
        assert_eq!(writer.base_dir(), target);
    }

    #[test]
    fn stem_has_expected_shape() {
        use chrono::{Local, TimeZone};
        let dt = Local.with_ymd_and_hms(2026, 4, 20, 14, 32, 5).unwrap();
        let stem = session_stem(dt, "a7f3");
        assert_eq!(stem, "2026-04-20T14-32-05_a7f3");
    }

    #[test]
    fn random_hex_is_four_chars() {
        let hex = random_hex4();
        assert_eq!(hex.len(), 4);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn distinct_sessions_have_distinct_stems() {
        use chrono::Local;
        let now = Local::now();
        let a = session_stem(now, &random_hex4());
        let b = session_stem(now, &random_hex4());
        // Extremely unlikely to collide on a 16-bit random suffix within one test run.
        assert_ne!(a, b);
    }

    #[test]
    fn sidecar_serializes_with_all_fields() {
        use chrono::{Local, TimeZone};
        let dt = Local.with_ymd_and_hms(2026, 4, 20, 14, 32, 5).unwrap();
        let sidecar = SessionSidecar {
            id: "2026-04-20T14-32-05_a7f3".to_string(),
            recorded_at: dt,
            duration_secs: 4.73,
            sample_rate: 16_000,
            engine: "whisper".to_string(),
            model: "ggml-large-v3-q8_0".to_string(),
            language: Some("hu".to_string()),
            profile: Some("translate".to_string()),
            post_process_command: Some("openrouter-translate".to_string()),
            voxtype_version: env!("CARGO_PKG_VERSION").to_string(),
            text_stages: TextStages { raw: true, processed: false, post: true },
        };
        let s = serde_json::to_string(&sidecar).unwrap();
        assert!(s.contains("\"id\":\"2026-04-20T14-32-05_a7f3\""));
        assert!(s.contains("\"language\":\"hu\""));
        assert!(s.contains("\"text_stages\":{"));
        assert!(s.contains("\"processed\":false"));
        // Round-trip parse
        let parsed: SessionSidecar = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.id, "2026-04-20T14-32-05_a7f3");
        assert_eq!(parsed.language.as_deref(), Some("hu"));
        assert!(!parsed.text_stages.processed);
    }

    #[test]
    fn sidecar_serializes_with_null_optionals() {
        use chrono::Local;
        let sidecar = SessionSidecar {
            id: "x".to_string(),
            recorded_at: Local::now(),
            duration_secs: 1.0,
            sample_rate: 16_000,
            engine: "whisper".to_string(),
            model: "tiny".to_string(),
            language: None,
            profile: None,
            post_process_command: None,
            voxtype_version: "0.0.0".to_string(),
            text_stages: TextStages { raw: true, processed: false, post: false },
        };
        let s = serde_json::to_string(&sidecar).unwrap();
        assert!(s.contains("\"language\":null"));
        assert!(s.contains("\"post_process_command\":null"));
    }

    #[test]
    fn wav_roundtrip_int16_16khz() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("out.wav");

        // 1 second of a silent ramp, 16 kHz mono f32 in [-1.0, 1.0).
        let samples: Vec<f32> = (0..16_000).map(|i| (i as f32) / 16_000.0 - 0.5).collect();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("wav file create");
        write_wav(file, &samples, 16_000).expect("wav write");

        let mut reader = hound::WavReader::open(&path).expect("wav open");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);
        let out: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(out.len(), 16_000);
    }

    fn sample_session(raw: &str, processed: &str, post: Option<&str>) -> CorpusSession {
        use chrono::{Local, TimeZone};
        CorpusSession {
            samples: vec![0.0; 16_000], // 1 second of silence
            sample_rate: 16_000,
            raw_text: raw.to_string(),
            processed_text: processed.to_string(),
            post_text: post.map(String::from),
            engine: "whisper".to_string(),
            model: "tiny".to_string(),
            language: Some("en".to_string()),
            profile: None,
            post_process_command: post.map(|_| "my-llm".to_string()),
            duration_secs: 1.0,
            recorded_at: Local.with_ymd_and_hms(2026, 4, 20, 14, 32, 5).unwrap(),
        }
    }

    #[test]
    fn save_writes_full_quadruplet() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = CorpusWriter::open(CorpusConfig {
            path: tmp.path().to_path_buf(),
        }).unwrap();

        let session = sample_session("hello world", "Hello, world.", Some("Hello, world!"));
        let id = writer.save(session).expect("save");

        assert!(tmp.path().join(format!("{id}.wav")).exists());
        assert!(tmp.path().join(format!("{id}.raw.txt")).exists());
        assert!(tmp.path().join(format!("{id}.processed.txt")).exists());
        assert!(tmp.path().join(format!("{id}.post.txt")).exists());
        assert!(tmp.path().join(format!("{id}.json")).exists());

        let raw = std::fs::read_to_string(tmp.path().join(format!("{id}.raw.txt"))).unwrap();
        assert_eq!(raw, "hello world");
        let post = std::fs::read_to_string(tmp.path().join(format!("{id}.post.txt"))).unwrap();
        assert_eq!(post, "Hello, world!");
    }

    #[test]
    fn save_elides_processed_when_equal_to_raw() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = CorpusWriter::open(CorpusConfig {
            path: tmp.path().to_path_buf(),
        }).unwrap();

        let session = sample_session("same", "same", Some("different"));
        let id = writer.save(session).unwrap();

        assert!(!tmp.path().join(format!("{id}.processed.txt")).exists());
        assert!(tmp.path().join(format!("{id}.post.txt")).exists());

        let json = std::fs::read_to_string(tmp.path().join(format!("{id}.json"))).unwrap();
        let parsed: SessionSidecar = serde_json::from_str(&json).unwrap();
        assert!(!parsed.text_stages.processed);
        assert!(parsed.text_stages.post);
    }

    #[test]
    fn save_elides_post_when_none() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = CorpusWriter::open(CorpusConfig {
            path: tmp.path().to_path_buf(),
        }).unwrap();

        let session = sample_session("raw", "processed", None);
        let id = writer.save(session).unwrap();

        assert!(!tmp.path().join(format!("{id}.post.txt")).exists());
        assert!(tmp.path().join(format!("{id}.processed.txt")).exists());

        let json = std::fs::read_to_string(tmp.path().join(format!("{id}.json"))).unwrap();
        let parsed: SessionSidecar = serde_json::from_str(&json).unwrap();
        assert!(!parsed.text_stages.post);
        assert!(parsed.post_process_command.is_none());
    }
}
