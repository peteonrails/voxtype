//! The worker's error type. Variant names match voxtype's `TranscribeError` so
//! the engine code reads the same in both places, and `kind()` travels over the
//! pipe so voxtype can rebuild the same variant on its side. The Display text is
//! the bare detail: voxtype adds its own prefix when it reports the error.

#[derive(Debug)]
pub enum TranscribeError {
    ModelNotFound(String),
    InitFailed(String),
    InferenceFailed(String),
    ConfigError(String),
}

impl TranscribeError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ModelNotFound(_) => "model_not_found",
            Self::InitFailed(_) => "init_failed",
            Self::InferenceFailed(_) => "inference_failed",
            Self::ConfigError(_) => "config_error",
        }
    }
}

impl std::fmt::Display for TranscribeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelNotFound(msg)
            | Self::InitFailed(msg)
            | Self::InferenceFailed(msg)
            | Self::ConfigError(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for TranscribeError {}
