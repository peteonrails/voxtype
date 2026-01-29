//! Voice Activity Detection model download

use super::{print_failure, print_info, print_success};
use crate::config::Config;
use crate::vad;
use std::process::Command;

/// Download the VAD model for the current transcription engine
pub async fn download_vad_model(config: &Config, force: bool) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();

    // Ensure models directory exists
    std::fs::create_dir_all(&models_dir)?;

    let filename = vad::get_default_model_filename(config.engine);
    let model_path = models_dir.join(filename);
    let url = vad::get_vad_model_url(config.engine);

    println!("\nVoice Activity Detection Model Setup\n");
    println!("====================================\n");
    println!("Engine: {:?}", config.engine);
    println!("Model: {}", filename);
    println!("Path: {:?}\n", model_path);

    // Check if already downloaded
    if model_path.exists() && !force {
        print_success(&format!("VAD model already installed at {:?}", model_path));
        println!();
        print_info("To enable VAD, add to config.toml:");
        println!("  [vad]");
        println!("  enabled = true");
        println!();
        print_info("Or use CLI flag: voxtype --vad");
        println!();
        print_info("Use --force to re-download");
        return Ok(());
    }

    println!("Downloading VAD model...");
    println!("URL: {}", url);

    // Use curl for downloading
    let status = Command::new("curl")
        .args([
            "-L",             // Follow redirects
            "--progress-bar", // Show progress bar
            "-o",
            model_path.to_str().unwrap_or("vad_model"),
            url,
        ])
        .status();

    match status {
        Ok(exit_status) if exit_status.success() => {
            print_success(&format!("VAD model saved to {:?}", model_path));
            println!();

            // Show how to enable
            print_info("To enable VAD, add to config.toml:");
            println!("  [vad]");
            println!("  enabled = true");
            println!();
            print_info("Or use CLI flag: voxtype --vad");

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

/// Check if VAD model is installed for the given engine
pub fn is_vad_model_installed(engine: crate::config::TranscriptionEngine) -> bool {
    let models_dir = Config::models_dir();
    let filename = vad::get_default_model_filename(engine);
    let model_path = models_dir.join(filename);
    model_path.exists()
}
