//! ML-based speaker diarization using ONNX Runtime
//!
//! Uses ECAPA-TDNN speaker embeddings for voice fingerprinting
//! and clustering to identify individual speakers.
//!
//! This module is only available with the `ml-diarization` feature.

use super::{DiarizationConfig, DiarizedSegment, Diarizer, SpeakerId};
use crate::meeting::data::AudioSource;
use crate::meeting::TranscriptSegment;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[cfg(feature = "ml-diarization")]
use ort::session::Session;
#[cfg(feature = "ml-diarization")]
use ort::value::Tensor;

/// Shortest segment, in ms, that may start a new speaker or move a
/// centroid. Shorter clips embed too noisily to be trusted with either.
pub const MIN_MINT_MS: u64 = 1_500;

/// Centroid similarity at or above which two speakers are merged. Matches of
/// one voice against its own centroid run about 0.6-0.9 with ECAPA, two
/// different people near 0.
pub const MERGE_SIMILARITY: f32 = 0.5;

/// Speaker embedding (voice fingerprint)
#[derive(Debug, Clone)]
pub struct SpeakerEmbedding {
    /// Embedding vector (typically 192 or 256 dimensions). Treated as a running
    /// centroid that is updated via online mean when new utterances cluster here.
    pub vector: Vec<f32>,
    /// Speaker ID this embedding belongs to
    pub speaker_id: SpeakerId,
    /// Number of embeddings merged into this centroid (for the online mean update)
    pub count: u32,
}

impl SpeakerEmbedding {
    /// The `n` of an `Auto(n)` speaker, for stable ordering.
    #[cfg(feature = "ml-diarization")]
    fn speaker_id_num(&self) -> u32 {
        match self.speaker_id {
            SpeakerId::Auto(n) => n,
            _ => u32::MAX,
        }
    }

    /// Cosine similarity with another embedding
    pub fn cosine_similarity(&self, other: &SpeakerEmbedding) -> f32 {
        if self.vector.len() != other.vector.len() {
            return 0.0;
        }

        let dot: f32 = self
            .vector
            .iter()
            .zip(other.vector.iter())
            .map(|(a, b)| a * b)
            .sum();

        let norm_a: f32 = self.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = other.vector.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot / (norm_a * norm_b)
    }
}

/// Mutable speaker tracking state, protected by Mutex for interior mutability.
/// This allows `Diarizer::diarize(&self, ...)` to update speaker state.
struct MlDiarizerState {
    /// Known speaker embeddings
    #[cfg(feature = "ml-diarization")]
    speaker_embeddings: Vec<SpeakerEmbedding>,
    /// Speaker labels (auto ID -> human label)
    speaker_labels: HashMap<u32, String>,
    /// Next speaker ID
    #[cfg(feature = "ml-diarization")]
    next_speaker_id: u32,
}

impl MlDiarizerState {
    fn new() -> Self {
        Self {
            #[cfg(feature = "ml-diarization")]
            speaker_embeddings: Vec::new(),
            speaker_labels: HashMap::new(),
            #[cfg(feature = "ml-diarization")]
            next_speaker_id: 0,
        }
    }

    /// The cluster an embedding is most similar to, with that similarity.
    #[cfg(feature = "ml-diarization")]
    fn nearest(&self, embedding: &[f32]) -> Option<(usize, f32)> {
        let probe = SpeakerEmbedding {
            vector: embedding.to_vec(),
            speaker_id: SpeakerId::Unknown,
            count: 1,
        };
        self.speaker_embeddings
            .iter()
            .enumerate()
            .map(|(i, e)| (i, probe.cosine_similarity(e)))
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
    }

    /// Fold an embedding into a centroid as an online mean. Cosine
    /// similarity is scale-invariant, so the vector is not renormalized.
    #[cfg(feature = "ml-diarization")]
    fn absorb(&mut self, idx: usize, embedding: &[f32]) {
        let existing = &mut self.speaker_embeddings[idx];
        let n = existing.count as f32;
        for (c, e) in existing.vector.iter_mut().zip(embedding.iter()) {
            *c = (*c * n + *e) / (n + 1.0);
        }
        existing.count += 1;
    }

    /// The cluster that has absorbed the most speech.
    #[cfg(feature = "ml-diarization")]
    fn dominant(&self) -> Option<usize> {
        self.speaker_embeddings
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.count.cmp(&b.1.count).then(b.0.cmp(&a.0)))
            .map(|(i, _)| i)
    }

    /// Give one transcript segment a speaker from its sub-window embeddings.
    ///
    /// A segment is labelled as a whole, and only long, mostly unfamiliar
    /// speech may start a new speaker. Minting per sub-window, as this used
    /// to, split one voice on a call into several speakers: a sub-second
    /// "Yeah." embeds noisily and fell under the threshold, and a single odd
    /// 4 s window of a long turn created a cluster even when the rest of the
    /// turn matched an existing one.
    ///
    /// - Matched sub-windows vote, and the winner absorbs them if the segment
    ///   is at least [`MIN_MINT_MS`] long.
    /// - A new speaker needs a segment of at least [`MIN_MINT_MS`] whose
    ///   sub-windows are mostly unmatched; it starts from their mean.
    /// - Anything else (a short clip, no majority, the speaker cap reached)
    ///   joins the nearest speaker when it is at all similar, else the
    ///   dominant one, and changes no centroid.
    #[cfg(feature = "ml-diarization")]
    fn assign_segment(
        &mut self,
        embeddings: &[Vec<f32>],
        duration_ms: u64,
        similarity_threshold: f32,
        max_speakers: u32,
    ) -> SpeakerId {
        if embeddings.is_empty() {
            return SpeakerId::Unknown;
        }
        let long_enough = duration_ms >= MIN_MINT_MS;

        let mut votes: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut unmatched: Vec<usize> = Vec::new();
        for (w, e) in embeddings.iter().enumerate() {
            match self.nearest(e) {
                Some((i, sim)) if sim > similarity_threshold => votes.entry(i).or_default().push(w),
                _ => unmatched.push(w),
            }
        }

        if long_enough
            && unmatched.len() * 2 > embeddings.len()
            && self.next_speaker_id < max_speakers
        {
            let dim = embeddings[unmatched[0]].len();
            let mut mean = vec![0.0f32; dim];
            for &w in &unmatched {
                for (m, x) in mean.iter_mut().zip(&embeddings[w]) {
                    *m += x / unmatched.len() as f32;
                }
            }
            let speaker_id = SpeakerId::Auto(self.next_speaker_id);
            tracing::debug!(
                "New speaker {} from {} of {} sub-window(s), {} ms",
                speaker_id,
                unmatched.len(),
                embeddings.len(),
                duration_ms
            );
            self.speaker_embeddings.push(SpeakerEmbedding {
                vector: mean,
                speaker_id: speaker_id.clone(),
                count: unmatched.len() as u32,
            });
            self.next_speaker_id += 1;
            return speaker_id;
        }

        // Most votes; ties go to the earlier speaker so results repeat.
        let winner = votes
            .iter()
            .max_by(|a, b| {
                a.1.len().cmp(&b.1.len()).then(
                    self.speaker_embeddings[*b.0]
                        .speaker_id_num()
                        .cmp(&self.speaker_embeddings[*a.0].speaker_id_num()),
                )
            })
            .map(|(i, ws)| (*i, ws.clone()));
        if let Some((idx, windows)) = winner {
            if long_enough {
                for w in windows {
                    self.absorb(idx, &embeddings[w]);
                }
            }
            return self.speaker_embeddings[idx].speaker_id.clone();
        }

        let mean_nearest = embeddings
            .iter()
            .filter_map(|e| self.nearest(e))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let idx = match mean_nearest {
            Some((i, sim)) if sim > 0.0 => Some(i),
            _ => self.dominant(),
        };
        idx.map(|i| self.speaker_embeddings[i].speaker_id.clone())
            .unwrap_or(SpeakerId::Unknown)
    }

    /// Merge speakers whose centroids have come to sound alike, folding the
    /// later one into the earlier. Returns `(merged, kept)` pairs in order.
    #[cfg(feature = "ml-diarization")]
    fn merge_similar(&mut self, merge_threshold: f32) -> Vec<(SpeakerId, SpeakerId)> {
        let mut merged = Vec::new();
        loop {
            let mut best: Option<(usize, usize, f32)> = None;
            for a in 0..self.speaker_embeddings.len() {
                for b in (a + 1)..self.speaker_embeddings.len() {
                    let sim =
                        self.speaker_embeddings[a].cosine_similarity(&self.speaker_embeddings[b]);
                    if sim >= merge_threshold && best.is_none_or(|(_, _, s)| sim > s) {
                        best = Some((a, b, sim));
                    }
                }
            }
            let Some((keep, drop, sim)) = best else {
                return merged;
            };
            let gone = self.speaker_embeddings.remove(drop);
            let kept = &mut self.speaker_embeddings[keep];
            let (n, m) = (kept.count as f32, gone.count as f32);
            for (c, g) in kept.vector.iter_mut().zip(&gone.vector) {
                *c = (*c * n + g * m) / (n + m);
            }
            kept.count += gone.count;
            tracing::debug!(
                "Merged speaker {} into {} (centroid similarity {:.3})",
                gone.speaker_id,
                kept.speaker_id,
                sim
            );
            merged.push((gone.speaker_id, kept.speaker_id.clone()));
        }
    }
}

/// ML-based speaker diarizer
pub struct MlDiarizer {
    /// Path to the ONNX model file
    model_path: Option<PathBuf>,
    /// ONNX session (lazy loaded)
    #[cfg(feature = "ml-diarization")]
    session: Option<Mutex<Session>>,
    /// Mutable speaker state behind Mutex for interior mutability
    state: Mutex<MlDiarizerState>,
    /// Similarity threshold for matching speakers
    #[cfg(feature = "ml-diarization")]
    similarity_threshold: f32,
    /// Maximum number of speakers to detect
    #[cfg(feature = "ml-diarization")]
    max_speakers: u32,
    /// Minimum segment duration for embedding (ms)
    #[cfg(feature = "ml-diarization")]
    min_segment_ms: u64,
    /// VAD sub-window length in seconds for ECAPA feeding
    #[cfg(feature = "ml-diarization")]
    vad_window_secs: f32,
    /// VAD sub-window hop in seconds
    #[cfg(feature = "ml-diarization")]
    vad_hop_secs: f32,
    /// RMS floor below which a sub-window is treated as silence
    #[cfg(feature = "ml-diarization")]
    vad_rms_floor: f32,
    /// Sample rate for audio
    #[cfg(feature = "ml-diarization")]
    sample_rate: u32,
}

impl MlDiarizer {
    /// Create a new ML diarizer
    pub fn new(config: &DiarizationConfig) -> Self {
        Self {
            model_path: config.model_path.as_ref().map(PathBuf::from),
            #[cfg(feature = "ml-diarization")]
            session: None,
            state: Mutex::new(MlDiarizerState::new()),
            #[cfg(feature = "ml-diarization")]
            similarity_threshold: config.similarity_threshold,
            #[cfg(feature = "ml-diarization")]
            max_speakers: config.max_speakers,
            #[cfg(feature = "ml-diarization")]
            min_segment_ms: config.min_segment_ms,
            #[cfg(feature = "ml-diarization")]
            vad_window_secs: config.vad_window_secs,
            #[cfg(feature = "ml-diarization")]
            vad_hop_secs: config.vad_hop_secs,
            #[cfg(feature = "ml-diarization")]
            vad_rms_floor: config.vad_rms_floor,
            #[cfg(feature = "ml-diarization")]
            sample_rate: 16000,
        }
    }

    /// Get or create default model path
    pub fn default_model_path() -> PathBuf {
        let data_dir = crate::config::Config::data_dir();
        data_dir.join("models").join("ecapa_tdnn.onnx")
    }

    /// Check if the model file exists
    pub fn model_exists(&self) -> bool {
        self.model_path
            .as_ref()
            .map(|p| p.exists())
            .unwrap_or_else(|| Self::default_model_path().exists())
    }

    /// Load the ONNX model
    #[cfg(feature = "ml-diarization")]
    pub fn load_model(&mut self) -> Result<(), String> {
        let path = self
            .model_path
            .clone()
            .unwrap_or_else(Self::default_model_path);

        if !path.exists() {
            return Err(format!(
                "Speaker embedding model not found: {:?}\n\
                Download from: https://huggingface.co/speechbrain/spkrec-ecapa-voxceleb\n\
                Place in: {:?}",
                path, path
            ));
        }

        match Session::builder() {
            Ok(mut builder) => match builder.commit_from_file(&path) {
                Ok(session) => {
                    self.session = Some(Mutex::new(session));
                    tracing::info!("Loaded speaker embedding model: {:?}", path);
                    Ok(())
                }
                Err(e) => Err(format!("Failed to load model: {}", e)),
            },
            Err(e) => Err(format!("Failed to create ONNX session: {}", e)),
        }
    }

    /// Extract embedding from audio samples
    #[cfg(feature = "ml-diarization")]
    pub fn extract_embedding(&self, samples: &[f32]) -> Result<Vec<f32>, String> {
        let mutex = self.session.as_ref().ok_or("Model not loaded")?;
        let mut session = mutex
            .lock()
            .map_err(|e| format!("Session lock poisoned: {}", e))?;

        // Prepare input tensor: [batch=1, samples]
        let input_tensor = Tensor::<f32>::from_array(([1usize, samples.len()], samples.to_vec()))
            .map_err(|e| format!("Failed to create input tensor: {}", e))?;

        // Run inference
        let outputs = session
            .run(ort::inputs![input_tensor])
            .map_err(|e| format!("Inference failed: {}", e))?;

        // Extract embedding from output - try "embedding" key, then "output", then first output
        let output = outputs
            .get("embedding")
            .or_else(|| outputs.get("output"))
            .ok_or("No output from model")?;

        let (_shape, embedding_data) = output
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Failed to extract tensor: {}", e))?;

        Ok(embedding_data.to_vec())
    }

    /// Label a speaker
    pub fn label_speaker(&self, auto_id: u32, label: String) {
        if let Ok(mut state) = self.state.lock() {
            state.speaker_labels.insert(auto_id, label);
        }
    }

    /// Get speaker label if set
    pub fn get_label(&self, speaker_id: &SpeakerId) -> Option<String> {
        let state = self.state.lock().ok()?;
        match speaker_id {
            SpeakerId::Auto(id) => state.speaker_labels.get(id).cloned(),
            _ => None,
        }
    }

    #[cfg(all(test, feature = "ml-diarization"))]
    fn samples_to_ms(&self, samples: usize) -> u64 {
        (samples as u64 * 1000) / self.sample_rate as u64
    }
}

impl Default for MlDiarizer {
    fn default() -> Self {
        Self::new(&DiarizationConfig::default())
    }
}

impl Diarizer for MlDiarizer {
    fn diarize(
        &self,
        samples: &[f32],
        _source: AudioSource,
        chunk_start_ms: u64,
        transcript_segments: &[TranscriptSegment],
    ) -> Vec<DiarizedSegment> {
        // If model is not loaded or feature is disabled, fall back to simple attribution
        #[cfg(not(feature = "ml-diarization"))]
        {
            let _ = (samples, chunk_start_ms);
            transcript_segments
                .iter()
                .map(|seg| DiarizedSegment {
                    speaker: SpeakerId::Unknown,
                    start_ms: seg.start_ms,
                    end_ms: seg.end_ms,
                    text: seg.text.clone(),
                    confidence: 0.0,
                })
                .collect()
        }

        #[cfg(feature = "ml-diarization")]
        {
            if self.session.is_none() {
                tracing::warn!("ML diarizer model not loaded, using unknown speaker");
                return transcript_segments
                    .iter()
                    .map(|seg| DiarizedSegment {
                        speaker: SpeakerId::Unknown,
                        start_ms: seg.start_ms,
                        end_ms: seg.end_ms,
                        text: seg.text.clone(),
                        confidence: 0.0,
                    })
                    .collect();
            }

            let mut results = Vec::new();

            for seg in transcript_segments {
                // Skip segments that are too short for reliable embedding
                if seg.duration_ms() < self.min_segment_ms {
                    results.push(DiarizedSegment {
                        speaker: SpeakerId::Unknown,
                        start_ms: seg.start_ms,
                        end_ms: seg.end_ms,
                        text: seg.text.clone(),
                        confidence: 0.0,
                    });
                    continue;
                }

                // Segment times are meeting-relative and the samples are this
                // chunk's. The chunk start comes from the caller: deriving it
                // from the first segment shifted every window by however long
                // the chunk was silent before its first word.
                let Some((start_sample, end_sample)) = super::segment_sample_range(
                    seg,
                    chunk_start_ms,
                    self.sample_rate,
                    samples.len(),
                ) else {
                    results.push(DiarizedSegment {
                        speaker: SpeakerId::Unknown,
                        start_ms: seg.start_ms,
                        end_ms: seg.end_ms,
                        text: seg.text.clone(),
                        confidence: 0.0,
                    });
                    continue;
                };

                let segment_samples = &samples[start_sample..end_sample];

                // ECAPA-TDNN is trained on ~2-5s utterances. Feeding whole
                // Soniox-style 30s mega-segments produces noisy averaged
                // embeddings that fail to cluster. Split into VAD-gated sub-windows
                // (length/hop/floor configurable) and pick the dominant per-segment label.
                let subwindows = super::vad_subwindows(
                    segment_samples,
                    self.sample_rate,
                    self.vad_window_secs,
                    self.vad_hop_secs,
                    self.vad_rms_floor,
                );

                // Extract all sub-window embeddings without holding the state
                // lock — ECAPA inference is the slow step and shouldn't block
                // any concurrent reader of speaker_embeddings. Fall back to a
                // single whole-segment window when no voiced sub-window is
                // found (very short or quiet segment).
                let windows = if subwindows.is_empty() {
                    vec![(0usize, segment_samples.len(), 0.0f32)]
                } else {
                    subwindows
                };
                let mut embeddings: Vec<Vec<f32>> = Vec::with_capacity(windows.len());
                for (ws, we, _rms) in windows {
                    match self.extract_embedding(&segment_samples[ws..we]) {
                        Ok(embedding) => embeddings.push(embedding),
                        Err(e) => tracing::warn!("Failed to extract embedding: {}", e),
                    }
                }

                let speaker = match self.state.lock() {
                    Ok(mut state) => state.assign_segment(
                        &embeddings,
                        seg.duration_ms(),
                        self.similarity_threshold,
                        self.max_speakers,
                    ),
                    Err(e) => {
                        tracing::warn!("Speaker state lock poisoned: {}", e);
                        SpeakerId::Unknown
                    }
                };

                let confidence = if matches!(speaker, SpeakerId::Unknown) {
                    0.0
                } else {
                    0.8
                };
                results.push(DiarizedSegment {
                    speaker,
                    start_ms: seg.start_ms,
                    end_ms: seg.end_ms,
                    text: seg.text.clone(),
                    confidence,
                });
            }

            results
        }
    }

    fn merge_similar_speakers(&self) -> Vec<(SpeakerId, SpeakerId)> {
        #[cfg(feature = "ml-diarization")]
        {
            self.state
                .lock()
                .map(|mut state| state.merge_similar(MERGE_SIMILARITY))
                .unwrap_or_default()
        }
        #[cfg(not(feature = "ml-diarization"))]
        Vec::new()
    }

    fn name(&self) -> &'static str {
        "ml"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = SpeakerEmbedding {
            vector: vec![1.0, 0.0, 0.0],
            speaker_id: SpeakerId::Auto(0),
            count: 1,
        };
        let b = SpeakerEmbedding {
            vector: vec![1.0, 0.0, 0.0],
            speaker_id: SpeakerId::Auto(1),
            count: 1,
        };
        assert!((a.cosine_similarity(&b) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = SpeakerEmbedding {
            vector: vec![1.0, 0.0, 0.0],
            speaker_id: SpeakerId::Auto(0),
            count: 1,
        };
        let b = SpeakerEmbedding {
            vector: vec![0.0, 1.0, 0.0],
            speaker_id: SpeakerId::Auto(1),
            count: 1,
        };
        assert!(a.cosine_similarity(&b).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = SpeakerEmbedding {
            vector: vec![1.0, 0.0, 0.0],
            speaker_id: SpeakerId::Auto(0),
            count: 1,
        };
        let b = SpeakerEmbedding {
            vector: vec![-1.0, 0.0, 0.0],
            speaker_id: SpeakerId::Auto(1),
            count: 1,
        };
        assert!((a.cosine_similarity(&b) + 1.0).abs() < 0.001);
    }

    #[test]
    fn test_speaker_labeling() {
        let diarizer = MlDiarizer::default();
        diarizer.label_speaker(0, "Alice".to_string());
        diarizer.label_speaker(1, "Bob".to_string());

        assert_eq!(
            diarizer.get_label(&SpeakerId::Auto(0)),
            Some("Alice".to_string())
        );
        assert_eq!(
            diarizer.get_label(&SpeakerId::Auto(1)),
            Some("Bob".to_string())
        );
        assert_eq!(diarizer.get_label(&SpeakerId::Auto(2)), None);
    }

    #[test]
    fn test_default_model_path() {
        let path = MlDiarizer::default_model_path();
        assert!(path.ends_with("ecapa_tdnn.onnx"));
    }

    /// Unit vectors standing in for ECAPA embeddings: `voice(a)` and
    /// `voice(b)` are the same speaker when a == b, and orthogonal otherwise.
    #[cfg(feature = "ml-diarization")]
    fn voice(n: usize) -> Vec<f32> {
        let mut v = vec![0.0; 8];
        v[n] = 1.0;
        v
    }

    /// Like `voice(n)` but noisier, as a sub-second clip embeds: it leans
    /// toward the speaker without clearing the 0.25 threshold.
    #[cfg(feature = "ml-diarization")]
    fn mumble(n: usize, other: usize) -> Vec<f32> {
        let mut v = vec![0.0; 8];
        v[n] = 0.2;
        v[other] = 1.0;
        v
    }

    #[test]
    #[cfg(feature = "ml-diarization")]
    fn long_unfamiliar_speech_starts_a_speaker_and_matching_speech_joins_it() {
        let mut s = MlDiarizerState::new();
        assert_eq!(
            s.assign_segment(&[voice(0), voice(0)], 6_000, 0.25, 10),
            SpeakerId::Auto(0)
        );
        assert_eq!(
            s.assign_segment(&[voice(0)], 3_000, 0.25, 10),
            SpeakerId::Auto(0)
        );
        assert_eq!(
            s.assign_segment(&[voice(1), voice(1)], 8_000, 0.25, 10),
            SpeakerId::Auto(1)
        );
        assert_eq!(
            s.speaker_embeddings[0].count, 3,
            "matching long speech moves the centroid"
        );
    }

    /// The Chris meeting: one Zoom voice, whose "Yeah." and "Okay." embed
    /// far from his long turns. Short clips must join him, never mint.
    #[test]
    #[cfg(feature = "ml-diarization")]
    fn short_clips_never_start_a_speaker() {
        let mut s = MlDiarizerState::new();
        assert_eq!(
            s.assign_segment(&[voice(0), voice(0)], 9_000, 0.25, 10),
            SpeakerId::Auto(0)
        );
        for _ in 0..20 {
            assert_eq!(
                s.assign_segment(&[mumble(0, 5)], 900, 0.25, 10),
                SpeakerId::Auto(0)
            );
        }
        assert_eq!(s.speaker_embeddings.len(), 1);
        assert_eq!(
            s.speaker_embeddings[0].count, 2,
            "short clips leave centroids alone"
        );
    }

    #[test]
    #[cfg(feature = "ml-diarization")]
    fn a_short_clip_joins_the_nearest_speaker_or_else_the_dominant_one() {
        let mut s = MlDiarizerState::new();
        s.assign_segment(&[voice(0), voice(0), voice(0)], 9_000, 0.25, 10);
        s.assign_segment(&[voice(1), voice(1)], 6_000, 0.25, 10);
        // Leans toward speaker 1 without matching it.
        assert_eq!(
            s.assign_segment(&[mumble(1, 6)], 800, 0.25, 10),
            SpeakerId::Auto(1)
        );
        // Like neither: the speaker with the most speech.
        assert_eq!(
            s.assign_segment(&[voice(7)], 800, 0.25, 10),
            SpeakerId::Auto(0)
        );
        // Nobody yet: unknown, never a new speaker.
        let mut empty = MlDiarizerState::new();
        assert_eq!(
            empty.assign_segment(&[voice(3)], 800, 0.25, 10),
            SpeakerId::Unknown
        );
        assert!(empty.speaker_embeddings.is_empty());
    }

    /// One odd 4 s window in a long turn used to start a speaker that then
    /// lay in wait (Remote 5 in the Chris meeting).
    #[test]
    #[cfg(feature = "ml-diarization")]
    fn a_dissenting_sub_window_does_not_start_a_speaker() {
        let mut s = MlDiarizerState::new();
        s.assign_segment(&[voice(0), voice(0)], 8_000, 0.25, 10);
        let turn = [voice(0), voice(4), voice(0), voice(0)];
        assert_eq!(
            s.assign_segment(&turn, 14_000, 0.25, 10),
            SpeakerId::Auto(0)
        );
        assert_eq!(s.speaker_embeddings.len(), 1);
    }

    #[test]
    #[cfg(feature = "ml-diarization")]
    fn the_speaker_cap_holds() {
        let mut s = MlDiarizerState::new();
        s.assign_segment(&[voice(0)], 5_000, 0.75, 2);
        s.assign_segment(&[voice(1)], 5_000, 0.75, 2);
        let third = s.assign_segment(&[voice(2)], 5_000, 0.75, 2);
        assert_ne!(third, SpeakerId::Auto(2));
        assert_eq!(s.speaker_embeddings.len(), 2);
    }

    #[test]
    #[cfg(feature = "ml-diarization")]
    fn centroids_are_online_means() {
        let mut s = MlDiarizerState::new();
        s.assign_segment(&[vec![1.0, 0.0, 0.0]], 5_000, 0.5, 10);
        s.assign_segment(&[vec![1.0, 1.0, 0.0]], 5_000, 0.5, 10);
        let c = &s.speaker_embeddings[0].vector;
        assert!(
            (c[0] - 1.0).abs() < 1e-5 && (c[1] - 0.5).abs() < 1e-5,
            "{c:?}"
        );
        assert_eq!(s.speaker_embeddings[0].count, 2);
    }

    #[test]
    #[cfg(feature = "ml-diarization")]
    fn speakers_that_converge_are_merged_into_the_earlier_one() {
        let mut s = MlDiarizerState::new();
        s.assign_segment(&[voice(0)], 5_000, 0.9, 10);
        s.assign_segment(
            &[vec![0.6, 0.8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]],
            5_000,
            0.9,
            10,
        );
        s.assign_segment(&[voice(3)], 5_000, 0.9, 10);
        assert_eq!(s.speaker_embeddings.len(), 3);

        let merged = s.merge_similar(MERGE_SIMILARITY);
        assert_eq!(merged, vec![(SpeakerId::Auto(1), SpeakerId::Auto(0))]);
        assert_eq!(s.speaker_embeddings.len(), 2);
        // Two different people stay apart, and nothing merges twice.
        assert!(s.merge_similar(MERGE_SIMILARITY).is_empty());
        // New speech from the merged voice lands on the survivor.
        assert_eq!(
            s.assign_segment(
                &[vec![0.6, 0.8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]],
                5_000,
                0.5,
                10
            ),
            SpeakerId::Auto(0)
        );
    }

    #[test]
    #[cfg(feature = "ml-diarization")]
    fn test_samples_to_ms() {
        let diarizer = MlDiarizer::default();
        assert_eq!(diarizer.samples_to_ms(16000), 1000);
        assert_eq!(diarizer.samples_to_ms(8000), 500);
        assert_eq!(diarizer.samples_to_ms(0), 0);
    }
}
