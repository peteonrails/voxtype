//! VAD model download and status

use super::manifest::MirroredFile;
use super::model;
use super::{print_info, print_success, print_warning};
use crate::config::Config;
use crate::vad::get_whisper_vad_model_filename;

/// Where the Silero VAD model is mirrored. It installs under the shorter
/// name `ggml-silero-vad.bin` that the VAD loader has always looked for.
fn vad_mirror_file() -> MirroredFile {
    MirroredFile {
        engine: "vad",
        model: "silero-v6.2.0".to_string(),
        file: "ggml-silero-v6.2.0.bin".to_string(),
        upstream_repo: "ggml-org/whisper-vad",
    }
}

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

    println!("Downloading Silero VAD model...");

    // Verified against the mirror's manifest before it takes its final name:
    // this function treats "the file exists" as "installed" and returns
    // early, so a partial file left at the final path would never be retried.
    model::download_mirrored_file(&vad_mirror_file(), &model_path, "silero-vad")?;

    print_success(&format!("Saved to {:?}", model_path));
    println!();
    print_info("Enable in config.toml:");
    println!("  [vad]");
    println!("  enabled = true");
    println!("  backend = \"whisper\"");
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vad_model_downloads_from_the_mirror_under_its_historical_name() {
        assert_eq!(
            vad_mirror_file().url(),
            "https://models.voxtype.io/vad/silero-v6.2.0/ggml-silero-v6.2.0.bin"
        );
        assert_eq!(get_whisper_vad_model_filename(), "ggml-silero-vad.bin");
    }
}
