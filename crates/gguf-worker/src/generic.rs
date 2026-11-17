//! Offline inference for curated transcribe.cpp families that do not have a
//! legacy Voxtype backend. Architecture and language intent arrive explicitly
//! from the validated parent config; the worker verifies the GGUF metadata
//! before accepting audio.

use crate::config::GenericGgufConfig;
use crate::error::TranscribeError;
use crate::gguf::{self, GgufEngine, GgufLoad, GgufSession};
use crate::Transcriber;
use transcribe_cpp::RunOptions;

const ENGINE: GgufEngine = GgufEngine {
    name: "GGUF",
    section: "gguf",
};

pub struct GenericGgufTranscriber {
    session: GgufSession,
    language: Option<String>,
}

impl GenericGgufTranscriber {
    pub fn new(config: &GenericGgufConfig) -> Result<Self, TranscribeError> {
        if config.streaming {
            return Err(TranscribeError::ConfigError(
                "[gguf] streaming = true is not implemented; these checkpoints run offline only. Set streaming = false or keep the existing engine. Config unchanged."
                    .to_string(),
            ));
        }
        let session = GgufSession::load(
            &ENGINE,
            &GgufLoad {
                model: &config.model,
                backend: &config.gguf_backend,
                threads: config.threads,
            },
        )?;
        let actual = session.arch()?;
        if actual != config.architecture {
            return Err(TranscribeError::InitFailed(format!(
                "{} is a '{actual}' GGUF, not the configured '{}' architecture",
                config.model, config.architecture
            )));
        }
        let language = match config.language.as_str() {
            "" | "auto" => None,
            value => Some(value.to_string()),
        };
        Ok(Self { session, language })
    }
}

impl Transcriber for GenericGgufTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let mut session = self.session.lock()?;
        let result = gguf::run(
            &mut session,
            samples,
            &RunOptions {
                language: self.language.clone(),
                ..Default::default()
            },
        )?;
        Ok(result.text.trim().to_string())
    }
}
