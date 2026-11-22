//! Controlled trait-contract tests, NOT actual ASR/model inference.
//! No provider, audio device, model weights, output driver or filesystem use.
use std::sync::{Arc, Mutex};
use voxtype::error::TranscribeError;
use voxtype::transcribe::{
    RuntimeMetadata, SlidingWindowConfig, SlidingWindowStreamingTranscriber, Transcriber,
};

#[derive(Default)]
struct MetadataOnly {
    reported: Mutex<RuntimeMetadata>,
    language: Mutex<Option<String>>,
}

impl Transcriber for MetadataOnly {
    fn transcribe(&self, _: &[f32]) -> Result<String, TranscribeError> {
        panic!("Metadata-only contract must never be labeled or run as inference")
    }

    fn runtime_metadata(&self) -> RuntimeMetadata {
        self.reported.lock().unwrap().clone()
    }

    fn last_detected_language(&self) -> Option<String> {
        self.language.lock().unwrap().clone()
    }
}

#[test]
fn streaming_wrapper_retains_current_underlying_runtime_measurements_without_guessing() {
    let base = Arc::new(MetadataOnly::default());
    let wrapper =
        SlidingWindowStreamingTranscriber::new(base.clone(), SlidingWindowConfig::default());
    assert_eq!(wrapper.runtime_metadata(), RuntimeMetadata::default());
    for measured in [
        RuntimeMetadata {
            model: Some("controlled-protocol-fixture.bin".into()),
            backend: Some("controlled-protocol-backend".into()),
            model_load_ms: Some(17),
        },
        RuntimeMetadata::default(),
        RuntimeMetadata {
            model: Some("another-controlled-protocol-fixture.bin".into()),
            backend: None,
            model_load_ms: Some(23),
        },
    ] {
        *base.reported.lock().unwrap() = measured.clone();
        assert_eq!(wrapper.runtime_metadata(), measured, "The factory's streaming adapter must not erase a measured model/backend/load time or retain stale measurements");
    }
}

#[test]
fn streaming_wrapper_retains_current_detected_language_and_unknown_state() {
    let base = Arc::new(MetadataOnly::default());
    let wrapper =
        SlidingWindowStreamingTranscriber::new(base.clone(), SlidingWindowConfig::default());
    for language in [Some("de".to_owned()), None, Some("en".to_owned())] {
        *base.language.lock().unwrap() = language.clone();
        assert_eq!(wrapper.last_detected_language(), language, "Streaming must preserve the underlying detected language, not silently revert to a configured hint");
    }
}
