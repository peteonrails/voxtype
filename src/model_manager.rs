//! Model manager for multi-model support
//!
//! Manages the lifecycle of Whisper models, supporting:
//! - LRU caching of loaded models (when gpu_isolation = false)
//! - On-demand loading with automatic eviction
//! - Fresh subprocess per model (when gpu_isolation = true)
//! - Remote backend model selection

use crate::config::{WhisperConfig, WhisperMode};
use crate::error::TranscribeError;
use crate::transcribe::{self, Transcriber};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A loaded model with usage tracking
struct LoadedModel {
    transcriber: Arc<dyn Transcriber>,
    last_used: Instant,
    is_primary: bool,
}

/// Manages multiple Whisper models with LRU eviction
pub struct ModelManager {
    /// Whisper configuration
    config: WhisperConfig,
    /// Path to config file (for subprocess transcribers)
    config_path: Option<PathBuf>,
    /// Loaded models (keyed by model name)
    loaded_models: HashMap<String, LoadedModel>,
    /// Maximum models to keep loaded
    max_loaded: usize,
    /// Timeout before evicting idle models
    cold_timeout: Duration,
}

impl ModelManager {
    /// Create a new model manager
    pub fn new(config: &WhisperConfig, config_path: Option<PathBuf>) -> Self {
        Self {
            config: config.clone(),
            config_path,
            loaded_models: HashMap::new(),
            max_loaded: config.max_loaded_models,
            cold_timeout: Duration::from_secs(config.cold_model_timeout_secs),
        }
    }

    /// Check if a model is available (configured as primary, secondary, or in available_models)
    pub fn is_model_available(&self, model: &str) -> bool {
        if model == self.config.model {
            return true;
        }
        if let Some(ref secondary) = self.config.secondary_model {
            if model == secondary {
                return true;
            }
        }
        self.config.available_models.contains(&model.to_string())
    }

    /// Get a transcriber for the specified model
    ///
    /// For GPU isolation mode, creates a fresh subprocess transcriber each time.
    /// For non-isolation mode, returns cached transcriber or loads on demand.
    pub fn get_transcriber(
        &mut self,
        model: Option<&str>,
    ) -> Result<Arc<dyn Transcriber>, TranscribeError> {
        // Clone the model name to avoid borrow issues
        let model_name = model
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.config.model.clone());

        // Validate model is available
        if !self.is_model_available(&model_name) {
            tracing::warn!(
                "Requested model '{}' not in available models, falling back to default '{}'",
                model_name,
                self.config.model
            );
            return self.get_transcriber(None);
        }

        // For remote backend, create transcriber with model override
        if self.config.effective_mode() == WhisperMode::Remote {
            return self.create_remote_transcriber(&model_name);
        }

        // For CLI backend, create transcriber each time (no caching needed)
        if self.config.effective_mode() == WhisperMode::Cli {
            return self.create_cli_transcriber(&model_name);
        }

        // For GPU isolation, always create fresh subprocess
        if self.config.gpu_isolation {
            return self.create_subprocess_transcriber(&model_name);
        }

        // For non-isolated local backend, use LRU cache
        self.get_or_load_cached(&model_name)
    }

    /// Create a remote transcriber with model override
    fn create_remote_transcriber(
        &self,
        model: &str,
    ) -> Result<Arc<dyn Transcriber>, TranscribeError> {
        let mut config = self.config.clone();
        // Override remote_model with requested model
        config.remote_model = Some(model.to_string());
        let transcriber = transcribe::remote::RemoteTranscriber::new(&config)?;
        Ok(Arc::new(transcriber))
    }

    /// Create a CLI transcriber with model override
    fn create_cli_transcriber(&self, model: &str) -> Result<Arc<dyn Transcriber>, TranscribeError> {
        let mut config = self.config.clone();
        config.model = model.to_string();
        tracing::info!("Using whisper-cli subprocess backend");
        let transcriber = transcribe::cli::CliTranscriber::new(&config)?;
        Ok(Arc::new(transcriber))
    }

    /// Create a subprocess transcriber for the specified model
    fn create_subprocess_transcriber(
        &self,
        model: &str,
    ) -> Result<Arc<dyn Transcriber>, TranscribeError> {
        let mut config = self.config.clone();
        config.model = model.to_string();
        let transcriber =
            transcribe::subprocess::SubprocessTranscriber::new(&config, self.config_path.clone())?;
        Ok(Arc::new(transcriber))
    }

    /// Get transcriber from cache or load on demand (non-isolated mode)
    fn get_or_load_cached(&mut self, model: &str) -> Result<Arc<dyn Transcriber>, TranscribeError> {
        // Check if already loaded
        if let Some(loaded) = self.loaded_models.get_mut(model) {
            loaded.last_used = Instant::now();
            tracing::debug!("Using cached model '{}'", model);
            return Ok(Arc::clone(&loaded.transcriber));
        }

        // Need to load - first check if we need to evict
        if self.loaded_models.len() >= self.max_loaded {
            self.evict_lru();
        }

        // Load the model
        tracing::info!("Loading model '{}' into cache", model);
        let mut config = self.config.clone();
        config.model = model.to_string();

        let transcriber = transcribe::whisper::WhisperTranscriber::new(&config)?;
        let is_primary = model == self.config.model;

        self.loaded_models.insert(
            model.to_string(),
            LoadedModel {
                transcriber: Arc::new(transcriber),
                last_used: Instant::now(),
                is_primary,
            },
        );

        Ok(Arc::clone(
            &self.loaded_models.get(model).unwrap().transcriber,
        ))
    }

    /// Evict the least recently used non-primary model
    fn evict_lru(&mut self) {
        // Find LRU non-primary model
        let lru_model = self
            .loaded_models
            .iter()
            .filter(|(_, m)| !m.is_primary)
            .min_by_key(|(_, m)| m.last_used)
            .map(|(name, _)| name.clone());

        if let Some(model) = lru_model {
            tracing::info!("Evicting model '{}' from cache (LRU)", model);
            self.loaded_models.remove(&model);
        }
    }

    /// Evict models that haven't been used recently
    ///
    /// Call this periodically (e.g., every 60 seconds) to free memory
    /// from models that are no longer being actively used.
    pub fn evict_idle_models(&mut self) {
        if self.cold_timeout.is_zero() {
            return; // Auto-eviction disabled
        }

        let cutoff = Instant::now() - self.cold_timeout;
        let to_evict: Vec<String> = self
            .loaded_models
            .iter()
            .filter(|(_, m)| !m.is_primary && m.last_used < cutoff)
            .map(|(name, _)| name.clone())
            .collect();

        for model in to_evict {
            tracing::info!(
                "Evicting idle model '{}' from cache (unused for {}s)",
                model,
                self.cold_timeout.as_secs()
            );
            self.loaded_models.remove(&model);
        }
    }

    /// Preload the primary model (if on_demand_loading is false)
    pub fn preload_primary(&mut self) -> Result<(), TranscribeError> {
        if self.config.on_demand_loading {
            tracing::debug!("Skipping primary model preload (on_demand_loading=true)");
            return Ok(());
        }

        if self.config.gpu_isolation {
            tracing::debug!("Skipping primary model preload (gpu_isolation=true)");
            return Ok(());
        }

        if self.config.effective_mode() == WhisperMode::Remote {
            tracing::debug!("Skipping primary model preload (remote backend)");
            return Ok(());
        }

        if self.config.effective_mode() == WhisperMode::Cli {
            tracing::debug!("Skipping primary model preload (cli backend)");
            return Ok(());
        }

        let model = self.config.model.clone();
        tracing::info!("Preloading primary model '{}'", model);
        let _ = self.get_or_load_cached(&model)?;
        Ok(())
    }

    /// Prepare a model for transcription (called when recording starts)
    ///
    /// For subprocess mode, this spawns the worker early so it can load
    /// the model while the user is speaking.
    pub fn prepare_model(&mut self, model: Option<&str>) -> Result<(), TranscribeError> {
        let model_name = model
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.config.model.clone());

        // Validate model
        if !self.is_model_available(&model_name) {
            tracing::warn!(
                "Cannot prepare unavailable model '{}', will use default",
                model_name
            );
            return Ok(());
        }

        // For GPU isolation, spawn subprocess early
        if self.config.gpu_isolation && self.config.effective_mode() == WhisperMode::Local {
            // Create and prepare subprocess transcriber
            let transcriber = self.create_subprocess_transcriber(&model_name)?;
            transcriber.prepare();
            // Store it temporarily for the upcoming transcription
            self.loaded_models.insert(
                format!("_prepared_{}", model_name),
                LoadedModel {
                    transcriber,
                    last_used: Instant::now(),
                    is_primary: false,
                },
            );
        }

        Ok(())
    }

    /// Get a prepared transcriber (if available) or create one
    ///
    /// This checks for a prepared subprocess transcriber first,
    /// then falls back to normal get_transcriber.
    pub fn get_prepared_transcriber(
        &mut self,
        model: Option<&str>,
    ) -> Result<Arc<dyn Transcriber>, TranscribeError> {
        let model_name = model
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.config.model.clone());
        let prepared_key = format!("_prepared_{}", model_name);

        // Check for prepared transcriber
        if let Some(prepared) = self.loaded_models.remove(&prepared_key) {
            tracing::debug!("Using prepared transcriber for model '{}'", model_name);
            return Ok(prepared.transcriber);
        }

        // No prepared transcriber, get normally
        self.get_transcriber(Some(&model_name))
    }

    /// Get the list of currently loaded models (for debugging/status)
    pub fn loaded_model_names(&self) -> Vec<&str> {
        self.loaded_models
            .keys()
            .filter(|k| !k.starts_with("_prepared_"))
            .map(|s| s.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> WhisperConfig {
        WhisperConfig {
            mode: Some(WhisperMode::Local),
            model: "base.en".to_string(),
            threads: Some(4),
            secondary_model: Some("large-v3-turbo".to_string()),
            available_models: vec!["medium.en".to_string()],
            max_loaded_models: 2,
            cold_model_timeout_secs: 300,
            ..Default::default()
        }
    }

    #[test]
    fn test_model_availability() {
        let config = test_config();
        let manager = ModelManager::new(&config, None);

        // Primary model is available
        assert!(manager.is_model_available("base.en"));

        // Secondary model is available
        assert!(manager.is_model_available("large-v3-turbo"));

        // Model in available_models is available
        assert!(manager.is_model_available("medium.en"));

        // Unknown model is not available
        assert!(!manager.is_model_available("tiny.en"));
    }

    #[test]
    fn test_new_manager() {
        let config = test_config();
        let manager = ModelManager::new(&config, None);

        assert_eq!(manager.max_loaded, 2);
        assert_eq!(manager.cold_timeout, Duration::from_secs(300));
        assert!(manager.loaded_models.is_empty());
    }
}
