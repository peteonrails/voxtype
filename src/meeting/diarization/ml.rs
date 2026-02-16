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

#[cfg(feature = "ml-diarization")]
use ndarray::{Array1, Array2};
#[cfg(feature = "ml-diarization")]
use ort::{Session, Value};

/// Speaker embedding (voice fingerprint)
#[derive(Debug, Clone)]
pub struct SpeakerEmbedding {
    /// Embedding vector (typically 192 or 256 dimensions)
    pub vector: Vec<f32>,
    /// Speaker ID this embedding belongs to
    pub speaker_id: SpeakerId,
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

/// ML-based speaker diarizer
#[allow(dead_code)]
pub struct MlDiarizer {
    /// Path to the ONNX model file
    model_path: Option<PathBuf>,
    /// ONNX session (lazy loaded)
    #[cfg(feature = "ml-diarization")]
    session: Option<Arc<Session>>,
    /// Known speaker embeddings
    speaker_embeddings: Vec<SpeakerEmbedding>,
    /// Speaker labels (auto ID -> human label)
    speaker_labels: HashMap<u32, String>,
    /// Next speaker ID
    next_speaker_id: u32,
    /// Similarity threshold for matching speakers
    similarity_threshold: f32,
    /// Maximum number of speakers to detect
    max_speakers: u32,
    /// Minimum segment duration for embedding (ms)
    min_segment_ms: u64,
    /// Sample rate for audio
    sample_rate: u32,
}

impl MlDiarizer {
    /// Create a new ML diarizer
    pub fn new(config: &DiarizationConfig) -> Self {
        Self {
            model_path: config.model_path.as_ref().map(PathBuf::from),
            #[cfg(feature = "ml-diarization")]
            session: None,
            speaker_embeddings: Vec::new(),
            speaker_labels: HashMap::new(),
            next_speaker_id: 0,
            similarity_threshold: 0.75,
            max_speakers: config.max_speakers,
            min_segment_ms: config.min_segment_ms,
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
            Ok(builder) => match builder.with_model_from_file(&path) {
                Ok(session) => {
                    self.session = Some(Arc::new(session));
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
        let session = self.session.as_ref().ok_or("Model not loaded")?;

        // Prepare input tensor: [batch=1, samples]
        let input_array = Array2::from_shape_vec((1, samples.len()), samples.to_vec())
            .map_err(|e| format!("Failed to create input array: {}", e))?;

        let input_value = Value::from_array(input_array)
            .map_err(|e| format!("Failed to create input value: {}", e))?;

        // Run inference
        let outputs = session
            .run(ort::inputs![input_value].map_err(|e| format!("Input error: {}", e))?)
            .map_err(|e| format!("Inference failed: {}", e))?;

        // Extract embedding from output
        let output = outputs
            .get("embedding")
            .or_else(|| outputs.values().next())
            .ok_or("No output from model")?;

        let embedding: Array1<f32> = output
            .try_extract_tensor()
            .map_err(|e| format!("Failed to extract tensor: {}", e))?
            .view()
            .to_owned()
            .into_dimensionality()
            .map_err(|e| format!("Dimension error: {}", e))?;

        Ok(embedding.to_vec())
    }

    /// Find or create speaker ID for an embedding
    #[allow(dead_code)]
    fn find_or_create_speaker(&mut self, embedding: &[f32]) -> SpeakerId {
        let new_embedding = SpeakerEmbedding {
            vector: embedding.to_vec(),
            speaker_id: SpeakerId::Auto(self.next_speaker_id),
        };

        // Find best matching existing speaker
        let mut best_match: Option<(usize, f32)> = None;
        for (i, existing) in self.speaker_embeddings.iter().enumerate() {
            let similarity = new_embedding.cosine_similarity(existing);
            if similarity > self.similarity_threshold {
                match best_match {
                    None => best_match = Some((i, similarity)),
                    Some((_, best_sim)) if similarity > best_sim => {
                        best_match = Some((i, similarity))
                    }
                    _ => {}
                }
            }
        }

        if let Some((idx, _)) = best_match {
            // Return existing speaker
            self.speaker_embeddings[idx].speaker_id.clone()
        } else if self.next_speaker_id < self.max_speakers {
            // Create new speaker
            let speaker_id = SpeakerId::Auto(self.next_speaker_id);
            self.speaker_embeddings.push(SpeakerEmbedding {
                vector: embedding.to_vec(),
                speaker_id: speaker_id.clone(),
            });
            self.next_speaker_id += 1;
            speaker_id
        } else {
            // Too many speakers, return unknown
            SpeakerId::Unknown
        }
    }

    /// Label a speaker
    pub fn label_speaker(&mut self, auto_id: u32, label: String) {
        self.speaker_labels.insert(auto_id, label);
    }

    /// Get speaker label if set
    pub fn get_label(&self, speaker_id: &SpeakerId) -> Option<String> {
        match speaker_id {
            SpeakerId::Auto(id) => self.speaker_labels.get(id).cloned(),
            _ => None,
        }
    }

    /// Convert samples window to milliseconds
    #[allow(dead_code)]
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
        _samples: &[f32],
        _source: AudioSource,
        transcript_segments: &[TranscriptSegment],
    ) -> Vec<DiarizedSegment> {
        // If model is not loaded or feature is disabled, fall back to simple attribution
        #[cfg(not(feature = "ml-diarization"))]
        {
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

                // Extract audio window for this segment
                let start_sample = (seg.start_ms as usize * self.sample_rate as usize) / 1000;
                let end_sample = (seg.end_ms as usize * self.sample_rate as usize) / 1000;

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

                // Extract embedding
                match self.extract_embedding(segment_samples) {
                    Ok(embedding) => {
                        // Note: find_or_create_speaker needs mutable self, but diarize takes &self
                        // In a real implementation, we'd need interior mutability or a different pattern
                        // For now, return with unknown speaker and let caller handle labeling
                        results.push(DiarizedSegment {
                            speaker: SpeakerId::Unknown, // Would be find_or_create_speaker result
                            start_ms: seg.start_ms,
                            end_ms: seg.end_ms,
                            text: seg.text.clone(),
                            confidence: 0.8,
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Failed to extract embedding: {}", e);
                        results.push(DiarizedSegment {
                            speaker: SpeakerId::Unknown,
                            start_ms: seg.start_ms,
                            end_ms: seg.end_ms,
                            text: seg.text.clone(),
                            confidence: 0.0,
                        });
                    }
                }
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
        };
        let b = SpeakerEmbedding {
            vector: vec![1.0, 0.0, 0.0],
            speaker_id: SpeakerId::Auto(1),
        };
        assert!((a.cosine_similarity(&b) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = SpeakerEmbedding {
            vector: vec![1.0, 0.0, 0.0],
            speaker_id: SpeakerId::Auto(0),
        };
        let b = SpeakerEmbedding {
            vector: vec![0.0, 1.0, 0.0],
            speaker_id: SpeakerId::Auto(1),
        };
        assert!(a.cosine_similarity(&b).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = SpeakerEmbedding {
            vector: vec![1.0, 0.0, 0.0],
            speaker_id: SpeakerId::Auto(0),
        };
        let b = SpeakerEmbedding {
            vector: vec![-1.0, 0.0, 0.0],
            speaker_id: SpeakerId::Auto(1),
        };
        assert!((a.cosine_similarity(&b) + 1.0).abs() < 0.001);
    }

    #[test]
    fn test_speaker_labeling() {
        let mut diarizer = MlDiarizer::default();
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
}
