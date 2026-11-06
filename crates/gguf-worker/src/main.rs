//! `voxtype-gguf-worker` — out-of-process host for transcribe.cpp GGUF models.
//!
//! ## Why this is its own crate
//!
//! transcribe.cpp and whisper-rs each vendor a copy of ggml, and the two export
//! the same 644 global symbols. Linked into one executable the linker resolves
//! each name once, so one copy serves both. Measured on rc/1.2.0, transcribe.cpp's
//! ggml 0.20.2 won outright and whisper.cpp, compiled against 0.9.5, was running
//! against it.
//!
//! A separate binary in the voxtype package is not enough: cargo features are
//! package-wide, so transcribe-cpp-sys's link directives reach every binary in
//! the package whether or not it holds GGUF code. The dependency has to leave
//! voxtype's graph entirely, so it lives here and voxtype talks to this process
//! over a pipe.
//!
//! ## Protocol
//!
//! 1. Spawned as `voxtype-gguf-worker --engine <cohere|parakeet> --config-json <json>`,
//!    where the JSON is that engine's config section serialized by voxtype.
//! 2. Loads the model and writes `READY\n` to stdout.
//! 3. Reads a `u32` little-endian sample count, then that many little-endian
//!    `f32` samples at 16 kHz mono.
//! 4. Writes one JSON line: `{"ok":true,"text":"..."}` or `{"ok":false,"error":"..."}`.
//! 5. Repeats 3-4 until stdin closes. The model stays resident across requests.
//!
//! stdout carries the protocol and nothing else; logging goes to stderr.

mod cohere;
mod config;
mod error;
mod gguf;
mod parakeet;

// Shared with voxtype's ONNX engines: one source file, compiled into both. Each
// imports only `crate::config::CohereConfig` and `crate::error::TranscribeError`,
// which this crate mirrors, so no copy of the logic can drift.
#[path = "../../../src/transcribe/cohere_chunking.rs"]
mod cohere_chunking;
#[path = "../../../src/transcribe/window.rs"]
mod window;

use std::io::{Read, Write};

use config::{CohereConfig, ParakeetConfig};
use error::TranscribeError;

/// One loaded GGUF model, ready to transcribe 16 kHz mono audio.
pub(crate) trait Transcriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError>;
}

const READY_SIGNAL: &str = "READY";

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn load(engine: &str, config_json: &str) -> Result<Box<dyn Transcriber>, TranscribeError> {
    let bad_config = |e: serde_json::Error| {
        TranscribeError::ConfigError(format!("bad [{engine}] config from voxtype: {e}"))
    };
    match engine {
        "cohere" => {
            let config: CohereConfig = serde_json::from_str(config_json).map_err(bad_config)?;
            Ok(Box::new(cohere::CohereGgufTranscriber::new(&config)?))
        }
        "parakeet" => {
            let config: ParakeetConfig = serde_json::from_str(config_json).map_err(bad_config)?;
            Ok(Box::new(parakeet::ParakeetGgufTranscriber::new(&config)?))
        }
        other => Err(TranscribeError::ConfigError(format!(
            "unknown engine '{other}'; expected cohere or parakeet"
        ))),
    }
}

fn failure(e: &TranscribeError) -> serde_json::Value {
    serde_json::json!({ "ok": false, "kind": e.kind(), "error": e.to_string() })
}

fn main() {
    // stderr is inherited from voxtype, so these lines land in the daemon's log
    // alongside its own. stdout is reserved for the protocol.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(engine), Some(config_json)) = (arg(&args, "--engine"), arg(&args, "--config-json"))
    else {
        eprintln!("[gguf-worker] usage: voxtype-gguf-worker --engine <cohere|parakeet> --config-json <json>");
        std::process::exit(2);
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let transcriber = match load(&engine, &config_json) {
        Ok(t) => t,
        Err(e) => {
            // In place of READY, so voxtype can report the real cause.
            eprintln!("[gguf-worker] {engine} model load failed: {e}");
            let _ = writeln!(out, "{}", failure(&e)).and_then(|()| out.flush());
            std::process::exit(1);
        }
    };
    if writeln!(out, "{READY_SIGNAL}")
        .and_then(|()| out.flush())
        .is_err()
    {
        std::process::exit(1);
    }
    eprintln!("[gguf-worker] {engine} ready");

    let stdin = std::io::stdin();
    let mut input = stdin.lock();

    loop {
        let mut len_buf = [0u8; 4];
        match input.read_exact(&mut len_buf) {
            Ok(()) => {}
            // Clean shutdown: the parent closed stdin.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                eprintln!("[gguf-worker] read error: {e}");
                break;
            }
        }

        let count = u32::from_le_bytes(len_buf) as usize;
        let mut bytes = vec![0u8; count * std::mem::size_of::<f32>()];
        if let Err(e) = input.read_exact(&mut bytes) {
            eprintln!("[gguf-worker] short audio read: {e}");
            break;
        }
        let samples: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();

        let response = match transcriber.transcribe(&samples) {
            Ok(text) => serde_json::json!({ "ok": true, "text": text }),
            Err(e) => {
                eprintln!("[gguf-worker] transcription failed: {e}");
                failure(&e)
            }
        };
        if writeln!(out, "{response}")
            .and_then(|()| out.flush())
            .is_err()
        {
            break;
        }
    }

    eprintln!("[gguf-worker] exiting");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_unknown_engine() {
        let err = load("whisper", "{}").err().unwrap();
        assert_eq!(err.kind(), "config_error");
        assert!(
            err.to_string().contains("unknown engine 'whisper'"),
            "{err}"
        );
    }

    #[test]
    fn rejects_a_config_that_does_not_parse() {
        let err = load("parakeet", "not json").err().unwrap();
        assert_eq!(err.kind(), "config_error");
        assert!(err.to_string().contains("bad [parakeet] config"), "{err}");
    }

    #[test]
    fn a_missing_model_travels_as_model_not_found() {
        let json = r#"{"model":"/nonexistent/voxtype/parakeet.gguf"}"#;
        let err = load("parakeet", json).err().unwrap();
        let line = failure(&err);
        assert_eq!(line["ok"], false);
        assert_eq!(line["kind"], "model_not_found");
        assert!(line["error"]
            .as_str()
            .unwrap()
            .contains("/nonexistent/voxtype/parakeet.gguf"));
    }
}
