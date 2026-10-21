//! transcribe.cpp plumbing shared by the GGUF engines.
//!
//! Every GGUF family loads the same way: resolve the configured path against
//! the models directory, check the header before C++ sees the file, request a
//! backend, refuse a software Vulkan device, and keep one session resident
//! across recordings. Only the run options and the long-audio strategy differ
//! between families, so those stay in each engine's module.

use crate::config::Config;
use crate::error::TranscribeError;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use transcribe_cpp::{
    Backend, Model, ModelOptions, RunOptions, Session, SessionOptions, Transcript,
};

/// The engine a GGUF session is loaded for, as it appears to the user.
pub(crate) struct GgufEngine {
    /// Display name for log lines and errors, e.g. "Cohere".
    pub name: &'static str,
    /// Config section, so errors name the key to fix, e.g. "cohere".
    pub section: &'static str,
}

/// What to load: the `[section]` settings every GGUF engine shares.
pub(crate) struct GgufLoad<'a> {
    pub model: &'a str,
    pub backend: &'a str,
    pub threads: Option<usize>,
}

/// One resident transcribe.cpp session. The model stays loaded for as long as
/// this lives, which is what lets dictation skip the load on every recording.
pub(crate) struct GgufSession {
    session: Mutex<Session>,
}

impl GgufSession {
    pub fn load(engine: &GgufEngine, load: &GgufLoad) -> Result<Self, TranscribeError> {
        let configured = PathBuf::from(load.model);
        let model_path = if configured.exists() || configured.is_absolute() {
            configured
        } else {
            Config::models_dir().join(configured)
        };
        let mut file = std::fs::File::open(&model_path).map_err(|e| {
            TranscribeError::ModelNotFound(format!("{}: {e}", model_path.display()))
        })?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic).map_err(|e| {
            TranscribeError::InitFailed(format!(
                "Cannot read GGUF header at {}: {e}",
                model_path.display()
            ))
        })?;
        if &magic != b"GGUF" {
            return Err(TranscribeError::InitFailed(format!(
                "{} is not a GGUF model",
                model_path.display()
            )));
        }

        let backend = parse_backend(load.backend, engine.section)?;
        let model = Model::load_with(
            &model_path,
            &ModelOptions {
                backend,
                ..Default::default()
            },
        )
        .map_err(|e| TranscribeError::InitFailed(format!("GGUF model load: {e}")))?;
        let device = model
            .device()
            .map_err(|e| TranscribeError::InitFailed(format!("GGUF device query: {e}")))?;
        if backend == Backend::Vulkan {
            let name = format!("{} {}", device.name, device.description).to_ascii_lowercase();
            if device.kind != "vulkan"
                || ["llvmpipe", "lavapipe", "swiftshader", "software"]
                    .iter()
                    .any(|renderer| name.contains(renderer))
            {
                return Err(TranscribeError::InitFailed(format!(
                    "{} requested hardware Vulkan but transcribe.cpp selected {} [{}] ({})",
                    engine.name, device.name, device.kind, device.description
                )));
            }
        }
        tracing::info!(
            "{} GGUF loaded on {} [{}] ({})",
            engine.name,
            device.name,
            model.backend(),
            device.description
        );
        let session = model
            .session_with(&SessionOptions {
                n_threads: load.threads.unwrap_or(0).try_into().map_err(|_| {
                    TranscribeError::ConfigError(format!(
                        "{}.threads exceeds i32::MAX",
                        engine.section
                    ))
                })?,
                ..Default::default()
            })
            .map_err(|e| TranscribeError::InitFailed(format!("GGUF session init: {e}")))?;
        Ok(Self {
            session: Mutex::new(session),
        })
    }

    /// The session, held for a whole transcription so chunked runs cannot
    /// interleave with another caller's.
    pub fn lock(&self) -> Result<MutexGuard<'_, Session>, TranscribeError> {
        self.session
            .lock()
            .map_err(|e| TranscribeError::InferenceFailed(format!("GGUF session lock: {e}")))
    }

    /// The model's `general.architecture`, e.g. "parakeet".
    #[cfg_attr(not(feature = "parakeet-gguf"), allow(dead_code))]
    pub fn arch(&self) -> Result<String, TranscribeError> {
        Ok(self.lock()?.model().arch())
    }
}

/// One forward pass, with the library's error mapped into voxtype's.
pub(crate) fn run(
    session: &mut Session,
    samples: &[f32],
    options: &RunOptions,
) -> Result<Transcript, TranscribeError> {
    session
        .run(samples, options)
        .map_err(|e| TranscribeError::InferenceFailed(format!("GGUF inference: {e}")))
}

/// Parse a `<section>.gguf_backend` value, case-insensitively.
pub(crate) fn parse_backend(value: &str, section: &str) -> Result<Backend, TranscribeError> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(Backend::Auto),
        "cpu" => Ok(Backend::Cpu),
        "cpu_accel" => Ok(Backend::CpuAccel),
        "vulkan" => Ok(Backend::Vulkan),
        "metal" => Ok(Backend::Metal),
        "cuda" => Ok(Backend::Cuda),
        "rocm" => Ok(Backend::Rocm),
        _ => Err(TranscribeError::ConfigError(format!(
            "Invalid {section}.gguf_backend: {value}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_errors_name_the_engine_section() {
        assert_eq!(
            parse_backend("Cpu_Accel", "parakeet").unwrap(),
            Backend::CpuAccel
        );
        let err = parse_backend("opencl", "parakeet").unwrap_err().to_string();
        assert!(err.contains("parakeet.gguf_backend"), "{err}");
    }

    #[test]
    fn missing_model_is_reported_as_not_found() {
        let engine = GgufEngine {
            name: "Test",
            section: "test",
        };
        let load = GgufLoad {
            model: "/nonexistent/voxtype/model.gguf",
            backend: "auto",
            threads: None,
        };
        assert!(matches!(
            GgufSession::load(&engine, &load),
            Err(TranscribeError::ModelNotFound(_))
        ));
    }
}
