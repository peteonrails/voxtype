//! Audio preprocessing module
//!
//! Provides audio preprocessing optimizations such as speedup and silence removal
//! to reduce transcription time for long audio recordings.
//!
//! Note: Whispered speech is not currently optimized - a future version will add
//! Voice Activity Detection (VAD) for better handling of quiet recordings.

use crate::error::{AudioError, Result};
use hound::{WavReader, WavSpec, WavWriter};
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, info};

/// Calculate optimal speedup factor to fit audio into N×29s batches.
///
/// Returns `None` if no speedup is needed (duration <= 29s) or if speedup
/// would exceed 2.0x. Otherwise returns a factor between 1.0 and 2.0.
pub fn calculate_speedup_factor(duration_secs: f32) -> Option<f32> {
    if duration_secs <= 29.0 {
        debug!("No speedup needed (duration {:.3}s <= 29.0s)", duration_secs);
        return None; // Fits in one batch, no speedup needed
    }

    // Find minimum batches at max 2x speedup
    // At 2x speedup, 29s becomes 58s, so we can fit duration_secs / 58.0 batches
    let min_batches = (duration_secs / 58.0).ceil(); // 29s * 2.0
    let target_duration = min_batches * 29.0;
    let speedup = duration_secs / target_duration;

    debug!(
        "Speedup calculation: duration={:.3}s, min_batches={}, target_duration={:.3}s, speedup={:.6}",
        duration_secs, min_batches, target_duration, speedup
    );

    if speedup <= 1.0 || speedup > 2.0 {
        debug!(
            "Speedup rejected (speedup={:.6}, must be 1.0 < factor <= 2.0)",
            speedup
        );
        return None;
    }

    debug!(
        "Speedup accepted: {:.6}x (will reduce {:.3}s to {:.3}s)",
        speedup, duration_secs, target_duration
    );
    Some(speedup)
}

/// Speed up audio samples using FFmpeg's atempo filter (no pitch adjustment).
///
/// Writes samples to a temporary WAV file, applies speedup via FFmpeg,
/// then reads back the sped-up samples.
fn speedup_samples(
    samples: &[f32],
    sample_rate: u32,
    factor: f32,
) -> Result<Vec<f32>> {
    // Create temporary files with unique names
    let temp_dir = std::env::temp_dir();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let input_path = temp_dir.join(format!("voxtype_speedup_input_{}.wav", timestamp));
    let output_path = temp_dir.join(format!("voxtype_speedup_output_{}.wav", timestamp));

    // Ensure cleanup happens even on error
    struct TempFileGuard(PathBuf);
    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _input_guard = TempFileGuard(input_path.clone());
    let _output_guard = TempFileGuard(output_path.clone());

    // Write input WAV file
    {
        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let file = File::create(&input_path)
            .map_err(|e| AudioError::StreamError(format!("Failed to create temp file: {}", e)))?;
        let mut writer = WavWriter::new(BufWriter::new(file), spec)
            .map_err(|e| AudioError::StreamError(format!("Failed to create WAV writer: {}", e)))?;

        // Convert f32 [-1.0, 1.0] to i16
        let max_val = (1i16 << 15) as f32;
        for &sample in samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let int_sample = (clamped * max_val) as i16;
            writer
                .write_sample(int_sample)
                .map_err(|e| AudioError::StreamError(format!("Failed to write sample: {}", e)))?;
        }

        writer
            .finalize()
            .map_err(|e| AudioError::StreamError(format!("Failed to finalize WAV: {}", e)))?;
    }

    debug!(
        "FFmpeg speedup: input={:?}, output={:?}, factor={:.6}",
        input_path, output_path, factor
    );

    // Call FFmpeg
    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-i",
            input_path.to_str().unwrap(),
            "-filter:a",
            &format!("atempo={}", factor),
            output_path.to_str().unwrap(),
        ])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AudioError::StreamError(
                    "FFmpeg not found in PATH. Install FFmpeg to use speedup optimization.".to_string(),
                )
            } else {
                AudioError::StreamError(format!("Failed to execute FFmpeg: {}", e))
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AudioError::StreamError(format!(
            "FFmpeg failed: {}",
            stderr
        ))
        .into());
    }

    // Read back sped-up WAV file
    let reader = WavReader::open(&output_path)
        .map_err(|e| AudioError::StreamError(format!("Failed to open output WAV: {}", e)))?;
    let spec = reader.spec();

    let sped_up_samples: Vec<f32> = match spec.sample_format {
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

    debug!(
        "Speedup complete: {} samples -> {} samples ({:.3}s -> {:.3}s)",
        samples.len(),
        sped_up_samples.len(),
        samples.len() as f32 / sample_rate as f32,
        sped_up_samples.len() as f32 / sample_rate as f32
    );

    Ok(sped_up_samples)
}

/// Helper function to write samples to a temporary WAV file
fn write_samples_to_wav(samples: &[f32], sample_rate: u32, path: &PathBuf) -> Result<()> {
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let file = File::create(path)
        .map_err(|e| AudioError::StreamError(format!("Failed to create temp file: {}", e)))?;
    let mut writer = WavWriter::new(BufWriter::new(file), spec)
        .map_err(|e| AudioError::StreamError(format!("Failed to create WAV writer: {}", e)))?;

    // Convert f32 [-1.0, 1.0] to i16
    let max_val = (1i16 << 15) as f32;
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let int_sample = (clamped * max_val) as i16;
        writer
            .write_sample(int_sample)
            .map_err(|e| AudioError::StreamError(format!("Failed to write sample: {}", e)))?;
    }

    writer
        .finalize()
        .map_err(|e| AudioError::StreamError(format!("Failed to finalize WAV: {}", e)))?;

    Ok(())
}

/// Helper function to read samples from a WAV file
fn read_samples_from_wav(path: &PathBuf) -> Result<Vec<f32>> {
    let reader = WavReader::open(path)
        .map_err(|e| AudioError::StreamError(format!("Failed to open WAV: {}", e)))?;
    let spec = reader.spec();

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

    Ok(samples)
}

/// Detect mean volume of audio samples using FFmpeg volumedetect filter.
///
/// Returns the mean volume in dB. Falls back to -30.0 dB if parsing fails.
fn detect_mean_volume(samples: &[f32], sample_rate: u32) -> Result<f32> {
    // Create temporary file
    let temp_dir = std::env::temp_dir();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let input_path = temp_dir.join(format!("voxtype_volumedetect_{}.wav", timestamp));

    // Ensure cleanup happens even on error
    struct TempFileGuard(PathBuf);
    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _guard = TempFileGuard(input_path.clone());

    // Write samples to WAV file
    write_samples_to_wav(samples, sample_rate, &input_path)?;

    debug!("FFmpeg volumedetect: input={:?}", input_path);

    // Call FFmpeg volumedetect
    let output = Command::new("ffmpeg")
        .args([
            "-i",
            input_path.to_str().unwrap(),
            "-af",
            "volumedetect",
            "-f",
            "null",
            "-",
        ])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AudioError::StreamError(
                    "FFmpeg not found in PATH. Install FFmpeg to use silence removal optimization.".to_string(),
                )
            } else {
                AudioError::StreamError(format!("Failed to execute FFmpeg: {}", e))
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        debug!("FFmpeg volumedetect failed: {}", stderr);
        // Fallback to default
        return Ok(-30.0);
    }

    // Parse stderr for mean_volume line
    // Format: "mean_volume: -XX.X dB"
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stderr.lines() {
        if line.contains("mean_volume:") {
            // Extract the dB value
            if let Some(colon_pos) = line.find("mean_volume:") {
                let after_colon = &line[colon_pos + "mean_volume:".len()..];
                let db_str = after_colon.trim().replace(" dB", "").trim().to_string();
                if let Ok(mean_db) = db_str.parse::<f32>() {
                    debug!("Detected mean volume: {} dB", mean_db);
                    return Ok(mean_db);
                }
            }
        }
    }

    // Fallback: return a reasonable default if parsing fails
    debug!("Could not parse mean_volume from ffmpeg output, using default -30 dB");
    Ok(-30.0)
}

/// Remove silence from audio samples using FFmpeg silenceremove filter.
///
/// The threshold is relative to mean volume (typically mean_volume - 20 dB).
/// The stop_duration is the minimum silence duration required before removal.
fn remove_silence(
    samples: &[f32],
    sample_rate: u32,
    threshold_db: f32,
    stop_duration_s: f32,
) -> Result<Vec<f32>> {
    // Create temporary files
    let temp_dir = std::env::temp_dir();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let input_path = temp_dir.join(format!("voxtype_silence_input_{}.wav", timestamp));
    let output_path = temp_dir.join(format!("voxtype_silence_output_{}.wav", timestamp));

    // Ensure cleanup happens even on error
    struct TempFileGuard(PathBuf);
    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _input_guard = TempFileGuard(input_path.clone());
    let _output_guard = TempFileGuard(output_path.clone());

    // Write input WAV file
    write_samples_to_wav(samples, sample_rate, &input_path)?;

    debug!(
        "FFmpeg silence removal: input={:?}, output={:?}, threshold={} dB, stop_duration={}s",
        input_path, output_path, threshold_db, stop_duration_s
    );

    // Call FFmpeg silenceremove
    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-i",
            input_path.to_str().unwrap(),
            "-af",
            &format!(
                "silenceremove=stop_periods=-1:stop_duration={}:stop_threshold={}dB",
                stop_duration_s, threshold_db
            ),
            output_path.to_str().unwrap(),
        ])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AudioError::StreamError(
                    "FFmpeg not found in PATH. Install FFmpeg to use silence removal optimization.".to_string(),
                )
            } else {
                AudioError::StreamError(format!("Failed to execute FFmpeg: {}", e))
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AudioError::StreamError(format!("FFmpeg failed: {}", stderr)).into());
    }

    // Read back silence-removed WAV file
    let processed_samples = read_samples_from_wav(&output_path)?;

    debug!(
        "Silence removal complete: {} samples -> {} samples ({:.3}s -> {:.3}s)",
        samples.len(),
        processed_samples.len(),
        samples.len() as f32 / sample_rate as f32,
        processed_samples.len() as f32 / sample_rate as f32
    );

    Ok(processed_samples)
}

/// Preprocess audio samples, applying silence removal and speedup optimizations if beneficial.
///
/// This is the main entry point for audio preprocessing. Processing order:
/// 1. Silence removal (if enabled)
/// 2. Speedup (if enabled, applied to silence-removed audio)
pub fn preprocess_audio(
    samples: &[f32],
    sample_rate: u32,
    silence_removal_enabled: bool,
    speedup_enabled: bool,
) -> Result<Vec<f32>> {
    if samples.is_empty() {
        return Ok(samples.to_vec());
    }

    let mut processed_samples = samples.to_vec();

    // Step 1: Apply silence removal if enabled
    if silence_removal_enabled {
        debug!("Detecting mean volume for silence removal");
        let mean_db = detect_mean_volume(&processed_samples, sample_rate)?;
        // Use max() to ensure threshold never goes below -44.0 dB
        // This prevents the threshold from being too low for microphones with higher noise floors
        let threshold_db = (mean_db - 20.0).max(-44.0);
        debug!("Removing silence: mean_volume={:.2} dB, threshold={:.2} dB", mean_db, threshold_db);

        // Note: Whispered speech is not currently optimized - a future version will add
        // Voice Activity Detection (VAD) for better handling of quiet recordings.
        let stop_duration_s = 0.1;
        debug!("Silence removal: using stop_duration={:.1}s", stop_duration_s);

        let original_duration = processed_samples.len() as f32 / sample_rate as f32;
        processed_samples = remove_silence(&processed_samples, sample_rate, threshold_db, stop_duration_s)?;
        let post_silence_duration = processed_samples.len() as f32 / sample_rate as f32;
        let reduction = original_duration - post_silence_duration;
        
        if reduction > 0.01 {
            // Only log if there was meaningful compression (>10ms)
            info!(
                "Silence removal compressed audio: {:.2}s -> {:.2}s (reduced by {:.2}s, {:.1}%)",
                original_duration,
                post_silence_duration,
                reduction,
                (reduction / original_duration) * 100.0
            );
        } else {
            debug!(
                "Silence removal: {:.3}s -> {:.3}s (reduced by {:.3}s)",
                original_duration,
                post_silence_duration,
                reduction
            );
        }
    }

    // Step 2: Apply speedup if enabled (on silence-removed audio)
    if speedup_enabled {
        let duration_secs = processed_samples.len() as f32 / sample_rate as f32;
        if let Some(factor) = calculate_speedup_factor(duration_secs) {
            debug!("Applying speedup optimization: {:.6}x", factor);
            processed_samples = speedup_samples(&processed_samples, sample_rate, factor)?;
        } else {
            debug!("No speedup optimization needed");
        }
    } else {
        debug!("Speedup optimization disabled");
    }

    Ok(processed_samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_speedup_factor_short() {
        assert_eq!(calculate_speedup_factor(20.0), None);
        assert_eq!(calculate_speedup_factor(29.0), None);
    }

    #[test]
    fn test_calculate_speedup_factor_long() {
        let factor = calculate_speedup_factor(60.0);
        assert!(factor.is_some());
        let factor = factor.unwrap();
        assert!(factor > 1.0 && factor <= 2.0);
    }

    #[test]
    fn test_calculate_speedup_factor_very_long() {
        let factor = calculate_speedup_factor(120.0);
        assert!(factor.is_some());
        let factor = factor.unwrap();
        assert!(factor > 1.0 && factor <= 2.0);
    }
}
