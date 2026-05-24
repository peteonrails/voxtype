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

    /// Find or create speaker ID for an embedding.
    ///
    /// On match, the existing centroid is updated with an online mean of the
    /// new embedding (each centroid tracks `count` merged samples). This
    /// improves robustness to within-speaker variation across long recordings
    /// versus anchoring on a single first-utterance embedding.
    #[cfg(feature = "ml-diarization")]
    fn find_or_create_speaker(
        &mut self,
        embedding: &[f32],
        similarity_threshold: f32,
        max_speakers: u32,
    ) -> SpeakerId {
        let new_embedding = SpeakerEmbedding {
            vector: embedding.to_vec(),
            speaker_id: SpeakerId::Auto(self.next_speaker_id),
            count: 1,
        };

        // Find best matching existing speaker
        let mut best_match: Option<(usize, f32)> = None;
        for (i, existing) in self.speaker_embeddings.iter().enumerate() {
            let similarity = new_embedding.cosine_similarity(existing);
            if similarity > similarity_threshold {
                match best_match {
                    None => best_match = Some((i, similarity)),
                    Some((_, best_sim)) if similarity > best_sim => {
                        best_match = Some((i, similarity))
                    }
                    _ => {}
                }
            }
        }

        if let Some((idx, sim)) = best_match {
            // Update the matched speaker's centroid with the online mean of the
            // new embedding. Cosine similarity is scale-invariant so we don't
            // need to renormalize the stored vector.
            let existing = &mut self.speaker_embeddings[idx];
            let n = existing.count as f32;
            for (c, e) in existing.vector.iter_mut().zip(embedding.iter()) {
                *c = (*c * n + *e) / (n + 1.0);
            }
            existing.count += 1;
            tracing::debug!(
                "Speaker match: {} (similarity: {:.3}, n={})",
                existing.speaker_id,
                sim,
                existing.count
            );
            existing.speaker_id.clone()
        } else if self.next_speaker_id < max_speakers {
            // Log best similarity for debugging
            let best_sim = self
                .speaker_embeddings
                .iter()
                .map(|e| new_embedding.cosine_similarity(e))
                .fold(f32::NEG_INFINITY, f32::max);
            if !self.speaker_embeddings.is_empty() {
                tracing::debug!(
                    "New speaker (best similarity: {:.3}, threshold: {:.3})",
                    best_sim,
                    similarity_threshold
                );
            }
            // Create new speaker
            let speaker_id = SpeakerId::Auto(self.next_speaker_id);
            self.speaker_embeddings.push(SpeakerEmbedding {
                vector: embedding.to_vec(),
                speaker_id: speaker_id.clone(),
                count: 1,
            });
            self.next_speaker_id += 1;
            speaker_id
        } else {
            // Too many speakers, return unknown
            SpeakerId::Unknown
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

    /// Convert samples window to milliseconds
    #[cfg(feature = "ml-diarization")]
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
        transcript_segments: &[TranscriptSegment],
    ) -> Vec<DiarizedSegment> {
        // If model is not loaded or feature is disabled, fall back to simple attribution
        #[cfg(not(feature = "ml-diarization"))]
        {
            let _ = samples;
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

            // Segment timestamps are meeting-relative, but samples are chunk-relative.
            // Subtract the chunk's base offset to get correct sample indices.
            let chunk_offset_ms = transcript_segments.first().map(|s| s.start_ms).unwrap_or(0);

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

                // Extract audio window for this segment (adjust to chunk-relative)
                let rel_start_ms = seg.start_ms.saturating_sub(chunk_offset_ms);
                let rel_end_ms = seg.end_ms.saturating_sub(chunk_offset_ms);
                let start_sample = (rel_start_ms as usize * self.sample_rate as usize) / 1000;
                let end_sample = (rel_end_ms as usize * self.sample_rate as usize) / 1000;

                if end_sample > samples.len() {
                    results.push(DiarizedSegment {
                        speaker: SpeakerId::Unknown,
                        start_ms: seg.start_ms,
                        end_ms: seg.end_ms,
                        text: seg.text.clone(),
                        confidence: 0.0,
                    });
                    continue;
                }

                let segment_samples = &samples[start_sample..end_sample.min(samples.len())];

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

                // Now take the lock once and process the cluster updates
                // sequentially (order matters: online centroid updates feed
                // back into subsequent matches within the same segment).
                let mut counts: HashMap<SpeakerId, u32> = HashMap::new();
                match self.state.lock() {
                    Ok(mut state) => {
                        for embedding in &embeddings {
                            let label = state.find_or_create_speaker(
                                embedding,
                                self.similarity_threshold,
                                self.max_speakers,
                            );
                            *counts.entry(label).or_insert(0) += 1;
                        }
                    }
                    Err(e) => tracing::warn!("Speaker state lock poisoned: {}", e),
                }

                // Dominant speaker = mode of sub-window labels.
                // Sort key: (named beats Unknown, higher count wins, then
                // lowest Auto(n) wins on ties — first-seen speaker keeps the
                // segment when two speakers vote-tie within a chunk).
                // Without the third component, HashMap iteration order would
                // make tied results nondeterministic across runs.
                let speaker = counts
                    .into_iter()
                    .min_by_key(|(sp, c)| {
                        let auto_id = match sp {
                            SpeakerId::Auto(n) => *n as i64,
                            _ => i64::MAX,
                        };
                        (matches!(sp, SpeakerId::Unknown), -(*c as i64), auto_id)
                    })
                    .map(|(sp, _)| sp)
                    .unwrap_or(SpeakerId::Unknown);

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

    #[test]
    #[cfg(feature = "ml-diarization")]
    fn test_find_or_create_speaker_new() {
        let mut state = MlDiarizerState::new();
        let embedding = vec![1.0, 0.0, 0.0];
        let speaker = state.find_or_create_speaker(&embedding, 0.75, 10);
        assert_eq!(speaker, SpeakerId::Auto(0));
        assert_eq!(state.next_speaker_id, 1);
        assert_eq!(state.speaker_embeddings.len(), 1);
    }

    #[test]
    #[cfg(feature = "ml-diarization")]
    fn test_find_or_create_speaker_match() {
        let mut state = MlDiarizerState::new();
        // Create first speaker
        let embedding1 = vec![1.0, 0.0, 0.0];
        let speaker1 = state.find_or_create_speaker(&embedding1, 0.75, 10);
        assert_eq!(speaker1, SpeakerId::Auto(0));

        // Same embedding should match
        let speaker2 = state.find_or_create_speaker(&embedding1, 0.75, 10);
        assert_eq!(speaker2, SpeakerId::Auto(0));
        assert_eq!(state.next_speaker_id, 1); // no new speaker created
    }

    #[test]
    #[cfg(feature = "ml-diarization")]
    fn test_find_or_create_speaker_different() {
        let mut state = MlDiarizerState::new();
        // Create first speaker
        let embedding1 = vec![1.0, 0.0, 0.0];
        let speaker1 = state.find_or_create_speaker(&embedding1, 0.75, 10);
        assert_eq!(speaker1, SpeakerId::Auto(0));

        // Orthogonal embedding should create new speaker
        let embedding2 = vec![0.0, 1.0, 0.0];
        let speaker2 = state.find_or_create_speaker(&embedding2, 0.75, 10);
        assert_eq!(speaker2, SpeakerId::Auto(1));
        assert_eq!(state.next_speaker_id, 2);
    }

    #[test]
    #[cfg(feature = "ml-diarization")]
    fn test_find_or_create_speaker_max_speakers() {
        let mut state = MlDiarizerState::new();
        // Fill up to max
        let e1 = vec![1.0, 0.0, 0.0];
        let e2 = vec![0.0, 1.0, 0.0];
        state.find_or_create_speaker(&e1, 0.75, 2);
        state.find_or_create_speaker(&e2, 0.75, 2);

        // Third distinct speaker should return Unknown
        let e3 = vec![0.0, 0.0, 1.0];
        let speaker = state.find_or_create_speaker(&e3, 0.75, 2);
        assert_eq!(speaker, SpeakerId::Unknown);
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
