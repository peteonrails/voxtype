//! Energy-based Voice Activity Detection
//!
//! A simple but effective VAD that uses RMS energy to detect speech.
//! Works well for filtering completely silent recordings without
//! requiring external model downloads.

use crate::config::VadConfig;
use crate::error::VadError;

use super::{VadResult, VoiceActivityDetector};

/// Energy-based VAD using RMS amplitude analysis
///
/// This implementation analyzes audio in short frames (20ms) and determines
/// speech presence based on energy levels exceeding a threshold. It's designed
/// to filter out completely silent or near-silent recordings that would cause
/// Whisper to hallucinate.
pub struct EnergyVad {
    /// Energy threshold for speech detection (0.0 - 1.0)
    /// Frames with RMS energy above this are considered speech
    threshold: f32,
    /// Minimum speech duration in milliseconds
    min_speech_duration_ms: u32,
}

impl EnergyVad {
    /// Create a new energy-based VAD instance
    pub fn new(config: &VadConfig) -> Self {
        // Map the config threshold (0.0-1.0) to an energy threshold
        // Default 0.5 maps to ~0.01 RMS, which filters silence but allows quiet speech
        let energy_threshold = map_threshold_to_energy(config.threshold);

        Self {
            threshold: energy_threshold,
            min_speech_duration_ms: config.min_speech_duration_ms,
        }
    }

    /// Calculate RMS energy of a sample slice
    fn calculate_rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum_squares: f32 = samples.iter().map(|&s| s * s).sum();
        (sum_squares / samples.len() as f32).sqrt()
    }
}

/// Map config threshold (0.0-1.0) to energy threshold
///
/// - 0.0 = very sensitive (energy threshold ~0.001, detects quiet whispers)
/// - 0.5 = balanced (energy threshold ~0.01, filters silence)
/// - 1.0 = aggressive (energy threshold ~0.1, requires louder speech)
fn map_threshold_to_energy(config_threshold: f32) -> f32 {
    // Exponential mapping: lower config values = lower energy threshold
    // Range: 0.001 to 0.1
    let t = config_threshold.clamp(0.0, 1.0);
    0.001 * (100.0_f32).powf(t)
}

impl VoiceActivityDetector for EnergyVad {
    fn detect(&self, samples: &[f32]) -> Result<VadResult, VadError> {
        if samples.is_empty() {
            return Ok(VadResult {
                has_speech: false,
                speech_duration_secs: 0.0,
                speech_ratio: 0.0,
                rms_energy: 0.0,
            });
        }

        const SAMPLE_RATE: usize = 16000;
        const FRAME_MS: usize = 20;
        const FRAME_SIZE: usize = SAMPLE_RATE * FRAME_MS / 1000; // 320 samples

        let mut speech_frames = 0usize;
        let mut total_frames = 0usize;
        let mut total_energy = 0.0f32;

        // Process audio in frames
        for frame in samples.chunks(FRAME_SIZE) {
            let rms = Self::calculate_rms(frame);
            total_energy += rms;
            total_frames += 1;

            if rms >= self.threshold {
                speech_frames += 1;
            }
        }

        let avg_rms = if total_frames > 0 {
            total_energy / total_frames as f32
        } else {
            0.0
        };

        let speech_duration_secs = (speech_frames * FRAME_MS) as f32 / 1000.0;
        let speech_ratio = if total_frames > 0 {
            speech_frames as f32 / total_frames as f32
        } else {
            0.0
        };

        // Determine if there's enough speech
        let min_speech_secs = self.min_speech_duration_ms as f32 / 1000.0;
        let has_speech = speech_duration_secs >= min_speech_secs;

        tracing::debug!(
            "VAD result: has_speech={}, speech_duration={:.2}s ({} frames), \
             speech_ratio={:.1}%, avg_rms={:.4}, threshold={:.4}",
            has_speech,
            speech_duration_secs,
            speech_frames,
            speech_ratio * 100.0,
            avg_rms,
            self.threshold
        );

        Ok(VadResult {
            has_speech,
            speech_duration_secs,
            speech_ratio,
            rms_energy: avg_rms,
        })
    }
}

/// Frame grid shared by [`EnergyVad::detect`] and [`split_on_silence`]:
/// 20 ms at 16 kHz.
const VAD_FRAME: usize = 320;

/// A speech range over the input samples, as returned by [`split_on_silence`].
pub type SpeechRange = std::ops::Range<usize>;

/// Split a recording into speech segments on silence gaps.
///
/// Conformer ASR degrades on long audio. Upstream GigaAM refuses it
/// outright — `gigaam/model.py` raises `"Too long wav file, use
/// 'transcribe_longform' method."` past 25 s — and `transcribe_longform`
/// is exactly VAD segmentation. This is the Rust equivalent, used by the
/// batch path for correctness and by the streaming path to decide where a
/// segment can be committed.
///
/// Returns ranges with leading and trailing silence trimmed, each at most
/// `max_secs` long. Splits land on gaps of at least `min_silence_ms`; when
/// speech runs past `max_secs` without one, the cut falls on the quietest
/// frame in the tail of the window rather than mid-syllable at the boundary.
///
/// An empty result means no frame crossed the speech threshold — the caller
/// decides whether to transcribe the buffer whole or drop it.
pub fn split_on_silence(samples: &[f32], max_secs: f32, min_silence_ms: u32) -> Vec<SpeechRange> {
    let floor = noise_floor(&frame_rms(samples));
    split_on_silence_at(samples, floor, max_secs, min_silence_ms)
}

/// Per-frame RMS on the 20 ms grid, the raw material for [`noise_floor`].
///
/// Exposed so a streaming caller can accumulate frame energies across a
/// whole session instead of re-deriving them from audio it has already
/// committed and dropped.
pub fn frame_rms(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(VAD_FRAME)
        .map(EnergyVad::calculate_rms)
        .collect()
}

/// The quiet end of a recording: the 5th-percentile frame energy.
///
/// Estimate this over as much audio as possible. Over a short buffer that
/// is mostly speech the 5th percentile lands *inside* speech, and every
/// threshold derived from it is far too high — that mistake shreds
/// continuous speech into fragments.
pub fn noise_floor(rms: &[f32]) -> f32 {
    if rms.is_empty() {
        return 0.0;
    }
    let mut sorted = rms.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sorted[sorted.len() / 20]
}

/// [`split_on_silence`] with the noise floor supplied by the caller.
///
/// Streaming sessions must use this: their pending buffer is whatever has
/// not been committed yet, far too short and too speech-dense to estimate a
/// floor from. They pass the floor measured over the whole session instead.
pub fn split_on_silence_at(
    samples: &[f32],
    noise_floor: f32,
    max_secs: f32,
    min_silence_ms: u32,
) -> Vec<SpeechRange> {
    let rms = frame_rms(samples);
    if rms.is_empty() {
        return Vec::new();
    }

    // Anchor the threshold to the recording's own noise floor, not to a
    // percentile of the whole signal: continuous speech has no true silence
    // to average in, so anything derived from the middle of the
    // distribution lands *inside* speech and shreds it.
    //
    // ponytail: 1.5x the 5th-percentile frame, measured against
    // test_audio/example.wav (11 s of continuous Russian, noise floor
    // 0.0092, speech median 0.0196). 1.2x finds no pause at all, 1.5x-2.0x
    // all find exactly the one real pause, 3x invents four. Biased to the
    // low edge because under-splitting is harmless (the cap still fires)
    // while over-splitting cuts mid-word. Re-run that sweep before moving it.
    let threshold = (noise_floor * 1.5).clamp(0.002, 0.05);

    let gap_frames = ((min_silence_ms as usize) / 20).max(1);
    let max_frames = ((max_secs * 50.0) as usize).max(3); // 50 frames per second

    let speech: Vec<bool> = rms.iter().map(|&r| r >= threshold).collect();
    let (first, last) = match (
        speech.iter().position(|&s| s),
        speech.iter().rposition(|&s| s),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return Vec::new(),
    };

    // Cut in the *middle* of each long-enough pause, so segments tile the
    // span from the first speech frame to the last. Nothing between them is
    // ever dropped: a mis-tuned threshold then costs a badly placed cut,
    // never a lost word — which is the failure that actually hurts.
    let mut bounds = vec![first];
    let mut silence_start: Option<usize> = None;
    for i in first..=last {
        if speech[i] {
            if let Some(s) = silence_start.take() {
                if i - s >= gap_frames {
                    bounds.push((s + i) / 2);
                }
            }
        } else if silence_start.is_none() {
            silence_start = Some(i);
        }
    }
    bounds.push(last + 1);

    let mut frames: Vec<(usize, usize)> = Vec::new();
    for pair in bounds.windows(2) {
        let (mut a, b) = (pair[0], pair[1]);
        // Speech that never pauses still has to be cut, or the encoder gets
        // a clip longer than anything it was trained on. Land the cut on the
        // quietest frame in the tail of the window rather than mid-syllable
        // at the boundary.
        while b - a > max_frames {
            let tail = a + max_frames * 2 / 3;
            let cut = tail + quietest_frame(&rms[tail..a + max_frames]);
            frames.push((a, cut + 1));
            a = cut + 1;
        }
        frames.push((a, b));
    }

    // Pad only the outer edges: inner boundaries sit inside a pause already,
    // and padding them would replay the same audio in two segments.
    let pad = (gap_frames / 2).min(5); // <= 100 ms
    let n = frames.len();
    frames
        .into_iter()
        .enumerate()
        .filter(|(_, (a, b))| b.saturating_sub(*a) >= MIN_SPEECH_FRAMES)
        .map(|(i, (a, b))| {
            let a = if i == 0 { a.saturating_sub(pad) } else { a };
            let b = if i + 1 == n { b + pad } else { b };
            (a * VAD_FRAME)..((b * VAD_FRAME).min(samples.len()))
        })
        .filter(|r| r.end > r.start)
        .collect()
}

/// Shortest run worth transcribing: below this it is a click, a cough or a
/// door, and the model would only hallucinate a word for it.
const MIN_SPEECH_FRAMES: usize = 3; // 60 ms

/// Index of the lowest-energy frame in `window`. `window` is never empty.
fn quietest_frame(window: &[f32]) -> usize {
    window
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_energy_vad_creation() {
        let config = VadConfig::default();
        let vad = EnergyVad::new(&config);
        assert!(vad.threshold > 0.0);
    }

    #[test]
    fn test_detect_silence() {
        let config = VadConfig::default();
        let vad = EnergyVad::new(&config);

        // Create 1 second of silence
        let silence: Vec<f32> = vec![0.0; 16000];
        let result = vad.detect(&silence).unwrap();

        assert!(!result.has_speech);
        assert_eq!(result.speech_duration_secs, 0.0);
        assert_eq!(result.rms_energy, 0.0);
    }

    #[test]
    fn test_detect_loud_audio() {
        let config = VadConfig::default();
        let vad = EnergyVad::new(&config);

        // Create 1 second of "loud" audio (sine wave)
        let samples: Vec<f32> = (0..16000)
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin() * 0.5)
            .collect();
        let result = vad.detect(&samples).unwrap();

        assert!(result.has_speech);
        assert!(result.speech_ratio > 0.9);
        assert!(result.rms_energy > 0.1);
    }

    #[test]
    fn test_detect_quiet_audio() {
        let config = VadConfig::default();
        let vad = EnergyVad::new(&config);

        // Create 1 second of very quiet audio
        let samples: Vec<f32> = (0..16000)
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin() * 0.001)
            .collect();
        let result = vad.detect(&samples).unwrap();

        // Very quiet audio should be detected as silence with default threshold
        assert!(!result.has_speech);
    }

    #[test]
    fn test_detect_empty_audio() {
        let config = VadConfig::default();
        let vad = EnergyVad::new(&config);

        let result = vad.detect(&[]).unwrap();
        assert!(!result.has_speech);
        assert_eq!(result.speech_duration_secs, 0.0);
    }

    #[test]
    fn test_threshold_mapping() {
        // Test threshold mapping function
        let low = map_threshold_to_energy(0.0);
        let mid = map_threshold_to_energy(0.5);
        let high = map_threshold_to_energy(1.0);

        assert!(low < mid);
        assert!(mid < high);
        assert!(low >= 0.001);
        assert!(high <= 0.1);
    }

    #[test]
    fn test_min_speech_duration() {
        let config = VadConfig {
            min_speech_duration_ms: 500, // 500ms minimum
            ..VadConfig::default()
        };
        let vad = EnergyVad::new(&config);

        // Create 200ms of loud audio followed by silence
        // This is less than the 500ms minimum
        let mut samples: Vec<f32> = (0..3200) // 200ms
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin() * 0.5)
            .collect();
        samples.extend(vec![0.0; 12800]); // 800ms silence

        let result = vad.detect(&samples).unwrap();

        // Should not pass because speech duration < min_speech_duration
        assert!(!result.has_speech);
    }

    #[test]
    fn test_calculate_rms() {
        // RMS of constant 1.0 should be 1.0
        let ones = vec![1.0f32; 100];
        assert!((EnergyVad::calculate_rms(&ones) - 1.0).abs() < 0.001);

        // RMS of constant 0.0 should be 0.0
        let zeros = vec![0.0f32; 100];
        assert_eq!(EnergyVad::calculate_rms(&zeros), 0.0);

        // RMS of sine wave with amplitude 1.0 should be ~0.707
        let sine: Vec<f32> = (0..1000)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI / 100.0).sin())
            .collect();
        let rms = EnergyVad::calculate_rms(&sine);
        assert!((rms - 0.707).abs() < 0.01);
    }

    /// 254 Hz tone at amplitude 0.3, `n` samples long.
    fn tone(n: usize) -> Vec<f32> {
        (0..n).map(|k| (k as f32 * 0.1).sin() * 0.3).collect()
    }

    #[test]
    fn split_cuts_on_the_pause_between_phrases() {
        // 1 s speech, 0.6 s pause, 1 s speech
        let mut audio = tone(16000);
        audio.extend(std::iter::repeat(0.0).take(9600));
        audio.extend(tone(16000));

        let segs = split_on_silence(&audio, 20.0, 400);

        assert_eq!(segs.len(), 2, "one segment per phrase, got {:?}", segs);
        assert!(segs[0].end < 25600, "first segment ran into the pause: {:?}", segs[0]);
        assert!(segs[1].start > 16000, "second segment started in the pause: {:?}", segs[1]);
    }

    #[test]
    fn split_caps_speech_that_never_pauses() {
        // 60 s without a single gap: the hard cap is the only thing that can
        // fire, and it must, or GigaAM gets a clip upstream refuses outright.
        let segs = split_on_silence(&tone(16000 * 60), 20.0, 400);

        assert!(segs.len() >= 3, "expected the cap to split, got {}", segs.len());
        for s in &segs {
            assert!(s.len() <= 16000 * 21, "segment over the 20 s cap: {:?}", s);
        }
    }

    #[test]
    fn split_returns_nothing_for_silence() {
        assert!(split_on_silence(&vec![0.0; 16000], 20.0, 400).is_empty());
    }

    /// Amplitude-modulated tone: loud and quiet stretches like real speech,
    /// but never an actual pause. This is the shape the first cut of the
    /// splitter shredded into four pieces mid-word, dropping the audio
    /// between them — the regression that matters most here.
    #[test]
    fn split_does_not_cut_continuous_speech() {
        let audio: Vec<f32> = (0..16000 * 10)
            .map(|k| {
                let t = k as f32 / 16000.0;
                let envelope = 0.02 + 0.06 * (0.5 + 0.5 * (t * 8.0).sin());
                (k as f32 * 0.1).sin() * envelope
            })
            .collect();

        let segs = split_on_silence(&audio, 20.0, 400);

        assert_eq!(segs.len(), 1, "continuous speech must stay whole, got {:?}", segs);
    }

    /// Segments must tile the speech span: whatever the threshold decides,
    /// no audio between the first and last spoken frame may go missing.
    #[test]
    fn split_never_drops_audio_between_segments() {
        let mut audio = tone(16000);
        audio.extend(std::iter::repeat(0.0).take(9600));
        audio.extend(tone(16000));

        let segs = split_on_silence(&audio, 20.0, 400);

        for pair in segs.windows(2) {
            assert_eq!(
                pair[0].end, pair[1].start,
                "gap between segments: {:?} then {:?}",
                pair[0], pair[1]
            );
        }
    }

    /// Drive the splitter the way a streaming session does — 100 ms at a
    /// time, floor measured over the whole session — and check that each
    /// pause commits its phrase.
    ///
    /// The bug this pins: the floor used to be measured over the *pending*
    /// buffer, which after a commit is a short speech-dense tail. Its 5th
    /// percentile landed inside speech, so the threshold shredded the next
    /// phrase into fragments that transcribed to nothing, and only the hard
    /// cap ever produced real text.
    #[test]
    fn streaming_commits_each_phrase_as_its_pause_arrives() {
        let mut audio = Vec::new();
        for _ in 0..3 {
            audio.extend(tone(16000 * 2));
            audio.extend(std::iter::repeat(0.0).take(8000)); // 0.5 s pause
        }

        let mut session_rms: Vec<f32> = Vec::new();
        let mut pending: Vec<f32> = Vec::new();
        let mut committed = 0usize;

        for chunk in audio.chunks(1600) {
            session_rms.extend(frame_rms(chunk));
            pending.extend_from_slice(chunk);

            let segs = split_on_silence_at(&pending, noise_floor(&session_rms), 8.0, 400);
            if segs.len() > 1 {
                committed += segs.len() - 1;
                let tail = segs.last().expect("checked len").start;
                pending.drain(..tail);
            }
        }

        // Two pauses close a phrase mid-session; the third is trailing
        // silence, and its phrase leaves with the end-of-stream flush.
        assert_eq!(committed, 2, "expected a commit per closed pause");
    }
}
