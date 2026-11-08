//! Model-format capabilities shared by selection, pickers and runtime dispatch.
//! Loading a config never migrates it. Only an intentional engine selection may
//! substitute an installed compatible checkpoint for an unsupported retained one.
use crate::config::{Config, TranscriptionEngine};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub struct BackendCapabilities {
    pub parakeet_onnx: bool,
    pub parakeet_gguf: bool,
    pub cohere_onnx: bool,
    pub cohere_gguf: bool,
}

impl BackendCapabilities {
    pub const fn compiled() -> Self {
        Self {
            parakeet_onnx: cfg!(feature = "parakeet"),
            parakeet_gguf: cfg!(feature = "parakeet-gguf"),
            cohere_onnx: cfg!(feature = "cohere"),
            cohere_gguf: cfg!(feature = "cohere-gguf"),
        }
    }

    pub fn supports(self, engine: &str, model: &str) -> bool {
        match (engine, is_gguf(model)) {
            ("parakeet", true) => self.parakeet_gguf,
            ("parakeet", false) => self.parakeet_onnx,
            ("cohere", true) => self.cohere_gguf,
            ("cohere", false) => self.cohere_onnx,
            _ => crate::config_set::engine_feature_compiled(engine),
        }
    }
}

pub fn is_gguf(model: &str) -> bool {
    model.to_ascii_lowercase().ends_with(".gguf")
}

pub fn validate_backend(engine: &str, model: &str) -> Result<(), String> {
    if let Some(checkpoint) = Path::new(model)
        .file_name()
        .and_then(|p| p.to_str())
        .and_then(crate::setup::model::find_gguf_model)
    {
        if checkpoint.engine != engine {
            return Err(format!("Model '{model}' belongs to {}, not {engine}. Select an engine-compatible checkpoint; config unchanged.", checkpoint.engine));
        }
    }
    if BackendCapabilities::compiled().supports(engine, model) {
        return Ok(());
    }
    let format = if is_gguf(model) { "GGUF" } else { "ONNX" };
    let feature = if is_gguf(model) {
        format!("{engine}-gguf")
    } else {
        engine.to_string()
    };
    let default = crate::model_catalog::default_model(engine);
    Err(format!(
        "{engine} model '{model}' selects the {format} backend, which is not compiled into this binary (requires --features {feature}). GGUF and ONNX require different backend features. Choose a compatible model with `voxtype config set {engine}.model {default}` (download first with `voxtype setup --download --model {default}`), or install a build supporting this explicit {format} model. Config unchanged."
    ))
}

pub fn model_path_in(models: &Path, model: &str) -> PathBuf {
    let path = Path::new(model);
    if path.is_absolute() || model.contains('/') {
        path.to_path_buf()
    } else {
        models.join(model)
    }
}

/// Validate a targeted choice before committing. No downloads or worker spawn.
pub fn validate_ready_in(engine: &str, model: &str, models: &Path) -> Result<(), String> {
    validate_backend(engine, model)?;
    if !matches!(engine, "parakeet" | "cohere") {
        return Ok(());
    }
    let path = model_path_in(models, model);
    let present = if is_gguf(model) {
        match crate::setup::model::find_gguf_model(model) {
            Some(checkpoint) => crate::setup::model::validate_gguf_model(&path, checkpoint).is_ok(),
            None => crate::setup::model::validate_download(
                &path,
                None,
                crate::setup::model::ContentCheck::Gguf,
            )
            .is_ok(),
        }
    } else {
        crate::model_catalog::model_health_in(models, engine, model)
            == crate::model_catalog::ModelHealth::Present
    };
    if !present {
        return Err(format!("{engine} model '{model}' is missing or incomplete at {}. Config unchanged. Select an installed compatible model, or consent to download it: {}", path.display(), crate::model_catalog::download_command(engine, model)));
    }
    Ok(())
}

pub fn validate_ready(engine: &str, model: &str) -> Result<(), String> {
    validate_ready_in(engine, model, &Config::models_dir())?;
    #[cfg(any(feature = "parakeet-gguf", feature = "cohere-gguf"))]
    if matches!(engine, "parakeet" | "cohere") && is_gguf(model) {
        let worker = crate::transcribe::gguf_subprocess::worker_path();
        if which::which(&worker).is_err() {
            return Err(format!("Matching GGUF worker '{}' is missing or not executable. Install the worker packaged with this application, or set VOXTYPE_GGUF_WORKER to a compatible executable. Config unchanged.", worker.display()));
        }
    }
    Ok(())
}

/// Resolve only an intentional engine switch. Never rewrite on config load.
pub fn select_engine_model_in(
    engine: &str,
    retained: Option<&str>,
    models: &Path,
) -> Result<String, String> {
    if let Some(model) = retained {
        if BackendCapabilities::compiled().supports(engine, model) {
            validate_ready_in(engine, model, models)?;
            return Ok(model.into());
        }
    }
    let default = crate::model_catalog::default_model(engine);
    let choice = std::iter::once(default)
        .chain(crate::model_catalog::model_catalog(engine))
        .find(|model| validate_ready_in(engine, model, models).is_ok());
    match choice {
        Some(model) => Ok(model.into()),
        None => Err(format!(
            "No installed compatible {engine} model is available. {}",
            validate_ready_in(engine, retained.unwrap_or(default), models).unwrap_err()
        )),
    }
}

/// Read the final file snapshot before any frontend requests a service restart.
pub fn validate_saved_for_restart(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let config: Config = toml::from_str(&text).map_err(|error| error.to_string())?;
    validate_ready(config.engine.name(), config.model_name())?;
    validate_idle_for_restart(&config)
}

/// Refuse a restart while an utterance or meeting is in progress.
pub fn validate_idle_for_restart(config: &Config) -> Result<(), String> {
    if !crate::daemon_status::is_daemon_running() {
        return Ok(());
    }
    let state = config
        .resolve_state_file()
        .and_then(|path| std::fs::read_to_string(path).ok());
    let meeting_idle = match std::fs::read_to_string(Config::runtime_dir().join("meeting_state")) {
        Ok(state) => state
            .lines()
            .next()
            .is_some_and(|value| matches!(value.trim(), "idle" | "stopped")),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    };
    if meeting_idle && state.as_deref().is_some_and(|state| state.trim() == "idle") {
        Ok(())
    } else {
        Err("Daemon is busy or idle state cannot be verified. Finish or cancel the current utterance/meeting before explicitly restarting. No restart performed.".into())
    }
}

/// Runtime routing errors must identify the unavailable format, not the engine.
pub fn validate_config_backend(config: &Config) -> Result<(), String> {
    if matches!(
        config.engine,
        TranscriptionEngine::Parakeet | TranscriptionEngine::Cohere
    ) {
        validate_backend(config.engine.name(), config.model_name())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_custom_checkpoint_and_missing_file_are_not_silently_replaced() {
        for engine in ["parakeet", "cohere"] {
            if !BackendCapabilities::compiled().supports(engine, "custom.gguf") {
                continue;
            }
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("custom.gguf");
            std::fs::write(&path, b"GGUFtest fixture, not inference weights").unwrap();
            let name = path.to_str().unwrap();
            assert_eq!(
                select_engine_model_in(engine, Some(name), directory.path()).unwrap(),
                name
            );
            std::fs::remove_file(&path).unwrap();
            assert!(select_engine_model_in(engine, Some(name), directory.path())
                .unwrap_err()
                .contains("missing"));
        }
    }

    #[test]
    fn incompatible_retained_model_selects_only_an_installed_supported_checkpoint() {
        for (engine, legacy) in [
            ("parakeet", "parakeet-tdt-0.6b-v2"),
            ("cohere", "cohere-transcribe-int8"),
        ] {
            let capabilities = BackendCapabilities::compiled();
            if !capabilities.supports(engine, "model.gguf") || capabilities.supports(engine, legacy)
            {
                continue;
            }
            let directory = tempfile::tempdir().unwrap();
            assert!(select_engine_model_in(engine, Some(legacy), directory.path()).is_err());
            let name = crate::model_catalog::default_model(engine);
            let checkpoint = crate::setup::model::find_gguf_model(name).unwrap();
            let file = std::fs::File::create(directory.path().join(name)).unwrap();
            file.set_len(checkpoint.size).unwrap();
            use std::io::Write;
            (&file).write_all(b"GGUF").unwrap();
            assert_eq!(
                select_engine_model_in(engine, Some(legacy), directory.path()).unwrap(),
                name
            );
        }
    }

    #[test]
    fn format_capabilities_preserve_explicit_legacy_and_custom_paths() {
        let legacy = BackendCapabilities {
            parakeet_onnx: true,
            parakeet_gguf: false,
            cohere_onnx: true,
            cohere_gguf: false,
        };
        let gguf = BackendCapabilities {
            parakeet_onnx: false,
            parakeet_gguf: true,
            cohere_onnx: false,
            cohere_gguf: true,
        };
        for engine in ["parakeet", "cohere"] {
            assert!(legacy.supports(engine, "/custom/legacy-model"));
            assert!(!legacy.supports(engine, "/custom/model.GGUF"));
            assert!(!gguf.supports(engine, "/custom/legacy-model"));
            assert!(gguf.supports(engine, "/custom/model.GGUF"));
        }
    }
}
