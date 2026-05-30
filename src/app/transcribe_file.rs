//! `voxtype transcribe <file>` — one-shot transcription of an audio file.
//!
//! `resample` lives here rather than in `src/audio/` because it has exactly
//! one call site (this command). Per the refactoring policy: don't extract
//! an abstraction from a single use site.

use std::path::PathBuf;
use voxtype::{config, transcribe, vad};

/// Transcribe an audio file
pub(crate) fn transcribe_file(config: &config::Config, path: &PathBuf) -> anyhow::Result<()> {
    use hound::WavReader;

    println!("Loading audio file: {:?}", path);

    let reader = WavReader::open(path)?;
    let spec = reader.spec();

    println!(
        "Audio format: {} Hz, {} channel(s), {:?}",
        spec.sample_rate, spec.channels, spec.sample_format
    );

    // Convert samples to f32 mono at 16kHz
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max_val = (1 << (spec.bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / max_val)
                .collect()
        }
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .filter_map(|s| s.ok())
            .collect(),
    };

    // Mix to mono if stereo
    let mono_samples: Vec<f32> = if spec.channels > 1 {
        samples
            .chunks(spec.channels as usize)
            .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
            .collect()
    } else {
        samples
    };

    // Resample to 16kHz if needed
    let final_samples = if spec.sample_rate != 16000 {
        println!("Resampling from {} Hz to 16000 Hz...", spec.sample_rate);
        resample(&mono_samples, spec.sample_rate, 16000)
    } else {
        mono_samples
    };

    println!(
        "Processing {} samples ({:.2}s)...",
        final_samples.len(),
        final_samples.len() as f32 / 16000.0
    );

    // Run VAD if enabled
    if let Ok(Some(vad)) = vad::create_vad(config) {
        match vad.detect(&final_samples) {
            Ok(result) => {
                println!(
                    "VAD: {:.2}s speech ({:.1}% of audio)",
                    result.speech_duration_secs,
                    result.speech_ratio * 100.0
                );
                if !result.has_speech {
                    println!("No speech detected, skipping transcription.");
                    return Ok(());
                }
            }
            Err(e) => {
                eprintln!("VAD warning: {}", e);
                // Continue with transcription if VAD fails
            }
        }
    }

    // Create transcriber and transcribe
    let transcriber = transcribe::create_transcriber(config)?;
    let text = transcriber.transcribe(&final_samples)?;

    println!("\n{}", text);
    Ok(())
}

/// Simple linear resampling
fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return samples.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = (samples.len() as f64 * ratio).ceil() as usize;
    let mut output = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let src_idx = i as f64 / ratio;
        let idx = src_idx.floor() as usize;
        let frac = (src_idx - idx as f64) as f32;

        let sample = if idx + 1 < samples.len() {
            samples[idx] * (1.0 - frac) + samples[idx + 1] * frac
        } else {
            samples.get(idx).copied().unwrap_or(0.0)
        };

        output.push(sample);
    }

    output
}
