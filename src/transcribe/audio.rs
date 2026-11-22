//! Shared audio encoders used by remote transcription backends.

use crate::error::TranscribeError;
use std::io::Cursor;

pub const SAMPLE_RATE: u32 = 16_000;

/// Convert one floating-point sample in [-1.0, 1.0] to signed 16-bit,
/// rounding to the nearest step.
fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

/// Encode 16 kHz mono floating-point samples as PCM signed 16-bit WAV.
pub fn encode_wav_s16le(samples: &[f32]) -> Result<Vec<u8>, TranscribeError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut buffer = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut buffer, spec)
        .map_err(|e| TranscribeError::AudioFormat(format!("WAV writer init: {}", e)))?;

    for &sample in samples {
        writer
            .write_sample(f32_to_i16(sample))
            .map_err(|e| TranscribeError::AudioFormat(format!("WAV sample write: {}", e)))?;
    }

    writer
        .finalize()
        .map_err(|e| TranscribeError::AudioFormat(format!("WAV finalize: {}", e)))?;
    Ok(buffer.into_inner())
}

/// Convert 16 kHz mono floating-point samples to raw PCM signed 16-bit
/// little-endian bytes, the frames a realtime WebSocket API takes.
pub fn pcm_s16le(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        bytes.extend_from_slice(&f32_to_i16(sample).to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_has_expected_header_and_size() {
        let samples = vec![0.0; SAMPLE_RATE as usize];
        let wav = encode_wav_s16le(&samples).unwrap();
        assert_eq!(wav.len(), 44 + samples.len() * 2);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        let data_len = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_len as usize, samples.len() * 2);
        let sample_rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
        assert_eq!(sample_rate, SAMPLE_RATE);
    }

    #[test]
    fn wav_clamps_samples_outside_unit_range() {
        let wav = encode_wav_s16le(&[-2.0, 2.0]).unwrap();
        assert_eq!(i16::from_le_bytes([wav[44], wav[45]]), i16::MIN + 1);
        assert_eq!(i16::from_le_bytes([wav[46], wav[47]]), i16::MAX);
    }

    #[test]
    fn samples_round_to_the_nearest_step() {
        // 0.5 * 32767 = 16383.5, which rounds away from zero.
        assert_eq!(f32_to_i16(0.5), 16384);
        assert_eq!(f32_to_i16(-0.5), -16384);
        assert_eq!(f32_to_i16(0.0), 0);
    }

    #[test]
    fn pcm_matches_the_wav_payload() {
        let samples = [0.0, 0.5, -2.0, 2.0];
        let wav = encode_wav_s16le(&samples).unwrap();
        assert_eq!(pcm_s16le(&samples), wav[44..]);
    }
}
