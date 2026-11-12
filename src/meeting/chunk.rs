//! Audio chunk processor for meeting transcription
//!
//! Handles splitting continuous audio into chunks, applying VAD,
//! and coordinating transcription.

use crate::error::TranscribeError;
use crate::meeting::data::{AudioSource, TranscriptSegment};
use crate::transcribe::Transcriber;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Configuration for chunk processing
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    /// Duration of each audio chunk in seconds
    pub chunk_duration_secs: u32,
    /// Minimum audio level to consider as speech (0.0 - 1.0)
    pub vad_threshold: f32,
    /// Sample rate (expected 16000 Hz)
    pub sample_rate: u32,
    /// Minimum chunk duration to process (in seconds)
    pub min_chunk_duration_secs: f32,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            chunk_duration_secs: 30,
            vad_threshold: 0.01,
            sample_rate: 16000,
            min_chunk_duration_secs: 0.5,
        }
    }
}

/// Audio buffer for a chunk being recorded
#[derive(Debug)]
pub struct ChunkBuffer {
    /// Audio samples (mono, f32)
    samples: Vec<f32>,
    /// Start time of this chunk
    started_at: Instant,
    /// Chunk ID
    chunk_id: u32,
    /// Audio source
    source: AudioSource,
    /// Start time offset in milliseconds (relative to meeting start)
    start_offset_ms: u64,
    /// Sample rate in Hz
    sample_rate: u32,
}

impl ChunkBuffer {
    /// Create a new chunk buffer
    pub fn new(chunk_id: u32, source: AudioSource, start_offset_ms: u64, sample_rate: u32) -> Self {
        Self {
            samples: Vec::with_capacity(sample_rate as usize * 30), // Pre-allocate for 30 seconds
            started_at: Instant::now(),
            chunk_id,
            source,
            start_offset_ms,
            sample_rate,
        }
    }

    /// Add audio samples to the buffer
    pub fn add_samples(&mut self, samples: &[f32]) {
        self.samples.extend_from_slice(samples);
    }

    /// Get the duration of audio in seconds
    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }

    /// Get the elapsed wall-clock time
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Take ownership of the samples, leaving buffer empty
    pub fn take_samples(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.samples)
    }

    /// Check if buffer has any audio
    pub fn has_audio(&self) -> bool {
        !self.samples.is_empty()
    }
}

/// Voice Activity Detection (VAD)
///
/// Simple energy-based VAD for filtering silent chunks.
/// Phase 3+ will use more sophisticated ML-based VAD.
pub struct VoiceActivityDetector {
    threshold: f32,
    sample_rate: u32,
    /// Window size for energy calculation in milliseconds
    window_ms: u32,
}

impl VoiceActivityDetector {
    /// Create a new VAD with the given threshold
    pub fn new(threshold: f32, sample_rate: u32) -> Self {
        Self {
            threshold,
            sample_rate,
            window_ms: 30,
        }
    }

    /// Check if the audio contains speech
    pub fn contains_speech(&self, samples: &[f32]) -> bool {
        if samples.is_empty() {
            return false;
        }

        // Calculate RMS energy over windows
        let window_size = (self.sample_rate * self.window_ms / 1000) as usize;
        if window_size == 0 {
            return false;
        }

        let speech_frames = samples
            .chunks(window_size)
            .filter(|chunk| Self::calculate_rms(chunk) > self.threshold)
            .count();
        let total_frames = samples.len() / window_size;
        if total_frames == 0 {
            return false;
        }

        // Speech in over 10% of the frames, or 300 ms of it in all. The
        // ratio alone dropped whole 30 s chunks holding one short sentence:
        // the opening "let's get started" while people join, or a "sounds
        // good" on the microphone track while the other side talks.
        speech_frames as f32 / total_frames as f32 > 0.1
            || speech_frames as u32 * self.window_ms >= MIN_SPEECH_MS
    }

    /// From the start of the first voiced analysis frame of `samples` to
    /// the end of the last, in samples; `None` when no frame is voiced.
    /// With the gate off (threshold 0) all of `samples` counts as voiced.
    pub fn voiced_span(&self, samples: &[f32]) -> Option<(usize, usize)> {
        if samples.is_empty() {
            return None;
        }
        if self.threshold <= 0.0 {
            return Some((0, samples.len()));
        }
        let window_size = ((self.sample_rate * self.window_ms / 1000) as usize).max(1);
        let voiced = |(_, chunk): &(usize, &[f32])| Self::calculate_rms(chunk) > self.threshold;
        let first = samples.chunks(window_size).enumerate().find(voiced)?.0;
        let last = samples.chunks(window_size).enumerate().rfind(voiced)?.0;
        Some((
            first * window_size,
            ((last + 1) * window_size).min(samples.len()),
        ))
    }

    /// Calculate RMS energy of samples
    fn calculate_rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum_squares: f32 = samples.iter().map(|s| s * s).sum();
        (sum_squares / samples.len() as f32).sqrt()
    }

    /// Get speech segments with their boundaries
    ///
    /// Returns a list of (start_sample, end_sample) tuples for speech regions.
    pub fn detect_speech_segments(&self, samples: &[f32]) -> Vec<(usize, usize)> {
        let window_size = (self.sample_rate * self.window_ms / 1000) as usize;
        if window_size == 0 || samples.is_empty() {
            return vec![];
        }

        let mut segments = vec![];
        let mut in_speech = false;
        let mut speech_start = 0;
        let mut silence_count = 0;
        let hangover = 5; // Number of silent frames to wait before ending segment

        for (i, chunk) in samples.chunks(window_size).enumerate() {
            let rms = Self::calculate_rms(chunk);
            let is_speech = rms > self.threshold;

            if is_speech {
                silence_count = 0;
                if !in_speech {
                    in_speech = true;
                    speech_start = i * window_size;
                }
            } else if in_speech {
                silence_count += 1;
                if silence_count >= hangover {
                    // End of speech segment
                    segments.push((speech_start, i * window_size));
                    in_speech = false;
                    silence_count = 0;
                }
            }
        }

        // Handle speech that extends to the end
        if in_speech {
            segments.push((speech_start, samples.len()));
        }

        segments
    }
}

/// Speech a chunk needs before it is transcribed at all.
const MIN_SPEECH_MS: u32 = 300;

/// How far before a whole-chunk engine's segment start, and after its end,
/// to look for the speech it timed. Whisper starts late less often than it
/// ends early, so the window leans forward.
const SEGMENT_SLACK_BEFORE: f32 = 0.2;
const SEGMENT_SLACK_AFTER: f32 = 0.3;

/// Silence kept around a fitted segment, like the pad on utterance regions.
const SEGMENT_PAD: f32 = 0.2;

/// Silence at least this long separates two utterances.
const UTTERANCE_GAP_SECS: f32 = 0.7;
/// Silence kept on each side of an utterance, so onsets and trailing
/// consonants below the energy threshold are not clipped.
const UTTERANCE_PAD_SECS: f32 = 0.2;
/// Shorter pieces (a click, a cough, a lone "mm") are folded into a
/// neighbour instead of being transcribed alone, where engines hallucinate.
const MIN_UTTERANCE_SECS: f32 = 0.6;

/// Split a chunk into utterances: sample ranges separated by pauses of at
/// least [`UTTERANCE_GAP_SECS`].
///
/// Only long silent runs are cut, at half the VAD threshold, so quiet
/// speech is never dropped: every sample outside a cut pause belongs to
/// some range. Returns the whole chunk when it has no such pause.
pub(crate) fn utterance_regions(
    samples: &[f32],
    sample_rate: u32,
    vad_threshold: f32,
) -> Vec<(usize, usize)> {
    let len = samples.len();
    let frame = (sample_rate as usize * 30 / 1000).max(1);
    let secs_to_samples = |secs: f32| (secs * sample_rate as f32) as usize;
    let min_gap = secs_to_samples(UTTERANCE_GAP_SECS);
    let pad = secs_to_samples(UTTERANCE_PAD_SECS);
    let silence = vad_threshold * 0.5;

    // Silent runs long enough to cut, as sample ranges.
    let mut gaps: Vec<(usize, usize)> = Vec::new();
    let mut run_start: Option<usize> = None;
    for (i, chunk) in samples.chunks(frame).enumerate() {
        let start = i * frame;
        if VoiceActivityDetector::calculate_rms(chunk) < silence {
            run_start.get_or_insert(start);
        } else if let Some(rs) = run_start.take() {
            if start - rs >= min_gap {
                gaps.push((rs, start));
            }
        }
    }
    if let Some(rs) = run_start {
        if len - rs >= min_gap {
            gaps.push((rs, len));
        }
    }

    // Utterances are what lies between the gaps, padded into them.
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0;
    for &(gap_start, gap_end) in &gaps {
        if gap_start > cursor {
            regions.push((cursor, gap_start));
        }
        cursor = gap_end;
    }
    if cursor < len {
        regions.push((cursor, len));
    }
    if regions.is_empty() {
        return vec![(0, len)];
    }
    for region in &mut regions {
        region.0 = region.0.saturating_sub(pad);
        region.1 = (region.1 + pad).min(len);
    }

    // Fold short pieces into the closer neighbour.
    let min_len = secs_to_samples(MIN_UTTERANCE_SECS);
    while regions.len() > 1 {
        let Some(i) = regions.iter().position(|r| r.1 - r.0 < min_len) else {
            break;
        };
        let gap_before = (i > 0).then(|| regions[i].0.saturating_sub(regions[i - 1].1));
        let gap_after = regions
            .get(i + 1)
            .map(|next| next.0.saturating_sub(regions[i].1));
        let merge_with_previous = match (gap_before, gap_after) {
            (Some(before), Some(after)) => before <= after,
            (Some(_), None) => true,
            _ => false,
        };
        if merge_with_previous {
            regions[i - 1].1 = regions[i].1;
            regions.remove(i);
        } else {
            regions[i + 1].0 = regions[i].0;
            regions.remove(i);
        }
    }

    regions
}

/// Shared silence that makes a good place to end a chunk.
const CUT_SILENCE_SECS: f32 = 0.3;

/// Where to end the next chunk of a meeting, given both tracks' buffered
/// audio and the nominal chunk length `target` in samples.
///
/// Ending a chunk mid-utterance splits it in two on both tracks. On the
/// microphone track that left the echo of a remote sentence as two
/// fragments, each too short for the bleed-through dedup to recognise, and
/// mangled the word the cut went through. So the chunk ends in the middle
/// of the latest stretch, in the second half of the chunk, where both
/// tracks are silent for [`CUT_SILENCE_SECS`]. With no such stretch it ends
/// at `target`, as it always did.
pub fn chunk_cut_point(
    mic: &[f32],
    loopback: &[f32],
    target: usize,
    sample_rate: u32,
    vad_threshold: f32,
) -> usize {
    let target = target.min(mic.len());
    let frame = (sample_rate as usize * 30 / 1000).max(1);
    let needed = ((CUT_SILENCE_SECS * sample_rate as f32) as usize).div_ceil(frame);
    let quiet = |track: &[f32], at: usize| {
        track
            .get(at..(at + frame).min(track.len()))
            .is_none_or(|f| f.is_empty() || VoiceActivityDetector::calculate_rms(f) < vad_threshold)
    };

    let mut run = 0;
    let mut end = target - target % frame;
    while end >= frame && end > target / 2 {
        let at = end - frame;
        if quiet(mic, at) && quiet(loopback, at) {
            run += 1;
            if run == needed {
                // Cut in the middle of the quiet stretch [at, at + run * frame).
                return at + run * frame / 2;
            }
        } else {
            run = 0;
        }
        end -= frame;
    }
    target
}

/// Processed chunk result
#[derive(Debug)]
pub struct ProcessedChunk {
    /// Chunk ID
    pub chunk_id: u32,
    /// Transcript segments from this chunk
    pub segments: Vec<TranscriptSegment>,
    /// Original audio duration in milliseconds
    pub audio_duration_ms: u64,
    /// Processing time in milliseconds
    pub processing_time_ms: u64,
}

/// Chunk processor
///
/// Coordinates audio buffering, VAD, and transcription for meeting mode.
pub struct ChunkProcessor {
    config: ChunkConfig,
    vad: VoiceActivityDetector,
    transcriber: Arc<dyn Transcriber>,
    next_segment_id: u32,
}

impl ChunkProcessor {
    /// Create a new chunk processor
    pub fn new(config: ChunkConfig, transcriber: Arc<dyn Transcriber>) -> Self {
        let vad = VoiceActivityDetector::new(config.vad_threshold, config.sample_rate);
        Self {
            config,
            vad,
            transcriber,
            next_segment_id: 0,
        }
    }

    /// Process a completed chunk of audio
    ///
    /// Applies VAD, transcribes speech regions, and returns transcript segments.
    pub fn process_chunk(
        &mut self,
        buffer: ChunkBuffer,
    ) -> Result<ProcessedChunk, TranscribeError> {
        let start_time = Instant::now();
        let chunk_id = buffer.chunk_id;
        let source = buffer.source;
        let start_offset_ms = buffer.start_offset_ms;

        let samples = buffer.samples;
        let sample_rate = self.config.sample_rate as f64;
        let audio_duration_ms = (samples.len() as f64 / sample_rate * 1000.0) as u64;

        // Skip if too short
        let min_samples =
            (self.config.min_chunk_duration_secs * self.config.sample_rate as f32) as usize;
        if samples.len() < min_samples {
            tracing::debug!(
                "Chunk {} too short ({:.2}s), skipping",
                chunk_id,
                samples.len() as f32 / self.config.sample_rate as f32
            );
            return Ok(ProcessedChunk {
                chunk_id,
                segments: vec![],
                audio_duration_ms,
                processing_time_ms: start_time.elapsed().as_millis() as u64,
            });
        }

        // Check for speech
        if !self.vad.contains_speech(&samples) {
            tracing::debug!(
                chunk_id,
                source = %source,
                duration_secs = samples.len() as f32 / self.config.sample_rate as f32,
                rms = VoiceActivityDetector::calculate_rms(&samples),
                threshold = self.config.vad_threshold,
                "Meeting chunk skipped: no speech detected"
            );
            return Ok(ProcessedChunk {
                chunk_id,
                segments: vec![],
                audio_duration_ms,
                processing_time_ms: start_time.elapsed().as_millis() as u64,
            });
        }

        // Transcribe each utterance on its own. Most engines return a single
        // untimed segment for whatever audio they are given, so transcribing
        // the whole chunk stamped every line of it with the chunk's start and
        // left nothing to order the two tracks by. Splitting at pauses gives
        // every utterance its own start time; on one track a pause is mostly
        // the other side talking, so the pieces are the turns. The Display
        // impl produces "You"/"Remote", matching the skip log above.
        //
        // Engines whose per-call cost does not shrink with the clip (Whisper,
        // a process or request per call) keep one call per chunk, timed by
        // whatever their transcribe_timed reports.
        let per_utterance = self.transcriber.short_clips_are_cheap();
        let regions = if per_utterance {
            utterance_regions(&samples, self.config.sample_rate, self.config.vad_threshold)
        } else {
            vec![(0, samples.len())]
        };
        tracing::info!(
            "Transcribing {} chunk {} ({:.1}s of audio, {} utterance(s))",
            source,
            chunk_id,
            samples.len() as f32 / self.config.sample_rate as f32,
            regions.len()
        );

        let mut segments = vec![];
        for (region_start, region_end) in regions {
            let region_offset_secs = region_start as f32 / self.config.sample_rate as f32;
            let timed_segments = self
                .transcriber
                .transcribe_timed(&samples[region_start..region_end])?;

            // Where the previous fitted segment ended, so a segment that
            // Whisper starts right at its neighbour's end cannot reach back
            // into the neighbour's speech.
            let mut prev_end = 0usize;
            for timed in &timed_segments {
                if timed.text.trim().is_empty() {
                    continue;
                }
                // A whole-chunk decode (Whisper) times its segments loosely:
                // a segment after a pause starts where the previous one
                // ended, so the pause is counted as speech, and it can put
                // words on a stretch with no sound at all (the familiar
                // "Thank you." over silence). Fit each segment to the voiced
                // audio inside its span, and drop it when there is none.
                let (start_secs, end_secs) = if per_utterance {
                    (timed.start_secs, timed.end_secs)
                } else {
                    let rate = self.config.sample_rate as f32;
                    let len = region_end - region_start;
                    let lo = (((timed.start_secs - SEGMENT_SLACK_BEFORE).max(0.0)) * rate) as usize;
                    let hi = (((timed.end_secs + SEGMENT_SLACK_AFTER) * rate) as usize).min(len);
                    let lo = lo.max(prev_end).min(hi);
                    let region = &samples[region_start..region_end];
                    match self.vad.voiced_span(&region[lo..hi]) {
                        None => {
                            tracing::debug!(
                                chunk_id,
                                text = %timed.text,
                                "Dropping a segment timed over silence"
                            );
                            continue;
                        }
                        Some((a, b)) => {
                            let pad = (SEGMENT_PAD * rate) as usize;
                            let from = (lo + a).saturating_sub(pad).max(lo);
                            let to = (lo + b + pad).min(hi);
                            prev_end = to;
                            (from as f32 / rate, to as f32 / rate)
                        }
                    }
                };

                let segment_id = self.next_segment_id;
                self.next_segment_id += 1;

                let seg_start_ms =
                    start_offset_ms + ((region_offset_secs + start_secs) * 1000.0) as u64;
                let seg_end_ms =
                    start_offset_ms + ((region_offset_secs + end_secs) * 1000.0) as u64;

                let mut segment = TranscriptSegment::new(
                    segment_id,
                    seg_start_ms,
                    seg_end_ms,
                    timed.text.clone(),
                    chunk_id,
                );
                segment.source = source;

                segments.push(segment);
            }
        }

        let processing_time_ms = start_time.elapsed().as_millis() as u64;
        tracing::debug!("Chunk {} processed in {}ms", chunk_id, processing_time_ms);

        Ok(ProcessedChunk {
            chunk_id,
            segments,
            audio_duration_ms,
            processing_time_ms,
        })
    }

    /// Number segments from `id` on. A meeting builds a processor per chunk,
    /// and every one starting at 0 gave each chunk's segments the same IDs.
    pub fn with_first_segment_id(mut self, id: u32) -> Self {
        self.next_segment_id = id;
        self
    }

    /// The ID the next segment will get.
    pub fn next_segment_id(&self) -> u32 {
        self.next_segment_id
    }

    /// Check if a chunk buffer is ready for processing
    pub fn is_chunk_ready(&self, buffer: &ChunkBuffer) -> bool {
        buffer.duration_secs() >= self.config.chunk_duration_secs as f32
    }

    /// Create a new chunk buffer
    pub fn new_buffer(
        &self,
        chunk_id: u32,
        source: AudioSource,
        start_offset_ms: u64,
    ) -> ChunkBuffer {
        ChunkBuffer::new(chunk_id, source, start_offset_ms, self.config.sample_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Behaves like most engines: one untimed segment for whatever it is
    /// given, via the trait's default `transcribe_timed`.
    struct UntimedTranscriber;

    impl Transcriber for UntimedTranscriber {
        fn short_clips_are_cheap(&self) -> bool {
            true
        }

        fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
            let voiced = samples.iter().any(|s| s.abs() > 0.05);
            Ok(if voiced {
                "words".to_string()
            } else {
                String::new()
            })
        }
    }

    /// `tone` seconds of 440 Hz at the given (start, end) spans, silence elsewhere.
    fn speech_at(total_secs: f32, spans: &[(f32, f32)]) -> Vec<f32> {
        let mut samples = create_silent_samples(total_secs);
        for &(start, end) in spans {
            let tone = create_test_samples(end - start, 440.0, 0.5);
            let at = (start * 16000.0) as usize;
            samples[at..at + tone.len()].copy_from_slice(&tone);
        }
        samples
    }

    /// Within one 30 ms analysis frame.
    fn near(actual: u64, expected: u64) -> bool {
        actual.abs_diff(expected) <= 30
    }

    fn assert_regions_near(regions: &[(usize, usize)], expected_secs: &[(f32, f32)]) {
        let as_ms: Vec<(u64, u64)> = regions
            .iter()
            .map(|&(s, e)| (s as u64 / 16, e as u64 / 16))
            .collect();
        assert_eq!(as_ms.len(), expected_secs.len(), "{:?}", as_ms);
        for (&(s, e), &(es, ee)) in as_ms.iter().zip(expected_secs) {
            assert!(
                near(s, (es * 1000.0) as u64) && near(e, (ee * 1000.0) as u64),
                "got {:?}, expected {:?} (seconds)",
                as_ms,
                expected_secs
            );
        }
    }

    /// Bug: every engine without native timestamps stamped the whole chunk
    /// with one start time, so its lines could not be ordered against the
    /// other track's.
    #[test]
    fn each_utterance_gets_its_own_timestamp() {
        let mut processor =
            ChunkProcessor::new(ChunkConfig::default(), Arc::new(UntimedTranscriber));
        let mut buffer = processor.new_buffer(3, AudioSource::Microphone, 90_000);
        buffer.add_samples(&speech_at(30.0, &[(2.0, 4.0), (10.0, 12.5), (25.0, 27.0)]));

        let result = processor.process_chunk(buffer).unwrap();
        // Each starts at its utterance less the 200 ms pad, offset by the chunk.
        let expected = [(91_800, 94_200), (99_800, 102_700), (114_800, 117_200)];
        assert_eq!(
            result.segments.len(),
            expected.len(),
            "{:?}",
            result.segments
        );
        for (seg, (start, end)) in result.segments.iter().zip(expected) {
            assert!(
                near(seg.start_ms, start) && near(seg.end_ms, end),
                "{:?}",
                result.segments
            );
            assert_eq!(seg.chunk_id, 3);
        }
    }

    /// Counts calls, and like Whisper says short clips are not cheaper.
    struct FixedCostTranscriber(std::sync::atomic::AtomicU32);

    impl Transcriber for FixedCostTranscriber {
        fn transcribe(&self, _samples: &[f32]) -> Result<String, TranscribeError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("words".to_string())
        }
    }

    #[test]
    fn fixed_cost_engines_get_one_call_per_chunk() {
        let transcriber = Arc::new(FixedCostTranscriber(Default::default()));
        let mut processor = ChunkProcessor::new(ChunkConfig::default(), transcriber.clone());
        let mut buffer = processor.new_buffer(0, AudioSource::Microphone, 0);
        buffer.add_samples(&speech_at(30.0, &[(2.0, 4.0), (10.0, 12.5), (25.0, 27.0)]));

        let result = processor.process_chunk(buffer).unwrap();
        assert_eq!(transcriber.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(result.segments.len(), 1);
    }

    /// Whisper's shape: one call per chunk (short clips are not cheaper),
    /// but the call returns its own timed segments.
    struct SegmentingTranscriber(std::sync::atomic::AtomicU32);

    impl Transcriber for SegmentingTranscriber {
        fn transcribe(&self, _samples: &[f32]) -> Result<String, TranscribeError> {
            unreachable!("meeting chunks use transcribe_timed")
        }

        fn transcribe_timed(
            &self,
            samples: &[f32],
        ) -> Result<Vec<crate::transcribe::TimedSegment>, TranscribeError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(samples.len(), 30 * 16000, "the whole chunk in one call");
            let seg = |text: &str, a: f32, b: f32| crate::transcribe::TimedSegment {
                text: text.to_string(),
                start_secs: a,
                end_secs: b,
            };
            Ok(vec![
                seg("We ship Friday.", 2.0, 4.0),
                seg(" ", 5.0, 6.0),
                // Whisper's way after a pause: starts where the last ended.
                seg("Agreed.", 4.0, 12.5),
                // Over pure silence: a hallucination, dropped.
                seg("Thank you.", 17.0, 19.0),
                seg("Bye.", 25.0, 27.0),
            ])
        }
    }

    /// Whisper meetings: one call per chunk, yet one line per utterance,
    /// each at its own time.
    #[test]
    fn a_segmenting_engine_gets_one_line_per_segment_from_one_call() {
        let transcriber = Arc::new(SegmentingTranscriber(Default::default()));
        let mut processor = ChunkProcessor::new(ChunkConfig::default(), transcriber.clone())
            .with_first_segment_id(7);
        let mut buffer = processor.new_buffer(2, AudioSource::Loopback, 60_000);
        buffer.add_samples(&speech_at(30.0, &[(2.0, 4.0), (10.0, 12.5), (25.0, 27.0)]));

        let result = processor.process_chunk(buffer).unwrap();
        assert_eq!(transcriber.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        let got: Vec<(u32, &str)> = result
            .segments
            .iter()
            .map(|s| (s.id, s.text.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![(7, "We ship Friday."), (8, "Agreed."), (9, "Bye.")]
        );
        // Each fitted to its own speech, 200 ms pad either side, offset by
        // the chunk: "Agreed." starts at 10 s, not where Whisper put it.
        let expected = [(61_800, 64_200), (69_800, 72_700), (84_800, 87_200)];
        for (seg, (start, end)) in result.segments.iter().zip(expected) {
            assert!(
                near(seg.start_ms, start) && near(seg.end_ms, end),
                "{:?}",
                result.segments
            );
        }
        assert!(result
            .segments
            .iter()
            .all(|s| s.source == AudioSource::Loopback));
    }

    /// The real Whisper model on real speech, when one is given:
    /// `VOXTYPE_TEST_WHISPER_MODEL=/path/ggml-tiny.en.bin cargo test whisper_meeting`.
    /// Two sentences eight seconds apart in one chunk become two lines at
    /// their own times, from one decode.
    #[test]
    fn whisper_meeting_chunk_gets_a_line_per_sentence() {
        let Ok(model) = std::env::var("VOXTYPE_TEST_WHISPER_MODEL") else {
            eprintln!("skipping: VOXTYPE_TEST_WHISPER_MODEL is not set");
            return;
        };
        let sentence: Vec<f32> = hound::WavReader::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vad/speech_long.wav"
        ))
        .unwrap()
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect();
        let mut audio = vec![0.0f32; 16000];
        audio.extend(&sentence);
        let second_at = audio.len() as f32 / 16000.0 + 8.0;
        audio.extend(vec![0.0f32; 8 * 16000]);
        audio.extend(&sentence);
        audio.resize(30 * 16000, 0.0);

        let config = crate::config::WhisperConfig {
            model,
            language: crate::config::LanguageConfig::Single("en".into()),
            ..Default::default()
        };
        let whisper = crate::transcribe::whisper::WhisperTranscriber::new(&config).unwrap();
        let mut processor = ChunkProcessor::new(ChunkConfig::default(), Arc::new(whisper));
        let mut buffer = processor.new_buffer(0, AudioSource::Loopback, 0);
        buffer.add_samples(&audio);
        let segments = processor.process_chunk(buffer).unwrap().segments;
        eprintln!("{segments:#?}");
        assert_eq!(segments.len(), 2, "{segments:?}");
        assert!(segments[0].start_ms.abs_diff(800) < 300, "{segments:?}");
        let expected = ((second_at - 0.2) * 1000.0) as u64;
        assert!(
            segments[1].start_ms.abs_diff(expected) < 300,
            "second line at {expected} ms: {segments:?}"
        );
    }

    /// Parakeet's own token times, when a model is given:
    /// `VOXTYPE_TEST_PARAKEET_MODEL=/path/parakeet-tdt-0.6b-v3-int8 cargo test
    /// --features parakeet parakeet_meeting -- --nocapture`. Two sentences
    /// 0.3 s apart are one utterance to the pause splitter; they come back as
    /// two lines, the second at its own start. With
    /// `VOXTYPE_TEST_MEETING_WAVS=a.wav:b.wav` it also prints, per file,
    /// the lines with and without the token times, and how much silence each
    /// line's start runs ahead of its speech.
    #[cfg(feature = "parakeet")]
    #[test]
    fn parakeet_meeting_lines_follow_its_token_times() {
        let Ok(model) = std::env::var("VOXTYPE_TEST_PARAKEET_MODEL") else {
            eprintln!("skipping: VOXTYPE_TEST_PARAKEET_MODEL is not set");
            return;
        };
        timed_lines_beat_one_line_per_utterance(Arc::new(
            crate::transcribe::parakeet::ParakeetTranscriber::new(&crate::config::ParakeetConfig {
                model,
                ..Default::default()
            })
            .unwrap(),
        ));
    }

    /// The same through voxtype-gguf-worker and transcribe.cpp's word times:
    /// `VOXTYPE_TEST_PARAKEET_GGUF_MODEL=/path/model.gguf
    /// VOXTYPE_GGUF_WORKER=target/debug/voxtype-gguf-worker cargo test
    /// --features parakeet-gguf parakeet_gguf_meeting -- --nocapture`.
    #[cfg(feature = "parakeet-gguf")]
    #[test]
    fn parakeet_gguf_meeting_lines_follow_its_word_times() {
        let Ok(model) = std::env::var("VOXTYPE_TEST_PARAKEET_GGUF_MODEL") else {
            eprintln!("skipping: VOXTYPE_TEST_PARAKEET_GGUF_MODEL is not set");
            return;
        };
        timed_lines_beat_one_line_per_utterance(Arc::new(
            crate::transcribe::gguf_subprocess::GgufSubprocessTranscriber::parakeet(
                &crate::config::ParakeetConfig {
                    model,
                    gguf_backend: "cpu".into(),
                    ..Default::default()
                },
            )
            .unwrap(),
        ));
    }

    #[cfg(any(feature = "parakeet", feature = "parakeet-gguf"))]
    fn timed_lines_beat_one_line_per_utterance(parakeet: Arc<dyn Transcriber>) {
        let read = |path: &str| -> Vec<f32> {
            hound::WavReader::open(path)
                .unwrap()
                .samples::<i16>()
                .map(|s| s.unwrap() as f32 / 32768.0)
                .collect()
        };
        /// The engine as meetings saw it before: one line per utterance.
        struct Untimed(Arc<dyn Transcriber>);
        impl Transcriber for Untimed {
            fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
                self.0.transcribe(samples)
            }
            fn short_clips_are_cheap(&self) -> bool {
                true
            }
        }
        let lines = |engine: Arc<dyn Transcriber>, track: &[f32]| {
            let mut processor = ChunkProcessor::new(ChunkConfig::default(), engine);
            let (mut out, mut at, mut id) = (Vec::new(), 0usize, 0u32);
            while at < track.len() {
                let rest = &track[at..];
                let cut = chunk_cut_point(rest, &[], 30 * 16000, 16000, 0.01).max(1);
                let mut buffer =
                    processor.new_buffer(id, AudioSource::Microphone, (at / 16) as u64);
                buffer.add_samples(&rest[..cut]);
                out.extend(processor.process_chunk(buffer).unwrap().segments);
                at += cut;
                id += 1;
            }
            out
        };
        // Silence between a line's start and its first voiced 30 ms frame.
        let lead_ms = |track: &[f32], seg: &TranscriptSegment| -> u64 {
            let from = (seg.start_ms * 16) as usize;
            track[from.min(track.len())..]
                .chunks(480)
                .position(|f| VoiceActivityDetector::calculate_rms(f) > 0.01)
                .map_or(0, |n| n as u64 * 30)
        };

        let sentence = read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vad/speech_long.wav"
        ));
        let mut audio = vec![0.0f32; 16000];
        audio.extend(&sentence);
        audio.extend(vec![0.0f32; 4800]);
        let second_at_ms = (audio.len() / 16) as u64;
        audio.extend(&sentence);
        audio.resize(15 * 16000, 0.0);
        let timed = lines(parakeet.clone(), &audio);
        let untimed = lines(Arc::new(Untimed(parakeet.clone())), &audio);
        eprintln!("timed: {timed:#?}\nuntimed: {untimed:#?}");
        assert_eq!(untimed.len(), 1, "one utterance to the pause splitter");
        assert!(timed.len() >= 2, "{timed:?}");
        assert!(
            timed
                .iter()
                .any(|s| s.start_ms.abs_diff(second_at_ms) < 400),
            "a line starts with the second sentence at {second_at_ms} ms: {timed:?}"
        );

        for path in std::env::var("VOXTYPE_TEST_MEETING_WAVS")
            .unwrap_or_default()
            .split(':')
            .filter(|p| !p.is_empty())
        {
            let track = read(path);
            for (name, engine) in [
                (
                    "untimed",
                    Arc::new(Untimed(parakeet.clone())) as Arc<dyn Transcriber>,
                ),
                ("timed", parakeet.clone()),
            ] {
                let segs = lines(engine, &track);
                let leads: Vec<u64> = segs.iter().map(|s| lead_ms(&track, s)).collect();
                eprintln!(
                    "{path} {name}: {} lines, lead silence mean {} ms, max {} ms",
                    segs.len(),
                    leads.iter().sum::<u64>() / leads.len().max(1) as u64,
                    leads.iter().max().unwrap_or(&0)
                );
                for (s, lead) in segs.iter().zip(&leads) {
                    eprintln!(
                        "  {:>6.2}-{:>6.2} lead {:>4} ms  {}",
                        s.start_ms as f32 / 1000.0,
                        s.end_ms as f32 / 1000.0,
                        lead,
                        s.text
                    );
                }
            }
        }
    }

    /// Bug: a chunk was transcribed only if over 10% of it was speech, so a
    /// 30 s chunk holding one two-second sentence was dropped whole.
    #[test]
    fn one_short_sentence_in_a_long_chunk_is_transcribed() {
        let mut processor =
            ChunkProcessor::new(ChunkConfig::default(), Arc::new(UntimedTranscriber));
        let mut buffer = processor.new_buffer(0, AudioSource::Microphone, 0);
        buffer.add_samples(&speech_at(30.0, &[(1.0, 3.0)]));

        let result = processor.process_chunk(buffer).unwrap();
        assert_eq!(result.segments.len(), 1, "sentence dropped");
        assert!(near(result.segments[0].start_ms, 800));
    }

    #[test]
    fn a_click_in_a_long_chunk_is_not_speech() {
        let vad = VoiceActivityDetector::new(0.01, 16000);
        assert!(!vad.contains_speech(&speech_at(30.0, &[(4.0, 4.1)])));
        assert!(vad.contains_speech(&speech_at(30.0, &[(4.0, 4.4)])));
    }

    #[test]
    fn chunks_end_in_a_pause_shared_by_both_tracks() {
        // The mic pauses at 6.5-7.5 s; the remote speaks 7.5-11 s across the
        // nominal 10 s boundary. The only shared pause in the second half
        // is 6.5-7.5, so the chunk ends there instead of mid-sentence.
        let mic = speech_at(12.0, &[(1.0, 6.5), (11.5, 12.0)]);
        let loopback = speech_at(12.0, &[(7.5, 11.0)]);
        let cut = chunk_cut_point(&mic, &loopback, 160_000, 16000, 0.01);
        assert!((104_000..=120_000).contains(&cut), "cut at {}", cut);
    }

    #[test]
    fn chunks_end_at_the_target_without_a_shared_pause() {
        // Someone is always talking from 4 s on.
        let mic = speech_at(12.0, &[(4.0, 8.0)]);
        let loopback = speech_at(12.0, &[(8.0, 12.0)]);
        assert_eq!(
            chunk_cut_point(&mic, &loopback, 160_000, 16000, 0.01),
            160_000
        );
    }

    #[test]
    fn chunks_prefer_the_latest_shared_pause() {
        let mic = speech_at(12.0, &[(5.0, 6.0), (7.0, 8.0)]);
        let loopback = speech_at(12.0, &[(8.5, 12.0)]);
        // Shared quiet: 6-7 s and 8-8.5 s; the later one wins.
        let cut = chunk_cut_point(&mic, &loopback, 160_000, 16000, 0.01);
        assert!((128_000..=136_000).contains(&cut), "cut at {}", cut);
    }

    #[test]
    fn chunks_without_loopback_cut_on_mic_pauses() {
        let mic = speech_at(12.0, &[(1.0, 8.0), (8.6, 12.0)]);
        let cut = chunk_cut_point(&mic, &[], 160_000, 16000, 0.01);
        assert!((128_000..=137_600).contains(&cut), "cut at {}", cut);
    }

    #[test]
    fn utterance_regions_cut_only_long_pauses() {
        // 0.3 s pause: one utterance. 1 s pause: two. The 0.5 s of leading
        // and trailing silence is too short to trim.
        let samples = speech_at(6.0, &[(0.5, 2.0), (2.3, 3.0), (4.0, 5.5)]);
        let regions = utterance_regions(&samples, 16000, 0.01);
        assert_regions_near(&regions, &[(0.0, 3.2), (3.8, 6.0)]);
    }

    #[test]
    fn utterance_regions_fold_short_blips_into_a_neighbour() {
        let samples = speech_at(8.0, &[(1.0, 3.0), (4.0, 4.1), (6.5, 7.5)]);
        let regions = utterance_regions(&samples, 16000, 0.01);
        // The 100 ms blip joins the utterance before it (1 s away vs 2.4 s).
        assert_regions_near(&regions, &[(0.8, 4.3), (6.3, 8.0)]);
    }

    #[test]
    fn utterance_regions_keep_the_whole_chunk_without_a_pause() {
        let samples = create_test_samples(5.0, 440.0, 0.5);
        assert_eq!(
            utterance_regions(&samples, 16000, 0.01),
            vec![(0, samples.len())]
        );
    }

    #[test]
    fn utterance_regions_keep_speech_too_quiet_for_the_vad_gate() {
        // RMS ~0.0057: under the 0.01 gate but over the 0.005 cut level.
        let quiet = create_test_samples(3.0, 440.0, 0.008);
        let regions = utterance_regions(&quiet, 16000, 0.01);
        assert_eq!(regions, vec![(0, quiet.len())]);
    }

    fn create_test_samples(duration_secs: f32, frequency_hz: f32, amplitude: f32) -> Vec<f32> {
        let sample_rate = 16000.0;
        let num_samples = (duration_secs * sample_rate) as usize;
        (0..num_samples)
            .map(|i| {
                let t = i as f32 / sample_rate;
                amplitude * (2.0 * std::f32::consts::PI * frequency_hz * t).sin()
            })
            .collect()
    }

    fn create_silent_samples(duration_secs: f32) -> Vec<f32> {
        let num_samples = (duration_secs * 16000.0) as usize;
        vec![0.0; num_samples]
    }

    #[test]
    fn test_chunk_buffer_duration() {
        let mut buffer = ChunkBuffer::new(0, AudioSource::Microphone, 0, 16000);
        buffer.add_samples(&vec![0.0; 16000]); // 1 second
        assert!((buffer.duration_secs() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_vad_silent_audio() {
        let vad = VoiceActivityDetector::new(0.01, 16000);
        let silent = create_silent_samples(1.0);
        assert!(!vad.contains_speech(&silent));
    }

    #[test]
    fn test_vad_speech_audio() {
        let vad = VoiceActivityDetector::new(0.01, 16000);
        let speech = create_test_samples(1.0, 440.0, 0.5);
        assert!(vad.contains_speech(&speech));
    }

    #[test]
    fn test_vad_detect_segments() {
        let vad = VoiceActivityDetector::new(0.01, 16000);

        // Create audio: silence, speech, silence
        let mut samples = create_silent_samples(0.5);
        samples.extend(create_test_samples(1.0, 440.0, 0.5));
        samples.extend(create_silent_samples(0.5));

        let segments = vad.detect_speech_segments(&samples);
        assert!(!segments.is_empty());

        // The speech should be detected roughly in the middle
        let (start, end) = segments[0];
        assert!(start > 0);
        assert!(end < samples.len());
    }

    #[test]
    fn test_chunk_config_default() {
        let config = ChunkConfig::default();
        assert_eq!(config.chunk_duration_secs, 30);
        assert_eq!(config.sample_rate, 16000);
    }

    #[test]
    fn test_rms_calculation() {
        let samples = vec![0.5, -0.5, 0.5, -0.5];
        let rms = VoiceActivityDetector::calculate_rms(&samples);
        assert!((rms - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_rms_empty() {
        let rms = VoiceActivityDetector::calculate_rms(&[]);
        assert_eq!(rms, 0.0);
    }

    #[test]
    fn test_chunk_buffer_empty() {
        let buffer = ChunkBuffer::new(0, AudioSource::Microphone, 0, 16000);
        assert!(!buffer.has_audio());
        assert!((buffer.duration_secs() - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_chunk_buffer_take_samples() {
        let mut buffer = ChunkBuffer::new(0, AudioSource::Microphone, 0, 16000);
        buffer.add_samples(&[0.1, 0.2, 0.3]);
        assert!(buffer.has_audio());

        let samples = buffer.take_samples();
        assert_eq!(samples.len(), 3);
        assert!(!buffer.has_audio());
    }

    #[test]
    fn test_chunk_buffer_multiple_adds() {
        let mut buffer = ChunkBuffer::new(0, AudioSource::Microphone, 0, 16000);
        buffer.add_samples(&vec![0.0; 8000]); // 0.5 seconds
        buffer.add_samples(&vec![0.0; 8000]); // 0.5 seconds
        assert!((buffer.duration_secs() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_chunk_buffer_elapsed() {
        let buffer = ChunkBuffer::new(0, AudioSource::Microphone, 0, 16000);
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(buffer.elapsed() >= std::time::Duration::from_millis(10));
    }

    #[test]
    fn test_vad_empty_samples() {
        let vad = VoiceActivityDetector::new(0.01, 16000);
        assert!(!vad.contains_speech(&[]));
    }

    #[test]
    fn test_vad_detect_segments_empty() {
        let vad = VoiceActivityDetector::new(0.01, 16000);
        let segments = vad.detect_speech_segments(&[]);
        assert!(segments.is_empty());
    }

    #[test]
    fn test_vad_detect_segments_all_silence() {
        let vad = VoiceActivityDetector::new(0.01, 16000);
        let silent = create_silent_samples(2.0);
        let segments = vad.detect_speech_segments(&silent);
        assert!(segments.is_empty());
    }

    #[test]
    fn test_vad_detect_segments_all_speech() {
        let vad = VoiceActivityDetector::new(0.01, 16000);
        let speech = create_test_samples(1.0, 440.0, 0.5);
        let segments = vad.detect_speech_segments(&speech);
        assert!(!segments.is_empty());
        // Speech covers the entire buffer
        let (start, end) = segments[0];
        assert_eq!(start, 0);
        assert_eq!(end, speech.len());
    }

    #[test]
    fn test_vad_threshold_boundary() {
        // Amplitude exactly at threshold should not be detected as speech
        // since RMS of a sine wave with amplitude A is A / sqrt(2)
        let threshold = 0.5;
        let vad = VoiceActivityDetector::new(threshold, 16000);

        // Very quiet audio (RMS below threshold)
        let quiet = create_test_samples(1.0, 440.0, 0.001);
        assert!(!vad.contains_speech(&quiet));

        // Loud audio (RMS above threshold)
        let loud = create_test_samples(1.0, 440.0, 1.0);
        assert!(vad.contains_speech(&loud));
    }

    #[test]
    fn test_vad_zero_sample_rate_no_panic() {
        // Edge case: zero sample rate should not panic
        let vad = VoiceActivityDetector::new(0.01, 0);
        assert!(!vad.contains_speech(&[0.5, 0.5, 0.5]));
        assert!(vad.detect_speech_segments(&[0.5, 0.5]).is_empty());
    }

    #[test]
    fn test_chunk_config_custom() {
        let config = ChunkConfig {
            chunk_duration_secs: 60,
            vad_threshold: 0.05,
            sample_rate: 48000,
            min_chunk_duration_secs: 1.0,
        };
        assert_eq!(config.chunk_duration_secs, 60);
        assert_eq!(config.sample_rate, 48000);
    }

    #[test]
    fn test_rms_uniform_value() {
        let samples = vec![0.3; 100];
        let rms = VoiceActivityDetector::calculate_rms(&samples);
        assert!((rms - 0.3).abs() < 0.01);
    }

    #[test]
    fn test_rms_single_sample() {
        let rms = VoiceActivityDetector::calculate_rms(&[0.7]);
        assert!((rms - 0.7).abs() < 0.01);
    }
}
