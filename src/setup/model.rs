//! Interactive model selection and download

use super::{print_failure, print_info, print_success};
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
    ModelInfo {
        name: "tiny.en",
        size_mb: 39,
        description: "Fastest, lowest accuracy",
        english_only: true,
    },
    ModelInfo {
        name: "base.en",
        size_mb: 142,
        description: "Good balance (default)",
        english_only: true,
    },
    ModelInfo {
        name: "small.en",
        size_mb: 466,
        description: "Better accuracy",
        english_only: true,
    },
    ModelInfo {
        name: "medium.en",
        size_mb: 1500,
        description: "High accuracy",
        english_only: true,
    },
    ModelInfo {
        name: "large-v3",
        size_mb: 3100,
        description: "Best accuracy, multilingual",
        english_only: false,
    },
    ModelInfo {
        name: "large-v3-turbo",
        size_mb: 1600,
        description: "Fast + accurate, multilingual (recommended for GPU)",
        english_only: false,
    },
];

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
        print!("\nModel already installed. Re-download? [y/N]: ");
        io::stdout().flush()?;

        let mut confirm = String::new();
        io::stdin().read_line(&mut confirm)?;

        if !confirm.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
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
        if let Some(config_path) = Config::default_path() {
            if config_path.exists() {
                // Read and update config
                let content = std::fs::read_to_string(&config_path)?;
                let updated = update_model_in_config(&content, model.name);
                std::fs::write(&config_path, updated)?;
                print_success(&format!(
                    "Config updated to use '{}' model",
                    model.name
                ));
            } else {
                print_info("No config file found. Run 'voxtype setup' first.");
            }
        }
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
            "-L",              // Follow redirects
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

            println!(
                "  {} ({:.0} MB) - {}",
                model.name, size, model.description
            );
            found = true;
        }
    }

    if !found {
        println!("  No models installed.");
        println!("\n  Run 'voxtype setup model' to download a model.");
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
