//! The parts of voxtype's `[cohere]` and `[parakeet]` sections this worker reads.
//!
//! voxtype serializes its full config structs and serde ignores the fields not
//! named here, so voxtype can grow its sections without touching the worker.
//! Defaults mirror voxtype's, so a missing key means the same thing on both
//! sides. The contract tests below round-trip voxtype's own types to keep the
//! two in step.
//!
//! The worker deliberately does not depend on the voxtype crate: voxtype links
//! whisper-rs unconditionally, and that would put whisper.cpp's ggml back on
//! this binary's link line.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct CohereConfig {
    pub model: String,
    #[serde(default = "default_gguf_backend")]
    pub gguf_backend: String,
    #[serde(default = "default_max_chunk_secs")]
    pub max_chunk_secs: u32,
    #[serde(default = "default_boundary_search_secs")]
    pub boundary_search_secs: f32,
    #[serde(default = "default_cohere_language")]
    pub language: String,
    #[serde(default)]
    pub threads: Option<usize>,
}

impl Default for CohereConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            gguf_backend: default_gguf_backend(),
            max_chunk_secs: default_max_chunk_secs(),
            boundary_search_secs: default_boundary_search_secs(),
            language: default_cohere_language(),
            threads: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ParakeetConfig {
    pub model: String,
    #[serde(default = "default_gguf_backend")]
    pub gguf_backend: String,
    #[serde(default)]
    pub threads: Option<usize>,
    #[serde(default = "default_parakeet_max_window_secs")]
    pub max_window_secs: f32,
    #[serde(default = "default_parakeet_window_overlap_secs")]
    pub window_overlap_secs: f32,
    #[serde(default)]
    pub streaming: bool,
}

impl Default for ParakeetConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            gguf_backend: default_gguf_backend(),
            threads: None,
            max_window_secs: default_parakeet_max_window_secs(),
            window_overlap_secs: default_parakeet_window_overlap_secs(),
            streaming: false,
        }
    }
}

fn default_gguf_backend() -> String {
    "auto".to_string()
}

fn default_max_chunk_secs() -> u32 {
    35
}

fn default_boundary_search_secs() -> f32 {
    2.5
}

fn default_cohere_language() -> String {
    "en".to_string()
}

fn default_parakeet_max_window_secs() -> f32 {
    240.0
}

fn default_parakeet_window_overlap_secs() -> f32 {
    5.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cohere_round_trips_from_voxtypes_own_type() {
        let sent = voxtype::config::CohereConfig {
            model: "/models/cohere.gguf".into(),
            gguf_backend: "vulkan".into(),
            max_chunk_secs: 20,
            boundary_search_secs: 1.5,
            language: "fr".into(),
            threads: Some(6),
            ..voxtype::config::CohereConfig::default()
        };
        let got: CohereConfig =
            serde_json::from_str(&serde_json::to_string(&sent).unwrap()).unwrap();
        assert_eq!(got.model, sent.model);
        assert_eq!(got.gguf_backend, sent.gguf_backend);
        assert_eq!(got.max_chunk_secs, sent.max_chunk_secs);
        assert_eq!(got.boundary_search_secs, sent.boundary_search_secs);
        assert_eq!(got.language, sent.language);
        assert_eq!(got.threads, sent.threads);
    }

    #[test]
    fn parakeet_round_trips_from_voxtypes_own_type() {
        let sent = voxtype::config::ParakeetConfig {
            model: "/models/parakeet.gguf".into(),
            gguf_backend: "cpu".into(),
            threads: Some(4),
            max_window_secs: 120.0,
            window_overlap_secs: 3.0,
            streaming: false,
            ..voxtype::config::ParakeetConfig::default()
        };
        let got: ParakeetConfig =
            serde_json::from_str(&serde_json::to_string(&sent).unwrap()).unwrap();
        assert_eq!(got.model, sent.model);
        assert_eq!(got.gguf_backend, sent.gguf_backend);
        assert_eq!(got.threads, sent.threads);
        assert_eq!(got.max_window_secs, sent.max_window_secs);
        assert_eq!(got.window_overlap_secs, sent.window_overlap_secs);
        assert_eq!(got.streaming, sent.streaming);
    }

    #[test]
    fn defaults_match_voxtype() {
        let ours = CohereConfig::default();
        let theirs = voxtype::config::CohereConfig::default();
        assert_eq!(ours.gguf_backend, theirs.gguf_backend);
        assert_eq!(ours.max_chunk_secs, theirs.max_chunk_secs);
        assert_eq!(ours.boundary_search_secs, theirs.boundary_search_secs);
        assert_eq!(ours.language, theirs.language);

        let ours = ParakeetConfig::default();
        let theirs = voxtype::config::ParakeetConfig::default();
        assert_eq!(ours.gguf_backend, theirs.gguf_backend);
        assert_eq!(ours.max_window_secs, theirs.max_window_secs);
        assert_eq!(ours.window_overlap_secs, theirs.window_overlap_secs);
    }
}
