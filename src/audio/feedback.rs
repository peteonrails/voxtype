//! Audio feedback module
//!
//! Provides audio cues (beeps/sounds) for recording start/stop events.
//! Supports multiple sound themes and custom sound files.

use crate::config::AudioFeedbackConfig;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::io::Cursor;
use std::path::PathBuf;

/// Sound event types
#[derive(Debug, Clone, Copy)]
pub enum SoundEvent {
    /// Recording started
    RecordingStart,
    /// Recording stopped
    RecordingStop,
    /// Recording/transcription cancelled
    Cancelled,
    /// Error occurred
    Error,
}

/// Audio feedback player
pub struct AudioFeedback {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    config: AudioFeedbackConfig,
    theme: SoundTheme,
}

/// A sound theme containing audio data for different events
struct SoundTheme {
    start: Vec<u8>,
    stop: Vec<u8>,
    cancel: Vec<u8>,
    error: Vec<u8>,
}

impl AudioFeedback {
    /// Create a new audio feedback player
    pub fn new(config: &AudioFeedbackConfig) -> Result<Self, String> {
        if !config.enabled {
            return Err("Audio feedback is disabled".to_string());
        }

        let (stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| format!("Failed to open audio output: {}", e))?;

        let theme = load_theme(&config.theme)?;

        Ok(Self {
            _stream: stream,
            stream_handle,
            config: config.clone(),
            theme,
        })
    }

    /// Play a sound for the given event
    pub fn play(&self, event: SoundEvent) {
        let sound_data = match event {
            SoundEvent::RecordingStart => &self.theme.start,
            SoundEvent::RecordingStop => &self.theme.stop,
            SoundEvent::Cancelled => &self.theme.cancel,
            SoundEvent::Error => &self.theme.error,
        };

        if sound_data.is_empty() {
            return;
        }

        if let Err(e) = self.play_wav(sound_data) {
            tracing::warn!("Failed to play feedback sound: {}", e);
        }
    }

    fn play_wav(&self, data: &[u8]) -> Result<(), String> {
        let cursor = Cursor::new(data.to_vec());
        let source = Decoder::new(cursor).map_err(|e| format!("Failed to decode audio: {}", e))?;

        // Apply volume control
        let source = source.amplify(self.config.volume);

        let sink = Sink::try_new(&self.stream_handle)
            .map_err(|e| format!("Failed to create audio sink: {}", e))?;

        sink.append(source);
        sink.detach(); // Let it play in the background

        Ok(())
    }
}

/// Load a sound theme by name or path
fn load_theme(theme_name: &str) -> Result<SoundTheme, String> {
    match theme_name {
        "default" => Ok(generate_default_theme()),
        "subtle" => Ok(generate_subtle_theme()),
        "mechanical" => Ok(generate_mechanical_theme()),
        path => load_custom_theme(path),
    }
}

/// Load a custom theme from a directory
fn load_custom_theme(path: &str) -> Result<SoundTheme, String> {
    let dir = PathBuf::from(path);
    if !dir.is_dir() {
        return Err(format!("Theme directory not found: {}", path));
    }

    let load_file = |name: &str| -> Vec<u8> {
        let file_path = dir.join(name);
        std::fs::read(&file_path).unwrap_or_default()
    };

    Ok(SoundTheme {
        start: load_file("start.wav"),
        stop: load_file("stop.wav"),
        cancel: load_file("cancel.wav"),
        error: load_file("error.wav"),
    })
}

// === Sound Generation ===
// Generate simple WAV sounds programmatically to avoid shipping binary assets

/// Generate a simple WAV file with a sine wave tone
fn generate_tone_wav(frequency: f32, duration_ms: u32, fade_ms: u32) -> Vec<u8> {
    let sample_rate = 44100u32;
    let num_samples = (sample_rate * duration_ms / 1000) as usize;
    let fade_samples = (sample_rate * fade_ms / 1000) as usize;

    let mut samples: Vec<i16> = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let mut amplitude = (2.0 * std::f32::consts::PI * frequency * t).sin();

        // Apply fade in/out envelope
        if i < fade_samples {
            amplitude *= i as f32 / fade_samples as f32;
        } else if i >= num_samples - fade_samples {
            amplitude *= (num_samples - i) as f32 / fade_samples as f32;
        }

        samples.push((amplitude * 16000.0) as i16);
    }

    encode_wav(&samples, sample_rate)
}

/// Generate a two-tone sound (rising or falling)
fn generate_two_tone_wav(freq1: f32, freq2: f32, duration_ms: u32, fade_ms: u32) -> Vec<u8> {
    let sample_rate = 44100u32;
    let num_samples = (sample_rate * duration_ms / 1000) as usize;
    let fade_samples = (sample_rate * fade_ms / 1000) as usize;
    let half_samples = num_samples / 2;

    let mut samples: Vec<i16> = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let freq = if i < half_samples { freq1 } else { freq2 };
        let mut amplitude = (2.0 * std::f32::consts::PI * freq * t).sin();

        // Apply fade in/out envelope
        if i < fade_samples {
            amplitude *= i as f32 / fade_samples as f32;
        } else if i >= num_samples - fade_samples {
            amplitude *= (num_samples - i) as f32 / fade_samples as f32;
        }

        samples.push((amplitude * 16000.0) as i16);
    }

    encode_wav(&samples, sample_rate)
}

/// Generate a click sound (short burst of noise with envelope)
fn generate_click_wav(duration_ms: u32) -> Vec<u8> {
    let sample_rate = 44100u32;
    let num_samples = (sample_rate * duration_ms / 1000) as usize;

    let mut samples: Vec<i16> = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        // Quick exponential decay envelope
        let envelope = (-5.0 * i as f32 / num_samples as f32).exp();
        // High-frequency noise burst
        let noise = if i % 2 == 0 { 1.0 } else { -1.0 };
        samples.push((noise * envelope * 12000.0) as i16);
    }

    encode_wav(&samples, sample_rate)
}

/// Encode samples as WAV format
fn encode_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let mut wav = Vec::new();

    // RIFF header
    wav.extend_from_slice(b"RIFF");
    let file_size = (36 + samples.len() * 2) as u32;
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    // fmt chunk
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    // data chunk
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&((samples.len() * 2) as u32).to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }

    wav
}

// === Built-in Themes ===

/// Default theme: Clear, pleasant tones
fn generate_default_theme() -> SoundTheme {
    SoundTheme {
        // Rising two-tone: 440Hz -> 880Hz (musical, energizing)
        start: generate_two_tone_wav(440.0, 880.0, 150, 20),
        // Falling two-tone: 880Hz -> 440Hz (completion)
        stop: generate_two_tone_wav(880.0, 440.0, 150, 20),
        // Quick descending triple-beep for cancel (distinct from stop)
        cancel: generate_tone_wav(600.0, 80, 10),
        // Low warning tone
        error: generate_two_tone_wav(300.0, 200.0, 200, 30),
    }
}

/// Subtle theme: Quiet, unobtrusive clicks
fn generate_subtle_theme() -> SoundTheme {
    SoundTheme {
        // Soft high click
        start: generate_tone_wav(1200.0, 50, 10),
        // Soft low click
        stop: generate_tone_wav(800.0, 50, 10),
        // Quick mid-tone for cancel
        cancel: generate_tone_wav(600.0, 40, 8),
        // Double low click
        error: generate_two_tone_wav(400.0, 300.0, 100, 15),
    }
}

/// Mechanical theme: Typewriter/keyboard-like sounds
fn generate_mechanical_theme() -> SoundTheme {
    SoundTheme {
        // Sharp click
        start: generate_click_wav(30),
        // Softer click
        stop: generate_click_wav(20),
        // Double click for cancel
        cancel: generate_click_wav(15),
        // Buzzer
        error: generate_tone_wav(150.0, 150, 20),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_tone_wav() {
        let wav = generate_tone_wav(440.0, 100, 10);
        // Check WAV header
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert!(!wav.is_empty());
    }

    #[test]
    fn test_generate_themes() {
        let default = generate_default_theme();
        assert!(!default.start.is_empty());
        assert!(!default.stop.is_empty());
        assert!(!default.cancel.is_empty());
        assert!(!default.error.is_empty());

        let subtle = generate_subtle_theme();
        assert!(!subtle.start.is_empty());
        assert!(!subtle.cancel.is_empty());

        let mechanical = generate_mechanical_theme();
        assert!(!mechanical.start.is_empty());
        assert!(!mechanical.cancel.is_empty());
    }
}
