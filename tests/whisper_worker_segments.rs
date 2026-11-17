//! The GPU-isolation worker (`voxtype transcribe-worker`) returns Whisper's
//! timed segments when the count word carries the timed flag, so meetings
//! with `gpu_isolation = true` also get one line per utterance.
//!
//! Needs a real Whisper model, so it runs only when one is named:
//! `VOXTYPE_TEST_WHISPER_MODEL=/path/ggml-tiny.en.bin cargo test --test whisper_worker_segments`

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn speech() -> Vec<f32> {
    hound::WavReader::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/vad/speech_long.wav"
    ))
    .unwrap()
    .samples::<i16>()
    .map(|s| s.unwrap() as f32 / 32768.0)
    .collect()
}

fn run_worker(model: &str, samples: &[f32], timed: bool) -> serde_json::Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_voxtype"))
        .args(["transcribe-worker", "--model", model, "--language", "en"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line.trim(), voxtype::transcribe::worker::READY_SIGNAL);

    let mut stdin = child.stdin.take().unwrap();
    let flag = if timed {
        voxtype::transcribe::worker::TIMED_FLAG
    } else {
        0
    };
    stdin
        .write_all(&(samples.len() as u32 | flag).to_le_bytes())
        .unwrap();
    for s in samples {
        stdin.write_all(&s.to_le_bytes()).unwrap();
    }
    drop(stdin);
    line.clear();
    stdout.read_line(&mut line).unwrap();
    child.wait().unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn the_worker_returns_timed_segments_when_asked() {
    let Ok(model) = std::env::var("VOXTYPE_TEST_WHISPER_MODEL") else {
        eprintln!("skipping: VOXTYPE_TEST_WHISPER_MODEL is not set");
        return;
    };
    let sentence = speech();
    let mut audio = vec![0.0f32; 16000];
    audio.extend(&sentence);
    audio.extend(vec![0.0f32; 8 * 16000]);
    audio.extend(&sentence);
    audio.resize(30 * 16000, 0.0);

    let timed = run_worker(&model, &audio, true);
    assert_eq!(timed["ok"], true, "{timed}");
    let segments = timed["segments"].as_array().unwrap();
    assert!(segments.len() >= 2, "{timed}");
    assert!(segments[0]["start_secs"].as_f64().unwrap() < 2.0, "{timed}");
    assert!(!timed["text"].as_str().unwrap().is_empty());

    // Without the flag the reply is as before: text, no segments.
    let plain = run_worker(&model, &sentence, false);
    assert_eq!(plain["ok"], true, "{plain}");
    assert!(plain.get("segments").is_none(), "{plain}");
    assert!(plain["text"].as_str().unwrap().contains("voice activity"));
}
