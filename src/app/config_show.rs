//! `voxtype config` (no subcommand) — print the resolved config to stdout,
//! including derived bits like the output-chain probe and the meeting tree.

use std::path::PathBuf;
use voxtype::{config, setup};

/// Format the `[meeting]` config sections for display in `voxtype config`.
///
/// Extracted so the regression test `config_displays_meeting_section`
/// can assert that the section is present without spawning a process.
/// The smoke pipeline previously missed that `voxtype config` omitted
/// the entire `[meeting]` tree even when meeting mode was configured.
pub(crate) fn format_meeting_config_section(meeting: &config::MeetingConfig) -> String {
    use std::fmt::Write;
    let mut s = String::new();

    let _ = writeln!(s, "\n[meeting]");
    let _ = writeln!(s, "  enabled = {}", meeting.enabled);
    let _ = writeln!(s, "  chunk_duration_secs = {}", meeting.chunk_duration_secs);
    let _ = writeln!(s, "  storage_path = {:?}", meeting.storage_path);
    let _ = writeln!(s, "  retain_audio = {}", meeting.retain_audio);
    let _ = writeln!(s, "  max_duration_mins = {}", meeting.max_duration_mins);

    let _ = writeln!(s, "\n[meeting.audio]");
    let _ = writeln!(s, "  mic_device = {:?}", meeting.audio.mic_device);
    let _ = writeln!(s, "  loopback_device = {:?}", meeting.audio.loopback_device);
    let _ = writeln!(s, "  echo_cancel = {:?}", meeting.audio.echo_cancel);

    let _ = writeln!(s, "\n[meeting.diarization]");
    let _ = writeln!(s, "  enabled = {}", meeting.diarization.enabled);
    let _ = writeln!(s, "  backend = {:?}", meeting.diarization.backend);
    let _ = writeln!(s, "  max_speakers = {}", meeting.diarization.max_speakers);
    if let Some(ref path) = meeting.diarization.model_path {
        let _ = writeln!(s, "  model_path = {:?}", path);
    }
    let _ = writeln!(
        s,
        "  min_segment_ms = {}",
        meeting.diarization.min_segment_ms
    );

    let _ = writeln!(s, "\n[meeting.summary]");
    let _ = writeln!(s, "  backend = {:?}", meeting.summary.backend);
    let _ = writeln!(s, "  ollama_url = {:?}", meeting.summary.ollama_url);
    let _ = writeln!(s, "  ollama_model = {:?}", meeting.summary.ollama_model);
    if let Some(ref endpoint) = meeting.summary.remote_endpoint {
        let _ = writeln!(s, "  remote_endpoint = {:?}", endpoint);
    }
    if meeting.summary.remote_api_key.is_some() {
        let _ = writeln!(s, "  remote_api_key = (set)");
    }
    let _ = writeln!(s, "  timeout_secs = {}", meeting.summary.timeout_secs);

    s
}

pub(crate) async fn show_config(config: &config::Config) -> anyhow::Result<()> {
    println!("Current Configuration\n");
    println!("=====================\n");

    println!("[hotkey]");
    println!("  key = {:?}", config.hotkey.key);
    println!("  modifiers = {:?}", config.hotkey.modifiers);
    println!("  mode = {:?}", config.hotkey.mode);

    println!("\n[audio]");
    println!("  device = {:?}", config.audio.device);
    println!("  sample_rate = {}", config.audio.sample_rate);
    println!("  max_duration_secs = {}", config.audio.max_duration_secs);
    println!("  wait_for_device = {}", config.audio.wait_for_device);

    println!("\n[audio.feedback]");
    println!("  enabled = {}", config.audio.feedback.enabled);
    println!("  theme = {:?}", config.audio.feedback.theme);
    println!("  volume = {}", config.audio.feedback.volume);

    // Show current engine
    println!("\n[engine]");
    println!("  engine = {:?}", config.engine);

    println!("\n[whisper]");
    println!("  model = {:?}", config.whisper.model);
    println!("  language = {:?}", config.whisper.language);
    println!("  translate = {}", config.whisper.translate);
    if let Some(threads) = config.whisper.threads {
        println!("  threads = {}", threads);
    }
    if let Some(gpu_device) = config.whisper.gpu_device {
        println!("  gpu_device = {}", gpu_device);
    }

    // Show Parakeet status
    println!("\n[parakeet]");
    if let Some(ref parakeet_config) = config.parakeet {
        println!("  model = {:?}", parakeet_config.model);
        if let Some(ref model_type) = parakeet_config.model_type {
            println!("  model_type = {:?}", model_type);
        }
        println!(
            "  on_demand_loading = {}",
            parakeet_config.on_demand_loading
        );
    } else {
        println!("  (not configured)");
    }

    // Check for available Parakeet models
    let models_dir = config::Config::models_dir();
    let mut parakeet_models: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("parakeet") {
                    // Check if it has the required ONNX files
                    let has_encoder = path.join("encoder-model.onnx").exists();
                    let has_decoder = path.join("decoder_joint-model.onnx").exists()
                        || path.join("model.onnx").exists();
                    if has_encoder || has_decoder {
                        parakeet_models.push(name);
                    }
                }
            }
        }
    }
    if parakeet_models.is_empty() {
        println!("  available models: (none found)");
    } else {
        println!("  available models: {}", parakeet_models.join(", "));
    }

    // Show Moonshine status (experimental)
    println!("\n[moonshine] (EXPERIMENTAL)");
    if let Some(ref moonshine_config) = config.moonshine {
        println!("  model = {:?}", moonshine_config.model);
        println!("  quantized = {}", moonshine_config.quantized);
        if let Some(threads) = moonshine_config.threads {
            println!("  threads = {}", threads);
        }
        println!(
            "  on_demand_loading = {}",
            moonshine_config.on_demand_loading
        );
    } else {
        println!("  (not configured)");
    }

    // Check for available Moonshine models
    let mut moonshine_models: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("moonshine") {
                    let has_encoder = path.join("encoder_model.onnx").exists()
                        || path.join("encoder_model_quantized.onnx").exists();
                    let has_decoder = path.join("decoder_model_merged.onnx").exists()
                        || path.join("decoder_model_merged_quantized.onnx").exists();
                    if has_encoder || has_decoder {
                        moonshine_models.push(name);
                    }
                }
            }
        }
    }
    if moonshine_models.is_empty() {
        println!("  available models: (none found)");
    } else {
        println!("  available models: {}", moonshine_models.join(", "));
    }

    // Show SenseVoice status (experimental)
    println!("\n[sensevoice] (EXPERIMENTAL)");
    if let Some(ref sensevoice_config) = config.sensevoice {
        println!("  model = {:?}", sensevoice_config.model);
        println!("  language = {:?}", sensevoice_config.language);
        println!("  use_itn = {}", sensevoice_config.use_itn);
        if let Some(threads) = sensevoice_config.threads {
            println!("  threads = {}", threads);
        }
        println!(
            "  on_demand_loading = {}",
            sensevoice_config.on_demand_loading
        );
    } else {
        println!("  (not configured)");
    }

    // Check for available SenseVoice models
    let mut sensevoice_models: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("sensevoice") {
                    let has_model =
                        path.join("model.int8.onnx").exists() || path.join("model.onnx").exists();
                    let has_tokens = path.join("tokens.txt").exists();
                    if has_model && has_tokens {
                        sensevoice_models.push(name);
                    }
                }
            }
        }
    }
    if sensevoice_models.is_empty() {
        println!("  available models: (none found)");
    } else {
        println!("  available models: {}", sensevoice_models.join(", "));
    }

    println!("\n[output]");
    println!("  mode = {:?}", config.output.mode);
    println!(
        "  fallback_to_clipboard = {}",
        config.output.fallback_to_clipboard
    );
    if let Some(ref driver_order) = config.output.driver_order {
        println!(
            "  driver_order = [{}]",
            driver_order
                .iter()
                .map(|d| format!("{:?}", d))
                .collect::<Vec<_>>()
                .join(", ")
        );
    } else {
        println!("  driver_order = (default: wtype -> dotool -> ydotool -> clipboard)");
    }
    println!("  type_delay_ms = {}", config.output.type_delay_ms);
    println!("  pre_type_delay_ms = {}", config.output.pre_type_delay_ms);
    println!("  restore_clipboard = {}", config.output.restore_clipboard);
    println!(
        "  restore_clipboard_delay_ms = {}",
        config.output.restore_clipboard_delay_ms
    );
    println!(
        "  wait_for_modifier_release = {}",
        config.output.wait_for_modifier_release
    );
    println!(
        "  modifier_release_timeout_ms = {}",
        config.output.modifier_release_timeout_ms
    );

    println!("\n[output.notification]");
    println!(
        "  on_recording_start = {}",
        config.output.notification.on_recording_start
    );
    println!(
        "  on_recording_stop = {}",
        config.output.notification.on_recording_stop
    );
    println!(
        "  on_transcription = {}",
        config.output.notification.on_transcription
    );
    println!("  urgency = {:?}", config.output.notification.urgency);

    // Meeting mode is opt-in. Always show the section so users can see the
    // resolved defaults — the smoke runner previously missed this entirely
    // and meeting users had no way to verify their config was loaded.
    print!("{}", format_meeting_config_section(&config.meeting));

    println!("\n[status]");
    println!("  icon_theme = {:?}", config.status.icon_theme);
    let icons = config.status.resolve_icons();
    println!(
        "  (resolved icons: idle={:?} recording={:?} transcribing={:?} stopped={:?})",
        icons.idle, icons.recording, icons.transcribing, icons.stopped
    );

    if let Some(ref state_file) = config.state_file {
        println!("\n[integration]");
        println!("  state_file = {:?}", state_file);
        if let Some(resolved) = config.resolve_state_file() {
            println!("  (resolves to: {:?})", resolved);
        }
    }

    // Show output chain status
    let output_status = setup::detect_output_chain().await;
    setup::print_output_chain_status(&output_status);

    println!("\n---");
    match config::Config::resolve_existing_path() {
        Some(path) => println!("Config file: {:?} (loaded)", path),
        None => println!(
            "Config file: {:?} (not found, using defaults; system fallback {:?} also missing)",
            config::Config::default_path().unwrap_or_else(|| PathBuf::from("(unknown)")),
            config::Config::system_path()
        ),
    }
    println!("Models dir: {:?}", config::Config::models_dir());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxtype::config;

    /// Regression: `voxtype config` once omitted the entire `[meeting]`
    /// tree, leaving meeting-mode users with no way to verify their config
    /// loaded correctly. The helper must emit all four sub-sections so the
    /// smoke pipeline can grep for them.
    #[test]
    fn config_displays_meeting_section() {
        let meeting = config::MeetingConfig::default();
        let rendered = format_meeting_config_section(&meeting);

        for header in [
            "[meeting]",
            "[meeting.audio]",
            "[meeting.diarization]",
            "[meeting.summary]",
        ] {
            assert!(
                rendered.contains(header),
                "format_meeting_config_section must emit {} header. Got:\n{}",
                header,
                rendered
            );
        }

        // A couple of representative fields, so renaming a struct member
        // doesn't silently drop output without breaking the test.
        assert!(rendered.contains("enabled = false"));
        assert!(rendered.contains("backend = "));
        assert!(rendered.contains("ollama_url = "));
    }
}
