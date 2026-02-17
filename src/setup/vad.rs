//! VAD model download and status

use super::{print_info, print_success, print_warning};
use crate::config::Config;
use crate::vad::{get_whisper_vad_model_filename, get_whisper_vad_model_url};
use std::process::Command;

/// Download the Silero VAD model
pub fn download_model() -> anyhow::Result<()> {
    let models_dir = Config::models_dir();
    let filename = get_whisper_vad_model_filename();
    let model_path = models_dir.join(filename);

    if model_path.exists() {
        print_success(&format!("VAD model already installed: {:?}", model_path));
        print_info("To re-download, delete the file and run this command again.");
        return Ok(());
    }

    std::fs::create_dir_all(&models_dir)?;

    let url = get_whisper_vad_model_url();

    println!("Downloading Silero VAD model...");
    println!("URL: {}", url);

    let status = Command::new("curl")
        .args([
            "-L",
            "--progress-bar",
            "-o",
            model_path.to_str().unwrap_or("model.bin"),
            url,
        ])
        .status();

    match status {
        Ok(exit_status) if exit_status.success() => {
            print_success(&format!("Saved to {:?}", model_path));
            println!();
            print_info("Enable in config.toml:");
            println!("  [vad]");
            println!("  enabled = true");
            println!("  backend = \"whisper\"");
            Ok(())
        }
        Ok(exit_status) => {
            let _ = std::fs::remove_file(&model_path);
            anyhow::bail!(
                "Download failed: curl exited with code {}",
                exit_status.code().unwrap_or(-1)
            )
        }
        Err(e) => {
            print_info("Please ensure curl is installed (e.g., 'sudo pacman -S curl')");
            anyhow::bail!("curl not available: {}", e)
        }
    }
}

/// Show VAD model status
pub fn show_status() {
    let models_dir = Config::models_dir();
    let filename = get_whisper_vad_model_filename();
    let model_path = models_dir.join(filename);

    println!("VAD Model Status\n");

    if model_path.exists() {
        let size = std::fs::metadata(&model_path).map(|m| m.len()).unwrap_or(0);
        print_success(&format!(
            "Silero VAD model installed: {:?} ({:.1} MB)",
            model_path,
            size as f64 / 1_048_576.0
        ));
    } else {
        print_warning("Silero VAD model not installed");
        print_info("Download with: voxtype setup vad");
        print_info("Energy VAD (no model needed) is available as an alternative.");
    }
}
