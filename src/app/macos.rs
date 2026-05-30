//! macOS first-launch auto-setup. Only compiled when the binary is launched as
//! a `.app` bundle on macOS.

use voxtype::{config, setup};

/// First-launch auto-setup for macOS app bundle launches.
///
/// Detects if this is the first launch by checking for a config file and downloaded model.
/// If either is missing, creates default config and downloads the recommended model
/// so the user can start recording immediately after granting permissions.
pub(crate) async fn first_launch_setup(_config: &config::Config) {
    // Check if config file exists
    let config_exists = config::Config::default_path()
        .map(|p| p.exists())
        .unwrap_or(false);

    // Check if any model is already downloaded
    let models_dir = config::Config::models_dir();
    let has_model = models_dir.exists()
        && std::fs::read_dir(&models_dir)
            .map(|entries| {
                entries.filter_map(|e| e.ok()).any(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    // Whisper models (ggml-*.bin) or Parakeet/ONNX model dirs
                    (name.starts_with("ggml-") && name.ends_with(".bin"))
                        || (e.path().is_dir() && e.path().join("encoder-model.onnx").exists())
                })
            })
            .unwrap_or(false);

    if config_exists && has_model {
        return; // Not first launch
    }

    // Create default config if missing
    if !config_exists {
        if let Some(config_path) = config::Config::default_path() {
            if let Some(parent) = config_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let content = config::default_config_content();
            if std::fs::write(&config_path, &content).is_ok() {
                tracing::info!("Created default config: {:?}", config_path);
            }
        }
    }

    // Download default model if none present
    if !has_model {
        // Detect system language to choose the right model
        let is_english = tokio::process::Command::new("defaults")
            .args(["read", "NSGlobalDomain", "AppleLanguages"])
            .output()
            .await
            .map(|o| {
                let s = String::from_utf8_lossy(&o.stdout);
                s.lines()
                    .find(|l| l.trim().starts_with('"'))
                    .map(|l| {
                        l.trim()
                            .trim_matches(|c| c == '"' || c == ',')
                            .starts_with("en")
                    })
                    .unwrap_or(true)
            })
            .unwrap_or(true);

        // Show notification that model is downloading
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                "display notification \"Downloading speech model (this may take a minute)...\" with title \"Voxtype\"",
            ])
            .status();

        #[cfg(feature = "parakeet")]
        let download_result = if is_english {
            tracing::info!("First launch: downloading Parakeet model");
            setup::model::download_parakeet_model("parakeet-tdt-0.6b-v3-int8")
                .and_then(|_| setup::model::set_parakeet_config("parakeet-tdt-0.6b-v3-int8"))
        } else {
            tracing::info!("First launch: downloading Whisper base model");
            setup::model::download_model("base")
                .and_then(|_| setup::model::set_model_config("base"))
        };

        #[cfg(not(feature = "parakeet"))]
        let download_result = {
            let model = if is_english { "base.en" } else { "base" };
            tracing::info!("First launch: downloading Whisper {} model", model);
            setup::model::download_model(model).and_then(|_| setup::model::set_model_config(model))
        };

        match download_result {
            Ok(_) => {
                let _ = std::process::Command::new("osascript")
                    .args([
                        "-e",
                        "display notification \"Ready! Press fn to start recording.\" with title \"Voxtype\"",
                    ])
                    .status();
            }
            Err(e) => {
                tracing::error!("Failed to download model: {}", e);
                let msg = format!(
                    "display notification \"Model download failed: {}. Run 'voxtype setup model' to try again.\" with title \"Voxtype\"",
                    e.to_string().replace('"', "'")
                );
                let _ = std::process::Command::new("osascript")
                    .args(["-e", &msg])
                    .status();
            }
        }
    }
}
