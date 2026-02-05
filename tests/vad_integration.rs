//! Deterministic VAD integration tests using pre-generated audio fixtures
//!
//! These tests verify VAD behavior with known audio files, allowing
//! CI testing without live audio or human interaction.

use std::path::PathBuf;
use voxtype::config::{Config, VadBackend, VadConfig};
use voxtype::vad::{create_vad, EnergyVad, VoiceActivityDetector};

/// Path to VAD test fixtures
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vad")
}

/// Load audio samples from a WAV file
fn load_wav(filename: &str) -> Vec<f32> {
    let path = fixtures_dir().join(filename);
    let reader = hound::WavReader::open(&path)
        .unwrap_or_else(|e| panic!("Failed to open {}: {}", path.display(), e));

    let spec = reader.spec();
    assert_eq!(spec.sample_rate, 16000, "Expected 16kHz audio");
    assert_eq!(spec.channels, 1, "Expected mono audio");

    let max_val = (1 << (spec.bits_per_sample - 1)) as f32;
    reader
        .into_samples::<i32>()
        .filter_map(|s| s.ok())
        .map(|s| s as f32 / max_val)
        .collect()
}

/// Create an Energy VAD with default config
fn energy_vad() -> EnergyVad {
    let config = VadConfig::default();
    EnergyVad::new(&config)
}

/// Create an Energy VAD with custom threshold
fn energy_vad_with_threshold(threshold: f32) -> EnergyVad {
    let config = VadConfig {
        enabled: true,
        backend: VadBackend::Energy,
        threshold,
        min_speech_duration_ms: 100,
        model: None,
    };
    EnergyVad::new(&config)
}

// ============================================================================
// Energy VAD Tests - Silence
// ============================================================================

#[test]
fn energy_vad_rejects_pure_silence() {
    let samples = load_wav("silence_2s.wav");
    let vad = energy_vad();
    let result = vad.detect(&samples).unwrap();

    assert!(!result.has_speech, "Pure silence should not be detected as speech");
    assert_eq!(result.speech_duration_secs, 0.0);
    assert_eq!(result.speech_ratio, 0.0);
    // RMS may be slightly above 0 due to WAV quantization noise
    assert!(result.rms_energy < 0.001, "RMS energy should be near zero");
}

#[test]
fn energy_vad_rejects_short_silence() {
    let samples = load_wav("silence_50ms.wav");
    let vad = energy_vad();
    let result = vad.detect(&samples).unwrap();

    assert!(!result.has_speech, "Short silence should not be detected as speech");
}

#[test]
fn energy_vad_rejects_low_noise() {
    let samples = load_wav("low_noise.wav");
    let vad = energy_vad();
    let result = vad.detect(&samples).unwrap();

    assert!(!result.has_speech, "Very low noise should not be detected as speech");
    assert!(result.rms_energy < 0.01, "RMS energy should be very low");
}

// ============================================================================
// Energy VAD Tests - Non-Speech Audio
// ============================================================================

#[test]
fn energy_vad_accepts_tone() {
    let samples = load_wav("tone_440hz.wav");
    let vad = energy_vad();
    let result = vad.detect(&samples).unwrap();

    // Energy VAD detects any audio with sufficient energy, not just speech
    assert!(result.has_speech, "Loud tone should be detected as 'audio present'");
    assert!(result.speech_ratio > 0.9, "Tone should fill most of the audio");
}

#[test]
fn energy_vad_accepts_white_noise() {
    let samples = load_wav("white_noise.wav");
    let vad = energy_vad();
    let result = vad.detect(&samples).unwrap();

    // White noise has high energy, so Energy VAD will detect it
    assert!(result.has_speech, "Loud white noise should be detected");
    assert!(result.speech_ratio > 0.9, "White noise should fill most of the audio");
    assert!(result.rms_energy > 0.01, "White noise should have measurable energy");
}

#[test]
fn energy_vad_accepts_mixed_tones() {
    let samples = load_wav("mixed_tones.wav");
    let vad = energy_vad();
    let result = vad.detect(&samples).unwrap();

    assert!(result.has_speech, "Mixed tones should be detected as audio");
}

// ============================================================================
// Energy VAD Tests - Speech
// ============================================================================

#[test]
fn energy_vad_accepts_speech_hello() {
    let samples = load_wav("speech_hello.wav");
    let vad = energy_vad();
    let result = vad.detect(&samples).unwrap();

    assert!(result.has_speech, "Speech should be detected");
    assert!(result.speech_ratio > 0.5, "Most of the clip should contain speech");
}

#[test]
fn energy_vad_accepts_speech_long() {
    let samples = load_wav("speech_long.wav");
    let vad = energy_vad();
    let result = vad.detect(&samples).unwrap();

    assert!(result.has_speech, "Long speech should be detected");
    assert!(result.speech_duration_secs > 1.0, "Should detect significant speech duration");
}

#[test]
fn energy_vad_accepts_speech_padded() {
    let samples = load_wav("speech_padded.wav");
    let vad = energy_vad();
    let result = vad.detect(&samples).unwrap();

    assert!(result.has_speech, "Speech with silence padding should still be detected");
    // The speech ratio should be lower due to silence padding
    assert!(result.speech_ratio < 0.8, "Speech ratio should reflect silence padding");
    assert!(result.speech_ratio > 0.1, "But should still detect the speech portion");
}

#[test]
fn energy_vad_handles_quiet_speech() {
    let samples = load_wav("speech_quiet.wav");

    // With default threshold, quiet speech might not be detected
    let vad_default = energy_vad();
    let result_default = vad_default.detect(&samples).unwrap();

    // With lower threshold, should detect quiet speech
    let vad_sensitive = energy_vad_with_threshold(0.2);
    let result_sensitive = vad_sensitive.detect(&samples).unwrap();

    // Sensitive VAD should detect more speech than default
    assert!(
        result_sensitive.speech_ratio >= result_default.speech_ratio,
        "More sensitive VAD should detect equal or more speech"
    );
}

// ============================================================================
// Energy VAD Tests - Threshold Behavior
// ============================================================================

#[test]
fn energy_vad_threshold_affects_detection() {
    let samples = load_wav("speech_hello.wav");

    // Very aggressive threshold
    let vad_aggressive = energy_vad_with_threshold(0.9);
    let result_aggressive = vad_aggressive.detect(&samples).unwrap();

    // Very sensitive threshold
    let vad_sensitive = energy_vad_with_threshold(0.1);
    let result_sensitive = vad_sensitive.detect(&samples).unwrap();

    // Sensitive should detect more than aggressive
    assert!(
        result_sensitive.speech_ratio >= result_aggressive.speech_ratio,
        "Sensitive threshold should detect more speech than aggressive"
    );
}

// ============================================================================
// VAD Factory Tests
// ============================================================================

#[test]
fn create_vad_returns_none_when_disabled() {
    let config = Config::default();
    assert!(!config.vad.enabled);

    let vad = create_vad(&config).unwrap();
    assert!(vad.is_none(), "VAD should be None when disabled");
}

#[test]
fn create_vad_returns_energy_vad_for_parakeet() {
    let mut config = Config::default();
    config.vad.enabled = true;
    config.vad.backend = VadBackend::Auto;
    config.engine = voxtype::config::TranscriptionEngine::Parakeet;

    // Auto + Parakeet = Energy VAD (no model needed)
    let vad = create_vad(&config).unwrap();
    assert!(vad.is_some(), "VAD should be created for Parakeet");
}

#[test]
fn create_vad_energy_backend_explicit() {
    let mut config = Config::default();
    config.vad.enabled = true;
    config.vad.backend = VadBackend::Energy;

    let vad = create_vad(&config).unwrap();
    assert!(vad.is_some(), "Energy VAD should always be creatable");

    // Verify it works with test audio
    let samples = load_wav("silence_2s.wav");
    let result = vad.unwrap().detect(&samples).unwrap();
    assert!(!result.has_speech);
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn energy_vad_handles_empty_audio() {
    let vad = energy_vad();
    let result = vad.detect(&[]).unwrap();

    assert!(!result.has_speech, "Empty audio should not be speech");
    assert_eq!(result.speech_duration_secs, 0.0);
    assert_eq!(result.speech_ratio, 0.0);
}

#[test]
fn energy_vad_handles_single_sample() {
    let vad = energy_vad();
    let result = vad.detect(&[0.5]).unwrap();

    // Single sample is too short to meet min_speech_duration
    assert!(!result.has_speech);
}

#[test]
fn energy_vad_min_speech_duration_filtering() {
    let samples = load_wav("speech_hello.wav");

    // With very high min_speech_duration, even valid speech should be rejected
    let config = VadConfig {
        enabled: true,
        backend: VadBackend::Energy,
        threshold: 0.5,
        min_speech_duration_ms: 10000, // 10 seconds - longer than the clip
        model: None,
    };
    let vad = EnergyVad::new(&config);
    let result = vad.detect(&samples).unwrap();

    assert!(!result.has_speech, "Speech shorter than min_duration should be rejected");
    // But speech_duration_secs should still report the actual detected duration
    assert!(result.speech_duration_secs > 0.0);
}

// ============================================================================
// Whisper VAD Tests (require model to be installed)
// ============================================================================

/// Check if the Whisper VAD model is installed
fn whisper_vad_model_available() -> bool {
    let models_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("voxtype/models");
    models_dir.join("ggml-silero-vad.bin").exists()
}

/// Create a Whisper VAD instance if model is available
fn try_create_whisper_vad() -> Option<voxtype::vad::WhisperVad> {
    if !whisper_vad_model_available() {
        return None;
    }

    let models_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("voxtype/models");
    let model_path = models_dir.join("ggml-silero-vad.bin");

    let config = VadConfig {
        enabled: true,
        backend: VadBackend::Whisper,
        threshold: 0.5,
        min_speech_duration_ms: 100,
        model: None,
    };

    voxtype::vad::WhisperVad::new(&model_path, &config).ok()
}

#[test]
fn whisper_vad_rejects_silence() {
    let Some(vad) = try_create_whisper_vad() else {
        eprintln!("Skipping: Whisper VAD model not installed");
        return;
    };

    let samples = load_wav("silence_2s.wav");
    let result = vad.detect(&samples).unwrap();

    assert!(!result.has_speech, "Whisper VAD should reject pure silence");
    assert_eq!(result.speech_duration_secs, 0.0);
}

#[test]
fn whisper_vad_rejects_tone() {
    let Some(vad) = try_create_whisper_vad() else {
        eprintln!("Skipping: Whisper VAD model not installed");
        return;
    };

    let samples = load_wav("tone_440hz.wav");
    let result = vad.detect(&samples).unwrap();

    // Whisper VAD (Silero) is trained on speech, should reject pure tones
    assert!(!result.has_speech, "Whisper VAD should reject non-speech tones");
}

#[test]
fn whisper_vad_rejects_white_noise() {
    let Some(vad) = try_create_whisper_vad() else {
        eprintln!("Skipping: Whisper VAD model not installed");
        return;
    };

    let samples = load_wav("white_noise.wav");
    let result = vad.detect(&samples).unwrap();

    // Whisper VAD should reject white noise as non-speech
    assert!(!result.has_speech, "Whisper VAD should reject white noise as non-speech");
}

#[test]
fn whisper_vad_accepts_speech() {
    let Some(vad) = try_create_whisper_vad() else {
        eprintln!("Skipping: Whisper VAD model not installed");
        return;
    };

    let samples = load_wav("speech_hello.wav");
    let result = vad.detect(&samples).unwrap();

    assert!(result.has_speech, "Whisper VAD should detect TTS speech");
    assert!(result.speech_ratio > 0.5, "Most of the speech clip should be detected");
}

#[test]
fn whisper_vad_accepts_long_speech() {
    let Some(vad) = try_create_whisper_vad() else {
        eprintln!("Skipping: Whisper VAD model not installed");
        return;
    };

    let samples = load_wav("speech_long.wav");
    let result = vad.detect(&samples).unwrap();

    assert!(result.has_speech, "Whisper VAD should detect longer speech");
    assert!(result.speech_duration_secs > 1.0, "Should detect multiple seconds of speech");
}

#[test]
fn whisper_vad_handles_padded_speech() {
    let Some(vad) = try_create_whisper_vad() else {
        eprintln!("Skipping: Whisper VAD model not installed");
        return;
    };

    let samples = load_wav("speech_padded.wav");
    let result = vad.detect(&samples).unwrap();

    assert!(result.has_speech, "Whisper VAD should detect speech even with silence padding");
    // Speech ratio should be lower due to silence padding
    assert!(result.speech_ratio < 0.7, "Should account for silence padding");
}

// ============================================================================
// Comparison Tests: Energy vs Whisper VAD behavior
// ============================================================================

#[test]
fn compare_vad_backends_on_tone() {
    // Energy VAD detects tones, Whisper VAD does not
    let samples = load_wav("tone_440hz.wav");

    let energy = energy_vad();
    let energy_result = energy.detect(&samples).unwrap();

    // Energy VAD should detect the tone as "audio present"
    assert!(energy_result.has_speech, "Energy VAD should detect tone");

    // If Whisper VAD is available, it should NOT detect tone as speech
    if let Some(whisper) = try_create_whisper_vad() {
        let whisper_result = whisper.detect(&samples).unwrap();
        assert!(!whisper_result.has_speech, "Whisper VAD should not detect tone as speech");
    }
}

#[test]
fn compare_vad_backends_on_speech() {
    // Both should detect speech
    let samples = load_wav("speech_hello.wav");

    let energy = energy_vad();
    let energy_result = energy.detect(&samples).unwrap();
    assert!(energy_result.has_speech, "Energy VAD should detect speech");

    if let Some(whisper) = try_create_whisper_vad() {
        let whisper_result = whisper.detect(&samples).unwrap();
        assert!(whisper_result.has_speech, "Whisper VAD should detect speech");
    }
}
