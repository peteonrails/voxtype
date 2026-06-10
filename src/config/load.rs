use super::parse::parse_config_with_defaults;
use super::{Config, LanguageConfig, OutputMode, SonioxConfig, TranscriptionEngine};
use crate::error::VoxtypeError;
use std::path::{Path, PathBuf};

/// Parse a boolean from an environment variable value.
/// Only "1" and "true" (case-insensitive) are truthy; everything else is falsy.
fn parse_bool_env(val: &str) -> bool {
    val == "1" || val.eq_ignore_ascii_case("true")
}

pub fn load_config(path: Option<&Path>) -> Result<Config, VoxtypeError> {
    // Start with defaults
    let mut config = Config::default();

    // Determine config file path. If --config wasn't passed, walk the
    // documented lookup chain: user config -> /etc/voxtype/config.toml.
    let config_path = match path {
        Some(p) => Some(PathBuf::from(p)),
        None => Config::resolve_existing_path(),
    };

    // Load from file if it exists. Layer the user's TOML over a default-rendered
    // Config so partial sections inherit defaults for fields the user didn't set.
    // Without this, a user config containing only `[hotkey]` or `[audio.feedback]`
    // would fail with "missing field 'audio'" / "missing field 'device'", because
    // serde's per-field defaults only fill in what the user omitted at the *field*
    // level, not what they omitted at a parent-section level (#421).
    if let Some(ref path) = config_path {
        if path.exists() {
            tracing::debug!("Loading config from {:?}", path);
            let contents = std::fs::read_to_string(path)
                .map_err(|e| VoxtypeError::Config(format!("Failed to read config: {}", e)))?;

            config = parse_config_with_defaults(&contents)
                .map_err(|e| VoxtypeError::Config(format!("Invalid config: {}", e)))?;
        } else {
            tracing::debug!("Config file not found at {:?}, using defaults", path);
        }
    } else {
        tracing::debug!("No config file found at user or system path, using built-in defaults");
    }

    // Override from environment variables
    // Hotkey
    if let Ok(key) = std::env::var("VOXTYPE_HOTKEY") {
        config.hotkey.key = key;
    }
    if let Ok(val) = std::env::var("VOXTYPE_HOTKEY_ENABLED") {
        config.hotkey.enabled = parse_bool_env(&val);
    }
    if let Ok(key) = std::env::var("VOXTYPE_CANCEL_KEY") {
        config.hotkey.cancel_key = Some(key);
    }

    // Whisper / engine
    if let Ok(model) = std::env::var("VOXTYPE_MODEL") {
        config.whisper.model = model;
    }
    if let Ok(engine) = std::env::var("VOXTYPE_ENGINE") {
        match engine.parse::<TranscriptionEngine>() {
            Ok(e) => config.engine = e,
            Err(_) => tracing::warn!("Unknown VOXTYPE_ENGINE value: {}", engine),
        }
    }
    if let Ok(lang) = std::env::var("VOXTYPE_LANGUAGE") {
        config.whisper.language = LanguageConfig::from_comma_separated(&lang);
    }
    if let Ok(val) = std::env::var("VOXTYPE_TRANSLATE") {
        config.whisper.translate = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_THREADS") {
        if let Ok(n) = val.parse::<usize>() {
            config.whisper.threads = Some(n);
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_GPU_ISOLATION") {
        config.whisper.gpu_isolation = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_GPU_DEVICE") {
        if let Ok(n) = val.parse::<i32>() {
            config.whisper.gpu_device = Some(n);
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_FLASH_ATTENTION") {
        config.whisper.flash_attention = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_ON_DEMAND_LOADING") {
        config.whisper.on_demand_loading = parse_bool_env(&val);
    }

    // Audio
    if let Ok(device) = std::env::var("VOXTYPE_AUDIO_DEVICE") {
        config.audio.device = device;
    }
    if let Ok(val) = std::env::var("VOXTYPE_MAX_DURATION_SECS") {
        if let Ok(n) = val.parse::<u32>() {
            config.audio.max_duration_secs = n;
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_AUDIO_FEEDBACK") {
        config.audio.feedback.enabled = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_PAUSE_MEDIA") {
        config.audio.pause_media = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_WAIT_FOR_DEVICE") {
        config.audio.wait_for_device = parse_bool_env(&val);
    }

    // Output
    if let Ok(mode) = std::env::var("VOXTYPE_OUTPUT_MODE") {
        config.output.mode = match mode.to_lowercase().as_str() {
            "clipboard" => OutputMode::Clipboard,
            "paste" => OutputMode::Paste,
            "file" => OutputMode::File,
            _ => OutputMode::Type,
        };
    }
    if let Ok(append_text) = std::env::var("VOXTYPE_APPEND_TEXT") {
        config.output.append_text = Some(append_text);
    }
    if std::env::var("VOXTYPE_WTYPE_SHIFT_PREFIX")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        config.output.wtype_shift_prefix = true;
    }
    if let Ok(val) = std::env::var("VOXTYPE_AUTO_SUBMIT") {
        config.output.auto_submit = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_SHIFT_ENTER_NEWLINES") {
        config.output.shift_enter_newlines = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_PRE_TYPE_DELAY") {
        if let Ok(n) = val.parse::<u32>() {
            config.output.pre_type_delay_ms = n;
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_TYPE_DELAY") {
        if let Ok(n) = val.parse::<u32>() {
            config.output.type_delay_ms = n;
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_FALLBACK_TO_CLIPBOARD") {
        config.output.fallback_to_clipboard = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_SPOKEN_PUNCTUATION") {
        config.text.spoken_punctuation = parse_bool_env(&val);
    }
    if let Ok(keys) = std::env::var("VOXTYPE_PASTE_KEYS") {
        config.output.paste_keys = Some(keys);
    }
    if let Ok(layout) = std::env::var("VOXTYPE_DOTOOL_XKB_LAYOUT") {
        config.output.dotool_xkb_layout = Some(layout);
    }
    if let Ok(variant) = std::env::var("VOXTYPE_DOTOOL_XKB_VARIANT") {
        config.output.dotool_xkb_variant = Some(variant);
    }
    if let Ok(layout) = std::env::var("VOXTYPE_EITYPE_XKB_LAYOUT") {
        config.output.eitype_xkb_layout = Some(layout);
    }
    if let Ok(variant) = std::env::var("VOXTYPE_EITYPE_XKB_VARIANT") {
        config.output.eitype_xkb_variant = Some(variant);
    }

    // Remote whisper
    if let Ok(endpoint) = std::env::var("VOXTYPE_REMOTE_ENDPOINT") {
        config.whisper.remote_endpoint = Some(endpoint);
    }
    if let Ok(key) = std::env::var("VOXTYPE_WHISPER_API_KEY") {
        config.whisper.remote_api_key = Some(key);
    }

    // Soniox
    if let Ok(key) = std::env::var("SONIOX_API_KEY") {
        config
            .soniox
            .get_or_insert_with(SonioxConfig::default)
            .api_key = Some(key);
    }
    if let Ok(val) = std::env::var("VOXTYPE_RESTORE_CLIPBOARD") {
        config.output.restore_clipboard = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_RESTORE_CLIPBOARD_DELAY_MS") {
        if let Ok(ms) = val.parse::<u32>() {
            config.output.restore_clipboard_delay_ms = ms;
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_SMART_AUTO_SUBMIT") {
        config.text.smart_auto_submit = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_FILTER_FILLERS") {
        config.text.filter_filler_words = parse_bool_env(&val);
    }

    Ok(config)
}

/// Save configuration to file
#[allow(dead_code)]
pub fn save_config(config: &Config, path: &Path) -> Result<(), VoxtypeError> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| VoxtypeError::Config(format!("Failed to create config dir: {}", e)))?;
    }

    let contents = toml::to_string_pretty(config)
        .map_err(|e| VoxtypeError::Config(format!("Failed to serialize config: {}", e)))?;

    std::fs::write(path, contents)
        .map_err(|e| VoxtypeError::Config(format!("Failed to write config: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_config_explicit_path() {
        // Explicit --config should always be used regardless of fallback.
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
                [hotkey]
                key = "F12"

                [audio]
                device = "default"
                sample_rate = 16000
                max_duration_secs = 30

                [whisper]
                model = "tiny.en"
                language = "en"

                [output]
                mode = "clipboard"
            "#,
        )
        .unwrap();

        let config = load_config(Some(&config_path)).unwrap();
        assert_eq!(config.hotkey.key, "F12");
        assert_eq!(config.whisper.model, "tiny.en");
        assert_eq!(config.output.mode, OutputMode::Clipboard);
    }
}
