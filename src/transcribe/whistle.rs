//! Whistle speech-to-text through Cactus Needle's native C API.
//!
//! The upstream runtime keeps process-global model state and is not thread-safe.
//! All API calls are therefore serialized, including calls made by separate
//! transcriber instances.

use crate::config::{Config, WhistleConfig};
use crate::error::TranscribeError;
use crate::transcribe::{TimedSegment, Transcriber};
use libloading::Library;
use serde::Deserialize;
use std::ffi::{c_char, c_int, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const OUTPUT_CAPACITY: usize = 1024 * 1024;

struct NativeRuntime {
    library: Library,
}

struct LoadedModel {
    runtime: Arc<NativeRuntime>,
    path: PathBuf,
}

static NATIVE_STATE: OnceLock<Mutex<Option<LoadedModel>>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct WhistleResult {
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    words: Vec<WhistleWord>,
}

#[derive(Debug, Deserialize)]
struct WhistleWord {
    word: String,
    start: f32,
    end: f32,
}

/// Cactus Whistle transcription backend.
pub struct WhistleTranscriber {
    model_path: PathBuf,
    language: Option<String>,
    keywords: String,
    last_language: Mutex<Option<String>>,
}

impl WhistleTranscriber {
    pub fn new(config: &WhistleConfig) -> Result<Self, TranscribeError> {
        config.validate_language().map_err(|e| {
            TranscribeError::InitFailed(format!("Invalid [whistle] configuration: {e}"))
        })?;
        let model_path = resolve_model_path(&config.model);
        if !model_path.is_file() {
            return Err(TranscribeError::ModelNotFound(format!(
                "Whistle model not found: {}\n  Run `voxtype setup --download --model whistle.cact` to install it.",
                model_path.display()
            )));
        }
        let runtime_path = std::env::var_os("VOXTYPE_WHISTLE_RUNTIME")
            .map(PathBuf::from)
            .or_else(|| config.runtime.clone());
        ensure_runtime_loaded(&model_path, runtime_path.as_deref())?;

        Ok(Self {
            model_path,
            language: config.language.clone(),
            keywords: config.keywords.join("\n"),
            last_language: Mutex::new(None),
        })
    }

    fn run(
        &self,
        samples: &[f32],
        word_timestamps: bool,
    ) -> Result<WhistleResult, TranscribeError> {
        if samples.len() > 30 * 16_000 {
            return Err(TranscribeError::InferenceFailed(
                "Whistle accepts audio clips up to 30 seconds".to_string(),
            ));
        }
        let language = self
            .language
            .as_deref()
            .map(CString::new)
            .transpose()
            .map_err(|e| TranscribeError::InferenceFailed(e.to_string()))?;
        let keywords = if self.keywords.is_empty() {
            None
        } else {
            Some(CString::new(self.keywords.as_str()).map_err(|e| {
                TranscribeError::InferenceFailed(format!("Invalid Whistle keywords: {e}"))
            })?)
        };

        let state = native_state();
        let loaded = state.lock().map_err(|_| {
            TranscribeError::InferenceFailed("Whistle native runtime lock was poisoned".into())
        })?;
        let loaded = loaded.as_ref().ok_or_else(|| {
            TranscribeError::InitFailed("Whistle native runtime is not initialized".into())
        })?;
        if loaded.path != self.model_path {
            return Err(TranscribeError::InitFailed(format!(
                "Whistle supports one model per process; '{}' is already loaded, but '{}' was requested",
                loaded.path.display(),
                self.model_path.display()
            )));
        }
        let runtime = &loaded.runtime;
        let mut output = vec![0u8; OUTPUT_CAPACITY];

        // Safety: function signatures match needle.h. The state mutex serializes
        // this process-global, non-thread-safe C API; input and output buffers
        // remain alive for the duration of the call.
        let result_code = unsafe {
            let transcribe = runtime
                .library
                .get::<unsafe extern "C" fn(
                    *const f32,
                    c_int,
                    *const c_char,
                    *const c_char,
                    c_int,
                    *mut c_char,
                    c_int,
                ) -> c_int>(b"needle_transcribe\0")
                .map_err(|e| runtime_error("needle_transcribe", e))?;
            transcribe(
                samples.as_ptr(),
                c_int::try_from(samples.len()).map_err(|_| {
                    TranscribeError::InferenceFailed("Whistle audio is too long".into())
                })?,
                language.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
                keywords.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
                i32::from(word_timestamps),
                output.as_mut_ptr().cast(),
                c_int::try_from(output.len()).unwrap_or(c_int::MAX),
            )
        };
        if result_code < 0 {
            return Err(native_error(runtime, "Whistle transcription failed"));
        }
        let result: WhistleResult = serde_json::from_slice(
            &output[..output.iter().position(|b| *b == 0).unwrap_or(output.len())],
        )
        .map_err(|e| {
            TranscribeError::InferenceFailed(format!("Could not parse Whistle result: {e}"))
        })?;
        if let Ok(mut last) = self.last_language.lock() {
            *last = result.language.clone();
        }
        Ok(result)
    }
}

impl Transcriber for WhistleTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        Ok(self.run(samples, false)?.text.trim().to_string())
    }

    fn transcribe_timed(&self, samples: &[f32]) -> Result<Vec<TimedSegment>, TranscribeError> {
        let result = self.run(samples, true)?;
        if result.words.is_empty() {
            let text = result.text.trim().to_string();
            return if text.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![TimedSegment {
                    text,
                    start_secs: 0.0,
                    end_secs: samples.len() as f32 / 16_000.0,
                }])
            };
        }
        Ok(result
            .words
            .into_iter()
            .map(|word| TimedSegment {
                text: word.word,
                start_secs: word.start,
                end_secs: word.end,
            })
            .collect())
    }

    fn last_detected_language(&self) -> Option<String> {
        self.last_language.lock().ok().and_then(|lang| lang.clone())
    }
}

fn native_state() -> &'static Mutex<Option<LoadedModel>> {
    NATIVE_STATE.get_or_init(|| Mutex::new(None))
}

fn resolve_model_path(model: &str) -> PathBuf {
    let path = Path::new(model);
    if path.is_absolute() || path.exists() {
        path.to_path_buf()
    } else if model == "whistle.cact" {
        Config::models_dir().join("whistle").join(model)
    } else {
        Config::models_dir().join(model)
    }
}

fn ensure_runtime_loaded(
    model_path: &Path,
    runtime_path: Option<&Path>,
) -> Result<(), TranscribeError> {
    // The upstream runtime's telemetry is opt-out. Voxtype is private/offline
    // by default, so disable it before loading any native code.
    std::env::set_var("NEEDLE_TELEMETRY", "0");
    std::env::set_var("DO_NOT_TRACK", "1");

    let load_started = std::time::Instant::now();
    let mut state = native_state().lock().map_err(|_| {
        TranscribeError::InitFailed("Whistle native runtime lock was poisoned".into())
    })?;
    if let Some(loaded) = state.as_ref() {
        if loaded.path == model_path {
            return Ok(());
        }
        return Err(TranscribeError::InitFailed(format!(
            "Whistle supports one model per process; '{}' is already loaded",
            loaded.path.display()
        )));
    }

    let library = unsafe {
        match runtime_path {
            Some(path) => Library::new(path),
            None => load_default_library(),
        }
    }
    .map_err(|e| {
        TranscribeError::InitFailed(format!(
            "Could not load the Cactus Needle native runtime: {e}. Install libneedle and either make it available to the system loader or set [whistle].runtime to its path."
        ))
    })?;
    let runtime = Arc::new(NativeRuntime { library });
    let bytes = std::fs::read(model_path).map_err(|e| {
        TranscribeError::ModelNotFound(format!(
            "Could not read Whistle model {}: {e}",
            model_path.display()
        ))
    })?;
    let code = unsafe {
        let load = runtime
            .library
            .get::<unsafe extern "C" fn(*const u8, u64) -> c_int>(b"needle_load\0")
            .map_err(|e| runtime_error("needle_load", e))?;
        load(bytes.as_ptr(), bytes.len() as u64)
    };
    if code < 0 {
        return Err(native_error(&runtime, "Could not load Whistle model"));
    }
    tracing::info!(
        model = %model_path.display(),
        load_ms = load_started.elapsed().as_millis(),
        "Whistle model loaded"
    );
    *state = Some(LoadedModel {
        runtime,
        path: model_path.to_path_buf(),
    });
    Ok(())
}

unsafe fn load_default_library() -> Result<Library, libloading::Error> {
    #[cfg(target_os = "windows")]
    const LIBRARY: &str = "libneedle.dll";
    #[cfg(target_os = "macos")]
    const LIBRARY: &str = "libneedle.dylib";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    const LIBRARY: &str = "libneedle.so";
    Library::new(LIBRARY).or_else(|_| {
        #[cfg(target_os = "windows")]
        const ALTERNATE: &str = "libneedle3.dll";
        #[cfg(target_os = "macos")]
        const ALTERNATE: &str = "libneedle3.dylib";
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        const ALTERNATE: &str = "libneedle3.so";
        Library::new(ALTERNATE)
    })
}

fn runtime_error(function: &str, error: libloading::Error) -> TranscribeError {
    TranscribeError::InitFailed(format!(
        "Cactus Needle runtime does not export {function}: {error}"
    ))
}

fn native_error(runtime: &NativeRuntime, fallback: &str) -> TranscribeError {
    let message = unsafe {
        runtime
            .library
            .get::<unsafe extern "C" fn() -> *const c_char>(b"needle_last_error\0")
            .ok()
            .map(|last_error| {
                let ptr = last_error();
                if ptr.is_null() {
                    return fallback.to_string();
                }
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            })
            .unwrap_or_else(|| fallback.to_string())
    };
    TranscribeError::InferenceFailed(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_model_names_under_voxtype_models_dir() {
        assert_eq!(
            resolve_model_path("whistle.cact"),
            Config::models_dir().join("whistle").join("whistle.cact")
        );
    }

    #[test]
    fn reports_missing_model_before_loading_runtime() {
        let config = WhistleConfig {
            model: "/tmp/voxtype-missing-whistle.cact".to_string(),
            ..WhistleConfig::default()
        };
        let error = match WhistleTranscriber::new(&config) {
            Ok(_) => panic!("missing model unexpectedly accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("Whistle model not found"));
    }

    #[test]
    fn reports_missing_native_runtime_clearly() {
        let directory = tempfile::tempdir().unwrap();
        let model = directory.path().join("whistle.cact");
        std::fs::write(&model, b"model placeholder").unwrap();
        let config = WhistleConfig {
            model: model.display().to_string(),
            runtime: Some(directory.path().join("missing-libneedle.so")),
            ..WhistleConfig::default()
        };
        let error = match WhistleTranscriber::new(&config) {
            Ok(_) => panic!("missing runtime unexpectedly accepted"),
            Err(error) => error,
        };
        assert!(error
            .to_string()
            .contains("Could not load the Cactus Needle native runtime"));
    }

    #[test]
    fn keeps_explicit_model_paths() {
        let path = PathBuf::from("/tmp/custom-whistle.cact");
        assert_eq!(resolve_model_path(path.to_str().unwrap()), path);
    }

    #[test]
    fn parses_native_transcription_and_words() {
        let result: WhistleResult = serde_json::from_str(
            r#"{"text":"bonjour","language":"fr","words":[{"word":"bonjour","start":0.1,"end":0.8,"probability":0.9}]}"#,
        )
        .unwrap();
        assert_eq!(result.text, "bonjour");
        assert_eq!(result.language.as_deref(), Some("fr"));
        assert_eq!(result.words.len(), 1);
        assert_eq!(result.words[0].start, 0.1);
    }
}
