//! Transcription worker process for GPU isolation
//!
//! This module implements a subprocess that handles transcription in isolation.
//! When `gpu_isolation = true`, the daemon spawns this worker for each
//! transcription, ensuring the GPU is fully released after transcription
//! completes (the process exits, releasing all GPU resources).
//!
//! Protocol (eager mode - subprocess spawned when recording starts):
//! 1. Worker starts, loads model
//! 2. Worker writes "READY\n" to stdout (signals model is loaded)
//! 3. Parent sends audio via stdin: [u32 sample_count (LE)][f32 samples (LE)...]
//!    The top bit of the count ([`TIMED_FLAG`]) asks for timed segments.
//! 4. Worker transcribes and writes JSON response to stdout
//! 5. Worker exits
//!
//! The key benefit: model loading happens while the user is speaking,
//! so perceived latency is just the transcription time.

use crate::config::WhisperConfig;
use crate::transcribe::Transcriber;
use std::io::{self, Read, Write};
use ureq::serde_json;

/// Ready signal sent after model is loaded
pub const READY_SIGNAL: &str = "READY";

/// Set in the sample-count word to ask for [`Transcriber::transcribe_timed`]
/// instead of `transcribe`. Counts are capped far below it, and the worker is
/// always the parent's own binary, so the two ends agree on the format.
pub const TIMED_FLAG: u32 = 1 << 31;

/// JSON response from the worker
#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub enum WorkerResponse {
    Success {
        ok: bool,
        text: String,
        /// Two-letter language code chosen for this transcription, if the
        /// worker tracked one. The parent reads this so layout-aware output
        /// methods (eitype, dotool) can switch keyboard layouts. Older
        /// parents that ignore the field simply skip the hint.
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        /// Whisper's timed segments, present when the parent asked for them
        #[serde(skip_serializing_if = "Option::is_none")]
        segments: Option<Vec<crate::transcribe::TimedSegment>>,
    },
    Error {
        ok: bool,
        error: String,
    },
}

impl WorkerResponse {
    pub fn success(text: String, language: Option<String>) -> Self {
        WorkerResponse::Success {
            ok: true,
            text,
            language,
            segments: None,
        }
    }

    pub fn timed(segments: Vec<crate::transcribe::TimedSegment>, language: Option<String>) -> Self {
        let text = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        WorkerResponse::Success {
            ok: true,
            text,
            language,
            segments: Some(segments),
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        WorkerResponse::Error {
            ok: false,
            error: msg.into(),
        }
    }
}

/// Run the transcription worker
///
/// This is the main entry point called from `voxtype transcribe-worker`.
/// It loads the model FIRST, signals ready, then waits for audio.
pub fn run_worker(config: &WhisperConfig) -> anyhow::Result<()> {
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    // Step 1: Load model first (while user is speaking)
    eprintln!("[worker] Loading model: {}", config.model);
    let load_start = std::time::Instant::now();

    let transcriber = match super::whisper::WhisperTranscriber::new(config) {
        Ok(t) => t,
        Err(e) => {
            // Write error and exit - parent will see no READY signal
            write_response_to(
                &mut stdout_lock,
                WorkerResponse::error(format!("Failed to load model: {}", e)),
            );
            return Ok(());
        }
    };

    eprintln!(
        "[worker] Model loaded in {:.2}s",
        load_start.elapsed().as_secs_f32()
    );

    // Step 2: Signal ready (model is loaded, waiting for audio)
    writeln!(stdout_lock, "{}", READY_SIGNAL)?;
    stdout_lock.flush()?;
    eprintln!("[worker] Signaled READY, waiting for audio...");

    // Step 3: Read audio from stdin
    let stdin = io::stdin();
    let mut stdin = stdin.lock();

    // Read sample count (u32 little-endian)
    let mut count_buf = [0u8; 4];
    if let Err(e) = stdin.read_exact(&mut count_buf) {
        write_response_to(
            &mut stdout_lock,
            WorkerResponse::error(format!("Failed to read sample count: {}", e)),
        );
        return Ok(());
    }
    let count_word = u32::from_le_bytes(count_buf);
    let timed = count_word & TIMED_FLAG != 0;
    let sample_count = (count_word & !TIMED_FLAG) as usize;

    // Validate sample count (prevent OOM from malformed input)
    // Max 10 minutes at 16kHz = 9,600,000 samples = ~38MB
    const MAX_SAMPLES: usize = 16000 * 60 * 10;
    if sample_count > MAX_SAMPLES {
        write_response_to(
            &mut stdout_lock,
            WorkerResponse::error(format!(
                "Sample count too large: {} (max {})",
                sample_count, MAX_SAMPLES
            )),
        );
        return Ok(());
    }

    if sample_count == 0 {
        write_response_to(
            &mut stdout_lock,
            WorkerResponse::error("Empty audio buffer"),
        );
        return Ok(());
    }

    // Read samples (f32 little-endian)
    let mut samples = vec![0f32; sample_count];
    let samples_bytes = unsafe {
        std::slice::from_raw_parts_mut(
            samples.as_mut_ptr() as *mut u8,
            sample_count * std::mem::size_of::<f32>(),
        )
    };

    if let Err(e) = stdin.read_exact(samples_bytes) {
        write_response_to(
            &mut stdout_lock,
            WorkerResponse::error(format!("Failed to read audio samples: {}", e)),
        );
        return Ok(());
    }

    eprintln!(
        "[worker] Received {} samples ({:.2}s)",
        sample_count,
        sample_count as f32 / 16000.0
    );

    // Step 4: Transcribe
    eprintln!("[worker] Starting transcription...");
    let transcribe_start = std::time::Instant::now();
    if timed {
        match transcriber.transcribe_timed(&samples) {
            Ok(segments) => {
                eprintln!(
                    "[worker] Timed transcription complete in {:.2}s: {} segment(s)",
                    transcribe_start.elapsed().as_secs_f32(),
                    segments.len()
                );
                let language = transcriber.last_detected_language();
                write_response_to(&mut stdout_lock, WorkerResponse::timed(segments, language));
            }
            Err(e) => {
                eprintln!("[worker] Transcription failed: {}", e);
                write_response_to(&mut stdout_lock, WorkerResponse::error(e.to_string()));
            }
        }
        return Ok(());
    }

    let result = transcriber.transcribe(&samples);

    match result {
        Ok(text) => {
            eprintln!(
                "[worker] Transcription complete in {:.2}s: {} chars",
                transcribe_start.elapsed().as_secs_f32(),
                text.len()
            );
            // Capture the chosen language so the parent can hint output
            // methods (eitype --layout, dotool DOTOOL_XKB_LAYOUT) about
            // what keyboard layout to use. Field is omitted from the JSON
            // if no language was tracked.
            let language = transcriber.last_detected_language();
            write_response_to(&mut stdout_lock, WorkerResponse::success(text, language));
        }
        Err(e) => {
            eprintln!("[worker] Transcription failed: {}", e);
            write_response_to(&mut stdout_lock, WorkerResponse::error(e.to_string()));
        }
    }

    Ok(())
}

/// Write a JSON response to the given writer
fn write_response_to<W: Write>(writer: &mut W, response: WorkerResponse) {
    if let Ok(json) = serde_json::to_string(&response) {
        let _ = writeln!(writer, "{}", json);
        let _ = writer.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_response_serialization() {
        let success = WorkerResponse::success("Hello world".to_string(), None);
        let json = serde_json::to_string(&success).unwrap();
        assert!(json.contains(r#""ok":true"#));
        assert!(json.contains(r#""text":"Hello world""#));
        // No language present: serialized form omits the field entirely.
        assert!(!json.contains(r#""language""#));

        let error = WorkerResponse::error("Something went wrong");
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains(r#""ok":false"#));
        assert!(json.contains(r#""error":"Something went wrong""#));
    }

    #[test]
    fn test_worker_response_serialization_with_language() {
        let success = WorkerResponse::success("Привет".to_string(), Some("ru".to_string()));
        let json = serde_json::to_string(&success).unwrap();
        assert!(json.contains(r#""ok":true"#));
        assert!(json.contains(r#""language":"ru""#));
    }

    #[test]
    fn timed_responses_carry_segments_and_joined_text() {
        let seg = |text: &str, a: f32, b: f32| crate::transcribe::TimedSegment {
            text: text.to_string(),
            start_secs: a,
            end_secs: b,
        };
        let r = WorkerResponse::timed(vec![seg("One.", 0.0, 1.5), seg("Two.", 2.0, 3.0)], None);
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""text":"One. Two.""#), "{json}");
        assert!(
            json.contains(r#""segments":[{"text":"One.","start_secs":0.0,"end_secs":1.5}"#),
            "{json}"
        );
        assert!(TIMED_FLAG as usize > 16000 * 60 * 10);
    }

    #[test]
    fn test_ready_signal() {
        assert_eq!(READY_SIGNAL, "READY");
    }
}
