//! Client for `voxtype-gguf-worker`, which runs transcribe.cpp GGUF models
//! (Cohere, Parakeet) in a separate process.
//!
//! transcribe.cpp and whisper-rs vendor colliding copies of ggml, so voxtype
//! never links transcribe.cpp; see `crates/gguf-worker/Cargo.toml` for the
//! measurements behind that. This client keeps the in-process engines'
//! behaviour otherwise:
//!
//! - The worker starts when the transcriber is constructed, so the daemon still
//!   loads the model at startup unless `on_demand_loading` defers construction,
//!   and a bad model path still fails at that point rather than mid-recording.
//! - The worker is long-lived and keeps the model resident across recordings.
//! - A worker that dies is replaced on the next `prepare()` or `transcribe()`.

use super::Transcriber;
use crate::config::{CohereConfig, Config, GgufConfig, ParakeetConfig};
use crate::error::TranscribeError;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

const READY_SIGNAL: &str = "READY";
const WORKER_BIN: &str = "voxtype-gguf-worker";

#[derive(Debug, PartialEq, Eq)]
enum Health {
    Serving,
    Broken,
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct GgufSubprocessTranscriber {
    /// `--engine` value and config section name, e.g. "parakeet".
    engine: &'static str,
    /// Display name for messages, e.g. "Parakeet".
    name: &'static str,
    /// The engine's config section, model path already resolved.
    config_json: String,
    worker: Mutex<Option<Worker>>,
}

impl GgufSubprocessTranscriber {
    pub fn cohere(config: &CohereConfig) -> Result<Self, TranscribeError> {
        // Validate chunking here so the error reads the same as the ONNX path's.
        crate::transcribe::cohere_chunking::CohereChunking::new(config)?;
        let mut config = config.clone();
        config.model = resolve_model(&config.model)?;
        Self::start("cohere", "Cohere", serialize(&config)?)
    }

    pub fn parakeet(config: &ParakeetConfig) -> Result<Self, TranscribeError> {
        // Streaming runs on parakeet-rs's cache-aware ONNX pipeline. Loading the
        // GGUF in batch mode instead would quietly drop the live partials the
        // user asked for.
        if config.streaming {
            return Err(TranscribeError::ConfigError(
                "[parakeet] streaming = true needs an ONNX streaming model; GGUF models \
                 run in batch mode only. Set streaming = false, or use an ONNX model \
                 such as parakeet-unified-en-0.6b"
                    .to_string(),
            ));
        }
        let mut config = config.clone();
        config.model = resolve_model(&config.model)?;
        Self::start("parakeet", "Parakeet", serialize(&config)?)
    }

    pub fn generic(config: &GgufConfig) -> Result<Self, TranscribeError> {
        crate::model_selection::validate_gguf_choice(
            &config.model,
            &config.architecture,
            &config.language,
            config.streaming,
        )
        .map_err(TranscribeError::ConfigError)?;
        let mut config = config.clone();
        config.model = resolve_model(&config.model)?;
        Self::start("gguf", "GGUF", serialize(&config)?)
    }

    fn start(
        engine: &'static str,
        name: &'static str,
        config_json: String,
    ) -> Result<Self, TranscribeError> {
        let this = Self {
            engine,
            name,
            config_json,
            worker: Mutex::new(None),
        };
        let start = std::time::Instant::now();
        *this.lock()? = Some(this.spawn()?);
        tracing::info!(
            "{} GGUF worker ready in {:.2}s",
            name,
            start.elapsed().as_secs_f32()
        );
        Ok(this)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<Worker>>, TranscribeError> {
        self.worker
            .lock()
            .map_err(|e| TranscribeError::InferenceFailed(format!("GGUF worker lock: {e}")))
    }

    fn spawn(&self) -> Result<Worker, TranscribeError> {
        let path = worker_path();
        let mut child = Command::new(&path)
            .arg("--engine")
            .arg(self.engine)
            .arg("--config-json")
            .arg(&self.config_json)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Cannot start {}: {e}. It ships alongside voxtype; reinstall voxtype \
                     if it is missing, or set VOXTYPE_GGUF_WORKER to its path.",
                    path.display()
                ))
            })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            TranscribeError::InitFailed(format!("GGUF worker {} stdin unavailable", path.display()))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            TranscribeError::InitFailed(format!(
                "GGUF worker {} stdout unavailable",
                path.display()
            ))
        })?;
        let mut stdout = BufReader::new(stdout);

        let mut line = String::new();
        let read = stdout.read_line(&mut line).map_err(|e| {
            TranscribeError::InitFailed(format!(
                "{} GGUF worker {} did not start: {e}",
                self.name,
                path.display()
            ))
        })?;
        if line.trim() == READY_SIGNAL {
            return Ok(Worker {
                child,
                stdin,
                stdout,
            });
        }

        let _ = child.kill();
        let _ = child.wait();
        if read == 0 {
            return Err(TranscribeError::InitFailed(format!(
                "{} GGUF worker {} exited before loading the model; its output is in the log above",
                self.name,
                path.display()
            )));
        }
        // The worker reports a load failure as a protocol line in place of READY.
        Err(protocol_error(&line)
            .map(|error| name_initializing_worker(error, &path))
            .unwrap_or_else(|| {
                TranscribeError::InitFailed(format!(
                    "{} GGUF worker {} sent {line:?} instead of {READY_SIGNAL}",
                    self.name,
                    path.display()
                ))
            }))
    }

    /// One request. The error side carries whether the worker is still usable:
    /// a failure it reported cleanly leaves it serving, while a broken pipe or
    /// an unreadable reply means it has to be replaced.
    fn run(
        &self,
        worker: &mut Worker,
        samples: &[f32],
    ) -> Result<String, (TranscribeError, Health)> {
        let broken = |e: TranscribeError| (e, Health::Broken);
        let pipe = |e: std::io::Error| {
            broken(TranscribeError::InferenceFailed(format!(
                "{} GGUF worker pipe: {e}",
                self.name
            )))
        };
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        worker
            .stdin
            .write_all(&(samples.len() as u32).to_le_bytes())
            .and_then(|()| worker.stdin.write_all(&bytes))
            .and_then(|()| worker.stdin.flush())
            .map_err(pipe)?;

        let mut line = String::new();
        if worker.stdout.read_line(&mut line).map_err(pipe)? == 0 {
            return Err(broken(TranscribeError::InferenceFailed(format!(
                "{} GGUF worker exited mid-transcription; its output is in the log above",
                self.name
            ))));
        }
        let response: serde_json::Value = serde_json::from_str(&line).map_err(|e| {
            broken(TranscribeError::InferenceFailed(format!(
                "{} GGUF worker sent an unreadable response ({e}): {line:?}",
                self.name
            )))
        })?;
        if response["ok"].as_bool() == Some(true) {
            return Ok(response["text"].as_str().unwrap_or_default().to_string());
        }
        Err(match protocol_error(&line) {
            Some(e) => (e, Health::Serving),
            None => broken(TranscribeError::InferenceFailed(format!(
                "{} GGUF worker sent a malformed failure: {line:?}",
                self.name
            ))),
        })
    }
}

impl Transcriber for GgufSubprocessTranscriber {
    fn prepare(&self) {
        let Ok(mut guard) = self.lock() else { return };
        let alive = guard
            .as_mut()
            .is_some_and(|w| matches!(w.child.try_wait(), Ok(None)));
        if alive {
            return;
        }
        match self.spawn() {
            Ok(worker) => *guard = Some(worker),
            Err(e) => tracing::warn!("{} GGUF worker restart failed: {e}", self.name),
        }
    }

    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let mut guard = self.lock()?;
        if guard.is_none() {
            *guard = Some(self.spawn()?);
        }
        match self.run(guard.as_mut().expect("worker present"), samples) {
            Ok(text) => Ok(text),
            Err((e, health)) => {
                if health == Health::Broken {
                    *guard = None;
                }
                Err(e)
            }
        }
    }
}

/// Resolve a bare model name against the models directory, the same way the
/// in-process engines did, and fail here if the file is missing so the error
/// names the path the user configured.
fn resolve_model(model: &str) -> Result<String, TranscribeError> {
    let configured = PathBuf::from(model);
    let path = if configured.exists() || configured.is_absolute() {
        configured
    } else {
        Config::models_dir().join(configured)
    };
    std::fs::metadata(&path)
        .map_err(|e| TranscribeError::ModelNotFound(format!("{}: {e}", path.display())))?;
    Ok(path.to_string_lossy().into_owned())
}

fn serialize<T: serde::Serialize>(config: &T) -> Result<String, TranscribeError> {
    serde_json::to_string(config)
        .map_err(|e| TranscribeError::InitFailed(format!("Cannot serialize GGUF config: {e}")))
}

/// Rebuild the voxtype error a worker reported as `{"ok":false,"kind":..,"error":..}`.
fn protocol_error(line: &str) -> Option<TranscribeError> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value["ok"].as_bool() != Some(false) {
        return None;
    }
    let msg = value["error"]
        .as_str()
        .unwrap_or("unknown error")
        .to_string();
    Some(match value["kind"].as_str() {
        Some("model_not_found") => TranscribeError::ModelNotFound(msg),
        Some("init_failed") => TranscribeError::InitFailed(msg),
        Some("config_error") => TranscribeError::ConfigError(msg),
        _ => TranscribeError::InferenceFailed(msg),
    })
}

fn name_initializing_worker(error: TranscribeError, path: &Path) -> TranscribeError {
    let context = |message| {
        format!(
            "GGUF worker {} failed to initialize: {message}",
            path.display()
        )
    };
    match error {
        TranscribeError::InitFailed(message) => TranscribeError::InitFailed(context(message)),
        TranscribeError::ModelNotFound(message) => TranscribeError::ModelNotFound(context(message)),
        TranscribeError::ConfigError(message) => TranscribeError::ConfigError(context(message)),
        TranscribeError::InferenceFailed(message) => {
            TranscribeError::InferenceFailed(context(message))
        }
        other => other,
    }
}

/// `VOXTYPE_GGUF_WORKER` if set, else the variant worker beside the running
/// executable, then the generic worker beside it, else whatever `PATH` finds.
pub(crate) fn worker_path() -> PathBuf {
    select_worker_path(
        std::env::var_os("VOXTYPE_GGUF_WORKER").as_deref(),
        std::env::current_exe().ok().as_deref(),
    )
}

fn select_worker_path(worker_override: Option<&OsStr>, executable: Option<&Path>) -> PathBuf {
    if let Some(path) = worker_override {
        return PathBuf::from(path);
    }

    executable
        .and_then(worker_beside_executable)
        .unwrap_or_else(|| PathBuf::from(WORKER_BIN))
}

fn worker_beside_executable(executable: &Path) -> Option<PathBuf> {
    // Linux current_exe() already resolves /proc/self/exe, but canonicalizing
    // here keeps the selection rule correct for callers and makes it explicit
    // that a `voxtype` symlink selects the real binary's variant.
    let resolved = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.to_path_buf());
    let directory = resolved.parent()?;
    let executable_name = resolved.file_name()?.to_str()?;

    if executable_name != WORKER_BIN {
        if let Some(variant) = executable_name.strip_prefix("voxtype-") {
            let variant_worker = directory.join(format!("{WORKER_BIN}-{variant}"));
            if variant_worker.exists() {
                return Some(variant_worker);
            }
        }
    }

    let generic_worker = directory.join(WORKER_BIN);
    generic_worker.exists().then_some(generic_worker)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::write(path, []).unwrap();
    }

    #[test]
    fn a_missing_model_fails_before_spawning_and_names_the_path() {
        let config = ParakeetConfig {
            model: "/nonexistent/voxtype/parakeet.gguf".into(),
            ..ParakeetConfig::default()
        };
        let err = GgufSubprocessTranscriber::parakeet(&config).err().unwrap();
        assert!(matches!(err, TranscribeError::ModelNotFound(_)), "{err}");
        assert!(err
            .to_string()
            .contains("/nonexistent/voxtype/parakeet.gguf"));
    }

    #[test]
    fn streaming_with_a_gguf_model_is_a_config_error() {
        let config = ParakeetConfig {
            model: "/nonexistent/voxtype/parakeet.gguf".into(),
            streaming: true,
            ..ParakeetConfig::default()
        };
        let err = GgufSubprocessTranscriber::parakeet(&config).err().unwrap();
        assert!(matches!(err, TranscribeError::ConfigError(_)), "{err}");
    }

    #[test]
    fn invalid_cohere_chunking_is_rejected_before_spawning() {
        let config = CohereConfig {
            model: "/nonexistent/voxtype/cohere.gguf".into(),
            max_chunk_secs: 99,
            ..CohereConfig::default()
        };
        let err = GgufSubprocessTranscriber::cohere(&config).err().unwrap();
        assert!(matches!(err, TranscribeError::ConfigError(_)), "{err}");
    }

    #[test]
    fn worker_errors_come_back_as_the_same_variant() {
        let line = r#"{"ok":false,"kind":"model_not_found","error":"/m.gguf: gone"}"#;
        assert!(matches!(
            protocol_error(line),
            Some(TranscribeError::ModelNotFound(m)) if m == "/m.gguf: gone"
        ));
        assert!(matches!(
            protocol_error(r#"{"ok":false,"kind":"config_error","error":"x"}"#),
            Some(TranscribeError::ConfigError(_))
        ));
        assert!(protocol_error(r#"{"ok":true,"text":"hi"}"#).is_none());
        assert!(protocol_error("READY").is_none());
    }

    #[test]
    fn vulkan_executable_prefers_its_variant_worker() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("voxtype-vulkan");
        let generic_worker = dir.path().join(WORKER_BIN);
        let variant_worker = dir.path().join("voxtype-gguf-worker-vulkan");
        touch(&executable);
        touch(&generic_worker);
        touch(&variant_worker);

        assert_eq!(select_worker_path(None, Some(&executable)), variant_worker);
    }

    #[test]
    fn vulkan_executable_falls_back_to_generic_worker() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("voxtype-vulkan");
        let generic_worker = dir.path().join(WORKER_BIN);
        touch(&executable);
        touch(&generic_worker);

        assert_eq!(select_worker_path(None, Some(&executable)), generic_worker);
    }

    #[test]
    fn plain_executable_uses_generic_worker() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("voxtype");
        let generic_worker = dir.path().join(WORKER_BIN);
        touch(&executable);
        touch(&generic_worker);

        assert_eq!(select_worker_path(None, Some(&executable)), generic_worker);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_executable_uses_resolved_binary_variant() {
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = dir.path().join("bin");
        let link_dir = dir.path().join("links");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&link_dir).unwrap();
        let executable = bin_dir.join("voxtype-vulkan");
        let variant_worker = bin_dir.join("voxtype-gguf-worker-vulkan");
        let symlink = link_dir.join("voxtype");
        touch(&executable);
        touch(&variant_worker);
        std::os::unix::fs::symlink(&executable, &symlink).unwrap();

        assert_eq!(select_worker_path(None, Some(&symlink)), variant_worker);
    }

    #[test]
    fn worker_override_takes_priority() {
        let override_path = Path::new("/opt/voxtype/custom-gguf-worker");
        let executable = Path::new("/usr/bin/voxtype-vulkan");

        assert_eq!(
            select_worker_path(Some(override_path.as_os_str()), Some(executable)),
            override_path
        );
    }

    #[test]
    fn initialization_protocol_errors_keep_their_kind_and_name_the_worker() {
        let path = Path::new("/usr/lib/voxtype/voxtype-gguf-worker-vulkan");
        for (kind, original) in [
            (
                "model_not_found",
                TranscribeError::ModelNotFound("missing model".into()),
            ),
            (
                "config_error",
                TranscribeError::ConfigError("bad backend".into()),
            ),
            (
                "inference_failed",
                TranscribeError::InferenceFailed("native failure".into()),
            ),
        ] {
            let expected_kind = std::mem::discriminant(&original);
            let error = name_initializing_worker(original, path);
            assert_eq!(std::mem::discriminant(&error), expected_kind, "{kind}");
            assert!(
                error.to_string().contains(path.to_str().unwrap()),
                "{kind}: {error}"
            );
        }
    }

    #[test]
    fn initialization_errors_name_the_worker_path() {
        let path = Path::new("/usr/bin/voxtype-gguf-worker-vulkan");
        let error = name_initializing_worker(
            TranscribeError::InitFailed("Vulkan device unavailable".into()),
            path,
        );

        assert!(matches!(error, TranscribeError::InitFailed(_)));
        assert!(error.to_string().contains(path.to_str().unwrap()));
        assert!(error.to_string().contains("Vulkan device unavailable"));
    }
}
