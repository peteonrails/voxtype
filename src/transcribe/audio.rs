//! Shared audio encoders used by remote transcription backends.

use crate::error::TranscribeError;
use std::io::Cursor;

pub const SAMPLE_RATE: u32 = 16_000;

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
        let sample = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(sample)
            .map_err(|e| TranscribeError::AudioFormat(format!("WAV sample write: {}", e)))?;
    }

    writer
        .finalize()
        .map_err(|e| TranscribeError::AudioFormat(format!("WAV finalize: {}", e)))?;
    Ok(buffer.into_inner())
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
    }

    #[test]
    fn wav_clamps_samples_outside_unit_range() {
        let wav = encode_wav_s16le(&[-2.0, 2.0]).unwrap();
        assert_eq!(i16::from_le_bytes([wav[44], wav[45]]), i16::MIN + 1);
        assert_eq!(i16::from_le_bytes([wav[46], wav[47]]), i16::MAX);
    }
}
