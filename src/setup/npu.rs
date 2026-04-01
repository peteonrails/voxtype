//! NPU backend management for voxtype
//!
//! Manages Intel NPU acceleration via OpenVINO. Unlike GPU (which switches binaries
//! via symlinks), NPU is config-driven: it sets `engine = "openvino"` in config.toml
//! and uses the same binary compiled with the `openvino-whisper` feature.

use crate::config::Config;
use std::path::Path;

const DEFAULT_OPENVINO_MODEL: &str = "base.en-int8";

/// Check if NPU hardware is present (/dev/accel/accel* devices)
fn detect_npu_hardware() -> bool {
    Path::new("/dev/accel").is_dir()
        && std::fs::read_dir("/dev/accel")
            .map(|entries| {
                entries.filter_map(|e| e.ok()).any(|e| {
                    e.file_name()
                        .to_str()
                        .map(|n| n.starts_with("accel"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
}

/// Check if the intel-npu-driver kernel module is loaded
fn check_npu_driver() -> bool {
    Path::new("/sys/module/intel_vpu").exists()
}

/// Check if an OpenVINO model is already downloaded
fn has_openvino_model() -> bool {
    let models_dir = Config::models_dir();
    if let Some(dir_name) = super::model::openvino_dir_name(DEFAULT_OPENVINO_MODEL) {
        let model_path = models_dir.join(dir_name);
        super::model::validate_openvino_model(&model_path).is_ok()
    } else {
        false
    }
}

/// Update the engine field in config.toml to a specific value, preserving everything else
fn update_engine_in_config(content: &str, engine: &str) -> String {
    let mut result = String::new();
    let mut engine_updated = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("engine") && trimmed.contains('=') && !trimmed.starts_with('[') {
            result.push_str(&format!("engine = \"{}\"\n", engine));
            engine_updated = true;
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    // If no engine line existed, don't add one -- set_openvino_config handles that for enable
    if !engine_updated && engine == "whisper" {
        // Nothing to do: there was no engine line, and the default is already whisper
    }

    // Preserve trailing newline behavior
    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Show NPU hardware and configuration status
pub fn show_status() {
    println!("NPU Status\n");

    // Hardware detection
    println!("Hardware:");
    if detect_npu_hardware() {
        super::print_success("NPU device detected (/dev/accel/)");
    } else {
        super::print_failure("No NPU device found (/dev/accel/)");
    }

    if check_npu_driver() {
        super::print_success("intel_vpu kernel module loaded");
    } else {
        super::print_failure("intel_vpu kernel module not loaded");
        super::print_info("Install intel-npu-driver and reboot");
    }

    // Feature check
    println!("\nBuild:");
    if cfg!(feature = "openvino-whisper") {
        super::print_success("openvino-whisper feature compiled in");
    } else {
        super::print_failure("openvino-whisper feature not compiled");
        super::print_info(
            "Install voxtype-openvino package or rebuild with --features openvino-whisper",
        );
    }

    // Config check
    println!("\nConfiguration:");
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                let engine_is_openvino = content.lines().any(|line| {
                    let trimmed = line.trim();
                    trimmed.starts_with("engine")
                        && trimmed.contains("openvino")
                        && !trimmed.starts_with('#')
                        && !trimmed.starts_with('[')
                });
                if engine_is_openvino {
                    super::print_success("Engine set to OpenVINO");
                } else {
                    super::print_info("Engine is not set to OpenVINO");
                }
            }
        }
    }

    // Model check
    if has_openvino_model() {
        super::print_success(&format!(
            "Default OpenVINO model ({}) installed",
            DEFAULT_OPENVINO_MODEL
        ));
    } else {
        super::print_info(&format!(
            "Default OpenVINO model ({}) not installed",
            DEFAULT_OPENVINO_MODEL
        ));
    }
}

/// Enable NPU acceleration
pub fn enable() -> anyhow::Result<()> {
    #[cfg(not(feature = "openvino-whisper"))]
    {
        anyhow::bail!(
            "NPU support requires the openvino-whisper feature.\n\
             Install the voxtype-openvino package, or rebuild with:\n  \
             cargo build --features openvino-whisper"
        );
    }

    #[cfg(feature = "openvino-whisper")]
    {
        println!("Enabling NPU acceleration (OpenVINO)...\n");

        // Check hardware (warn, don't fail)
        if detect_npu_hardware() {
            super::print_success("NPU device detected");
        } else {
            super::print_warning(
                "No NPU device found at /dev/accel/. OpenVINO will fall back to CPU.\n       \
                 If you have an Intel NPU, ensure intel-npu-driver is installed and reboot.",
            );
        }

        if !check_npu_driver() {
            super::print_warning(
                "intel_vpu kernel module not loaded.\n       \
                 Install intel-npu-driver and reboot to use NPU hardware.",
            );
        }

        // Download default model if needed
        if !has_openvino_model() {
            println!();
            super::model::download_openvino_model(DEFAULT_OPENVINO_MODEL)?;
        } else {
            super::print_success(&format!(
                "OpenVINO model '{}' already installed",
                DEFAULT_OPENVINO_MODEL
            ));
        }

        // Update config to use OpenVINO engine
        super::model::set_openvino_config(DEFAULT_OPENVINO_MODEL)?;
        super::print_success("Config updated: engine = \"openvino\"");

        println!();
        println!("NPU acceleration enabled (OpenVINO engine).");
        println!();
        println!("Restart voxtype to apply:");
        println!("  systemctl --user restart voxtype");

        Ok(())
    }
}

/// Disable NPU acceleration (revert to Whisper engine)
pub fn disable() -> anyhow::Result<()> {
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let updated = update_engine_in_config(&content, "whisper");
            std::fs::write(&config_path, updated)?;
            super::print_success("Config updated: engine = \"whisper\"");
        } else {
            super::print_info("No config file found, nothing to disable");
        }
    } else {
        anyhow::bail!("Could not determine config path");
    }

    println!();
    println!("NPU acceleration disabled (reverted to Whisper engine).");
    println!("The [openvino] config section has been preserved for easy re-enable.");
    println!();
    println!("Restart voxtype to apply:");
    println!("  systemctl --user restart voxtype");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_engine_to_whisper() {
        let config = r#"engine = "openvino"

[whisper]
model = "base.en"

[openvino]
model = "base.en-int8"
"#;
        let result = update_engine_in_config(config, "whisper");
        assert!(result.contains("engine = \"whisper\""));
        assert!(!result.contains("engine = \"openvino\""));
        // openvino section preserved
        assert!(result.contains("[openvino]"));
        assert!(result.contains("model = \"base.en-int8\""));
    }

    #[test]
    fn test_update_engine_to_openvino() {
        let config = r#"engine = "whisper"

[whisper]
model = "base.en"
"#;
        let result = update_engine_in_config(config, "openvino");
        assert!(result.contains("engine = \"openvino\""));
        assert!(!result.contains("engine = \"whisper\""));
    }

    #[test]
    fn test_update_engine_no_engine_line() {
        let config = r#"[whisper]
model = "base.en"
"#;
        let result = update_engine_in_config(config, "whisper");
        // No engine line existed, and we're setting to whisper (the default) -- no change
        assert!(!result.contains("engine ="));
        assert!(result.contains("[whisper]"));
    }

    #[test]
    fn test_update_engine_preserves_comments() {
        let config = r#"# Main config
engine = "openvino"

# Whisper settings
[whisper]
model = "base.en"
"#;
        let result = update_engine_in_config(config, "whisper");
        assert!(result.contains("# Main config"));
        assert!(result.contains("# Whisper settings"));
        assert!(result.contains("engine = \"whisper\""));
    }

    #[test]
    fn test_detect_npu_hardware_returns_false_without_device() {
        // In test/CI environments, /dev/accel typically doesn't exist
        // This just verifies the function doesn't panic
        let _result = detect_npu_hardware();
    }

    #[test]
    fn test_check_npu_driver_returns_false_without_module() {
        // In test/CI environments, intel_vpu module typically isn't loaded
        let _result = check_npu_driver();
    }
}
