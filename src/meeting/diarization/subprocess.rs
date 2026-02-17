//! Subprocess-based diarization for memory isolation
//!
//! Runs speaker embedding extraction in a subprocess that exits after
//! processing, releasing memory. Useful on memory-constrained systems.

use super::{DiarizationConfig, DiarizedSegment, Diarizer, SpeakerId};
use crate::meeting::data::AudioSource;
use crate::meeting::TranscriptSegment;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

/// Subprocess-based diarizer wrapper
#[allow(dead_code)]
pub struct SubprocessDiarizer {
    /// Diarization configuration
    config: DiarizationConfig,
    /// Child process handle
    child: Option<Child>,
}

impl SubprocessDiarizer {
    /// Create a new subprocess diarizer
    pub fn new(config: DiarizationConfig) -> Self {
        Self {
            config,
            child: None,
        }
    }

    /// Spawn the worker subprocess
    #[allow(dead_code)]
    fn spawn_worker(&mut self) -> Result<&mut Child, String> {
        if self.child.is_some() {
            return self
                .child
                .as_mut()
                .ok_or("Child already exists".to_string());
        }

        let exe = std::env::current_exe().map_err(|e| format!("Failed to get exe path: {}", e))?;

        let mut cmd = Command::new(exe);
        cmd.arg("--diarization-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        if let Some(ref model_path) = self.config.model_path {
            cmd.arg("--model").arg(model_path);
        }

        let child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn worker: {}", e))?;
        self.child = Some(child);
        self.child.as_mut().ok_or("Failed to get child".to_string())
    }

    /// Send audio samples to worker and receive embeddings
    #[allow(dead_code)]
    fn process_in_worker(
        &mut self,
        samples: &[f32],
        segments: &[TranscriptSegment],
    ) -> Result<Vec<DiarizedSegment>, String> {
        let child = self.spawn_worker()?;

        let stdin = child.stdin.as_mut().ok_or("No stdin")?;
        let stdout = child.stdout.as_mut().ok_or("No stdout")?;

        // Send sample count
        writeln!(stdin, "{}", samples.len()).map_err(|e| format!("Write error: {}", e))?;

        // Send samples (as space-separated floats, chunked)
        for chunk in samples.chunks(1000) {
            let line: String = chunk
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            writeln!(stdin, "{}", line).map_err(|e| format!("Write error: {}", e))?;
        }

        // Send segment count
        writeln!(stdin, "{}", segments.len()).map_err(|e| format!("Write error: {}", e))?;

        // Send segments (start_ms end_ms text)
        for seg in segments {
            writeln!(stdin, "{} {} {}", seg.start_ms, seg.end_ms, seg.text)
                .map_err(|e| format!("Write error: {}", e))?;
        }

        stdin.flush().map_err(|e| format!("Flush error: {}", e))?;

        // Read results
        let reader = BufReader::new(stdout);
        let mut results = Vec::new();

        for line in reader.lines() {
            let line = line.map_err(|e| format!("Read error: {}", e))?;
            if line.is_empty() || line == "END" {
                break;
            }

            // Parse: speaker_id start_ms end_ms confidence text
            let parts: Vec<&str> = line.splitn(5, ' ').collect();
            if parts.len() < 5 {
                continue;
            }

            let speaker = parse_speaker_id(parts[0]);
            let start_ms: u64 = parts[1].parse().unwrap_or(0);
            let end_ms: u64 = parts[2].parse().unwrap_or(0);
            let confidence: f32 = parts[3].parse().unwrap_or(0.0);
            let text = parts[4].to_string();

            results.push(DiarizedSegment {
                speaker,
                start_ms,
                end_ms,
                text,
                confidence,
            });
        }

        // Kill the subprocess to release memory
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;

        Ok(results)
    }
}

#[allow(dead_code)]
fn parse_speaker_id(s: &str) -> SpeakerId {
    match s {
        "You" => SpeakerId::You,
        "Remote" => SpeakerId::Remote,
        "Unknown" => SpeakerId::Unknown,
        s if s.starts_with("SPEAKER_") => {
            if let Ok(id) = s.trim_start_matches("SPEAKER_").parse() {
                SpeakerId::Auto(id)
            } else {
                SpeakerId::Unknown
            }
        }
        s => SpeakerId::Named(s.to_string()),
    }
}

impl Diarizer for SubprocessDiarizer {
    fn diarize(
        &self,
        _samples: &[f32],
        _source: AudioSource,
        transcript_segments: &[TranscriptSegment],
    ) -> Vec<DiarizedSegment> {
        // Note: Diarizer trait takes &self, but we need &mut self for subprocess
        // This is a limitation - in practice, we'd use interior mutability
        // For now, return simple attribution as fallback
        transcript_segments
            .iter()
            .map(|seg| {
                let speaker = match seg.source {
                    AudioSource::Microphone => SpeakerId::You,
                    AudioSource::Loopback => SpeakerId::Remote,
                    AudioSource::Unknown => SpeakerId::Unknown,
                };
                DiarizedSegment {
                    speaker,
                    start_ms: seg.start_ms,
                    end_ms: seg.end_ms,
                    text: seg.text.clone(),
                    confidence: 0.5,
                }
            })
            .collect()
    }

    fn name(&self) -> &'static str {
        "subprocess"
    }
}

/// Worker entry point for subprocess diarization
/// Called when voxtype is run with --diarization-worker
pub fn run_worker(_model_path: Option<&str>) -> Result<(), String> {
    use std::io::{stdin, stdout, BufRead};

    let stdin = stdin();
    let mut stdout = stdout();
    let mut reader = stdin.lock();
    let mut line = String::new();

    // Read sample count
    line.clear();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("Read error: {}", e))?;
    let sample_count: usize = line
        .trim()
        .parse()
        .map_err(|e| format!("Parse error: {}", e))?;

    // Read samples
    let mut samples = Vec::with_capacity(sample_count);
    let mut remaining = sample_count;
    while remaining > 0 {
        line.clear();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("Read error: {}", e))?;
        for s in line.split_whitespace() {
            if let Ok(f) = s.parse::<f32>() {
                samples.push(f);
                remaining = remaining.saturating_sub(1);
            }
        }
    }

    // Read segment count
    line.clear();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("Read error: {}", e))?;
    let segment_count: usize = line
        .trim()
        .parse()
        .map_err(|e| format!("Parse error: {}", e))?;

    // Read segments
    let mut segments = Vec::with_capacity(segment_count);
    for _ in 0..segment_count {
        line.clear();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("Read error: {}", e))?;
        let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
        if parts.len() >= 3 {
            let start_ms: u64 = parts[0].parse().unwrap_or(0);
            let end_ms: u64 = parts[1].parse().unwrap_or(0);
            let text = parts[2].to_string();
            segments.push((start_ms, end_ms, text));
        }
    }

    // Process with ML diarizer (simplified - just return with unknown speaker for now)
    // In a real implementation, we'd load the ONNX model and run inference
    for (start_ms, end_ms, text) in segments {
        writeln!(stdout, "Unknown {} {} 0.5 {}", start_ms, end_ms, text)
            .map_err(|e| format!("Write error: {}", e))?;
    }

    writeln!(stdout, "END").map_err(|e| format!("Write error: {}", e))?;
    stdout.flush().map_err(|e| format!("Flush error: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_speaker_id() {
        assert_eq!(parse_speaker_id("You"), SpeakerId::You);
        assert_eq!(parse_speaker_id("Remote"), SpeakerId::Remote);
        assert_eq!(parse_speaker_id("Unknown"), SpeakerId::Unknown);
        assert_eq!(parse_speaker_id("SPEAKER_00"), SpeakerId::Auto(0));
        assert_eq!(parse_speaker_id("SPEAKER_05"), SpeakerId::Auto(5));
        assert_eq!(
            parse_speaker_id("Alice"),
            SpeakerId::Named("Alice".to_string())
        );
    }
}
