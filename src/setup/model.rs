//! Interactive model selection and download

use super::{print_failure, print_info, print_success, print_warning};
use crate::config::Config;
use crate::transcribe::whisper::{get_model_filename, get_model_url};
use std::io::{self, Write};
use std::process::Command;

/// Model information for display
struct ModelInfo {
    name: &'static str,
    size_mb: u32,
    description: &'static str,
    english_only: bool,
}

const MODELS: &[ModelInfo] = &[
    // Tiny models
    ModelInfo {
        name: "tiny",
        size_mb: 75,
        description: "Fastest, lowest accuracy",
        english_only: false,
    },
    ModelInfo {
        name: "tiny.en",
        size_mb: 39,
        description: "Fastest, lowest accuracy",
        english_only: true,
    },
    // Base models
    ModelInfo {
        name: "base",
        size_mb: 142,
        description: "Good balance (default)",
        english_only: false,
    },
    ModelInfo {
        name: "base.en",
        size_mb: 142,
        description: "Good balance (default)",
        english_only: true,
    },
    // Small models
    ModelInfo {
        name: "small",
        size_mb: 466,
        description: "Better accuracy",
        english_only: false,
    },
    ModelInfo {
        name: "small.en",
        size_mb: 466,
        description: "Better accuracy",
        english_only: true,
    },
    // Medium models
    ModelInfo {
        name: "medium",
        size_mb: 1500,
        description: "High accuracy",
        english_only: false,
    },
    ModelInfo {
        name: "medium.en",
        size_mb: 1500,
        description: "High accuracy",
        english_only: true,
    },
    // Large models
    ModelInfo {
        name: "large-v3",
        size_mb: 3100,
        description: "Best accuracy",
        english_only: false,
    },
    ModelInfo {
        name: "large-v3-turbo",
        size_mb: 1600,
        description: "Fast + accurate (recommended for GPU)",
        english_only: false,
    },
];

/// Check if a model name is valid
pub fn is_valid_model(name: &str) -> bool {
    MODELS.iter().any(|m| m.name == name)
}

/// Get list of valid model names (for error messages)
pub fn valid_model_names() -> Vec<&'static str> {
    MODELS.iter().map(|m| m.name).collect()
}

/// Run interactive model selection
pub async fn interactive_select() -> anyhow::Result<()> {
    println!("Voxtype Model Selection\n");
    println!("=======================\n");

    let models_dir = Config::models_dir();
    println!("Models directory: {:?}\n", models_dir);

    // Show available models with status
    println!("Available Whisper Models:\n");

    for (i, model) in MODELS.iter().enumerate() {
        let filename = get_model_filename(model.name);
        let model_path = models_dir.join(&filename);
        let installed = model_path.exists();

        let status = if installed {
            "\x1b[32m[installed]\x1b[0m"
        } else {
            ""
        };

        let lang = if model.english_only {
            "English"
        } else {
            "Multilingual"
        };

        println!(
            "  [{:>2}] {:<16} ({:>4} MB) - {} ({}) {}",
            i + 1,
            model.name,
            model.size_mb,
            model.description,
            lang,
            status
        );
    }

    println!("\n  [ 0] Cancel\n");

    // Get user selection
    print!("Select model to download [0-{}]: ", MODELS.len());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let selection: usize = input.trim().parse().unwrap_or(0);

    if selection == 0 || selection > MODELS.len() {
        println!("\nCancelled.");
        return Ok(());
    }

    let model = &MODELS[selection - 1];
    let filename = get_model_filename(model.name);
    let model_path = models_dir.join(&filename);

    // Check if already installed
    if model_path.exists() {
        println!("\nModel '{}' is already installed.\n", model.name);
        println!("  [1] Set as default model (update config)");
        println!("  [2] Re-download");
        println!("  [0] Cancel\n");

        print!("Select option [1]: ");
        io::stdout().flush()?;

        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let choice = choice.trim();

        match choice {
            "" | "1" => {
                // Set as default without re-downloading
                update_config_model(model.name)?;
                println!("\n---");
                println!("Model setup complete! Run 'voxtype' to start using it.");
                return Ok(());
            }
            "2" => {
                // Continue to download below
            }
            _ => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    }

    // Download the model
    download_model(model.name)?;

    // Offer to update config
    println!("\nWould you like to set this as your default model?");
    print!("Update config to use {}? [Y/n]: ", model.name);
    io::stdout().flush()?;

    let mut update_config = String::new();
    io::stdin().read_line(&mut update_config)?;

    if update_config.trim().is_empty() || update_config.trim().eq_ignore_ascii_case("y") {
        update_config_model(model.name)?;
    }

    println!("\n---");
    println!("Model setup complete! Run 'voxtype' to start using it.");

    Ok(())
}

/// Download a specific model using curl
pub fn download_model(model_name: &str) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();
    let filename = get_model_filename(model_name);
    let model_path = models_dir.join(&filename);

    // Ensure directory exists
    std::fs::create_dir_all(&models_dir)?;

    let url = get_model_url(model_name);

    println!("\nDownloading {}...", model_name);
    println!("URL: {}", url);

    // Use curl for downloading - it handles progress display and redirects
    let status = Command::new("curl")
        .args([
            "-L",             // Follow redirects
            "--progress-bar", // Show progress bar
            "-o",
            model_path.to_str().unwrap_or("model.bin"),
            &url,
        ])
        .status();

    match status {
        Ok(exit_status) if exit_status.success() => {
            print_success(&format!("Saved to {:?}", model_path));
            Ok(())
        }
        Ok(exit_status) => {
            print_failure(&format!(
                "Download failed: curl exited with code {}",
                exit_status.code().unwrap_or(-1)
            ));
            // Clean up partial download
            let _ = std::fs::remove_file(&model_path);
            anyhow::bail!("Download failed")
        }
        Err(e) => {
            print_failure(&format!("Failed to run curl: {}", e));
            print_info("Please ensure curl is installed (e.g., 'sudo pacman -S curl')");
            anyhow::bail!("curl not available: {}", e)
        }
    }
}

/// Set a specific model as the default (must already be downloaded)
pub async fn set_model(model_name: &str, restart: bool) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();
    let filename = get_model_filename(model_name);
    let model_path = models_dir.join(&filename);

    // Verify the model exists
    if !model_path.exists() {
        print_failure(&format!("Model '{}' is not installed", model_name));
        println!("\n  Run 'voxtype setup model' to download it first.");
        println!("  Or 'voxtype setup model --list' to see installed models.");
        anyhow::bail!("Model not installed: {}", model_name);
    }

    // Update the config
    update_config_model(model_name)?;

    if restart {
        println!("  Restarting daemon...");
        let status = tokio::process::Command::new("systemctl")
            .args(["--user", "restart", "voxtype"])
            .status()
            .await;

        match status {
            Ok(s) if s.success() => {
                print_success("Daemon restarted with new model");
            }
            _ => {
                print_warning("Could not restart daemon (not running as systemd service?)");
                print_info("Restart manually: systemctl --user restart voxtype");
            }
        }
    } else {
        print_info("Restart daemon to use new model: systemctl --user restart voxtype");
        println!(
            "       Or use: voxtype setup model --set {} --restart",
            model_name
        );
    }

    Ok(())
}

/// List installed models
pub fn list_installed() {
    println!("Installed Whisper Models\n");
    println!("========================\n");

    let models_dir = Config::models_dir();

    if !models_dir.exists() {
        println!("No models directory found: {:?}", models_dir);
        return;
    }

    let mut found = false;

    for model in MODELS {
        let filename = get_model_filename(model.name);
        let model_path = models_dir.join(&filename);

        if model_path.exists() {
            let size = std::fs::metadata(&model_path)
                .map(|m| m.len() as f64 / 1024.0 / 1024.0)
                .unwrap_or(0.0);

            println!("  {} ({:.0} MB) - {}", model.name, size, model.description);
            found = true;
        }
    }

    if !found {
        println!("  No models installed.");
        println!("\n  Run 'voxtype setup model' to download a model.");
    }
}

/// Update the config file to use a specific model (with status messages)
fn update_config_model(model_name: &str) -> anyhow::Result<()> {
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let updated = update_model_in_config(&content, model_name);
            std::fs::write(&config_path, updated)?;
            print_success(&format!("Config updated to use '{}' model", model_name));
            Ok(())
        } else {
            print_info("No config file found. Run 'voxtype setup' first.");
            Ok(())
        }
    } else {
        anyhow::bail!("Could not determine config path")
    }
}

/// Update the config file to use a specific model (quiet, no output)
pub fn set_model_config(model_name: &str) -> anyhow::Result<()> {
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let updated = update_model_in_config(&content, model_name);
            std::fs::write(&config_path, updated)?;
        }
        // Silently succeed if config doesn't exist yet - setup will create it
        Ok(())
    } else {
        anyhow::bail!("Could not determine config path")
    }
}

/// Update the model setting in a config string
fn update_model_in_config(config: &str, model_name: &str) -> String {
    // Simple regex-free replacement for the model line
    let mut result = String::new();
    let mut in_whisper_section = false;

    for line in config.lines() {
        let trimmed = line.trim();

        // Track if we're in the [whisper] section
        if trimmed.starts_with('[') {
            in_whisper_section = trimmed == "[whisper]";
        }

        // Replace model line in whisper section
        if in_whisper_section && trimmed.starts_with("model") {
            result.push_str(&format!("model = \"{}\"\n", model_name));
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    // Remove trailing newline if original didn't have one
    if !config.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_model_in_config_basic() {
        let config = r#"[whisper]
model = "base.en"
language = "en"
"#;
        let result = update_model_in_config(config, "large-v3");
        assert!(result.contains(r#"model = "large-v3""#));
        assert!(!result.contains("base.en"));
    }

    #[test]
    fn test_update_model_in_config_preserves_other_sections() {
        let config = r#"[hotkey]
key = "SCROLLLOCK"

[whisper]
model = "base.en"
language = "en"

[output]
mode = "type"
"#;
        let result = update_model_in_config(config, "small.en");
        assert!(result.contains(r#"model = "small.en""#));
        assert!(result.contains(r#"key = "SCROLLLOCK""#));
        assert!(result.contains(r#"mode = "type""#));
        assert!(result.contains("[hotkey]"));
        assert!(result.contains("[output]"));
    }

    #[test]
    fn test_update_model_in_config_only_changes_whisper_section() {
        // If there's a "model" key in another section, it should not be changed
        let config = r#"[some_other_section]
model = "should_not_change"

[whisper]
model = "base.en"
"#;
        let result = update_model_in_config(config, "large-v3");
        assert!(result.contains(r#"model = "should_not_change""#));
        assert!(result.contains(r#"model = "large-v3""#));
    }

    #[test]
    fn test_update_model_in_config_handles_comments() {
        let config = r#"[whisper]
# Model to use
model = "base.en"
# Language setting
language = "en"
"#;
        let result = update_model_in_config(config, "medium.en");
        assert!(result.contains(r#"model = "medium.en""#));
        assert!(result.contains("# Model to use"));
        assert!(result.contains("# Language setting"));
    }

    #[test]
    fn test_models_list_contains_expected_models() {
        let model_names: Vec<&str> = MODELS.iter().map(|m| m.name).collect();
        // Multilingual models
        assert!(model_names.contains(&"tiny"));
        assert!(model_names.contains(&"base"));
        assert!(model_names.contains(&"small"));
        assert!(model_names.contains(&"medium"));
        // English-only models
        assert!(model_names.contains(&"tiny.en"));
        assert!(model_names.contains(&"base.en"));
        assert!(model_names.contains(&"small.en"));
        assert!(model_names.contains(&"medium.en"));
        // Large models (multilingual only)
        assert!(model_names.contains(&"large-v3"));
        assert!(model_names.contains(&"large-v3-turbo"));
    }

    #[test]
    fn test_model_info_sizes_are_reasonable() {
        for model in MODELS {
            // All models should have positive size
            assert!(model.size_mb > 0, "Model {} has invalid size", model.name);
            // Tiny models should be smallest, large should be biggest
            if model.name.starts_with("tiny") {
                assert!(model.size_mb < 100);
            }
            if model.name == "large-v3" {
                assert!(model.size_mb > 2000);
            }
        }
    }

    #[test]
    fn test_is_valid_model() {
        // Valid multilingual models
        assert!(is_valid_model("tiny"));
        assert!(is_valid_model("base"));
        assert!(is_valid_model("small"));
        assert!(is_valid_model("medium"));
        // Valid English-only models
        assert!(is_valid_model("tiny.en"));
        assert!(is_valid_model("base.en"));
        assert!(is_valid_model("small.en"));
        assert!(is_valid_model("medium.en"));
        // Valid large models
        assert!(is_valid_model("large-v3"));
        assert!(is_valid_model("large-v3-turbo"));

        // Invalid models
        assert!(!is_valid_model("invalid"));
        assert!(!is_valid_model("large"));
        assert!(!is_valid_model(""));
        assert!(!is_valid_model("LARGE-V3")); // case sensitive
    }

    #[test]
    fn test_valid_model_names() {
        let names = valid_model_names();
        assert!(names.contains(&"tiny.en"));
        assert!(names.contains(&"large-v3-turbo"));
        assert_eq!(names.len(), MODELS.len());
    }
}
