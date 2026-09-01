//! OpenVINO Whisper engine configuration.

use serde::{Deserialize, Serialize};

use super::super::{default_on_demand_loading, default_true};

/// OpenVINO Whisper speech-to-text configuration (Intel NPU/CPU/GPU via OpenVINO GenAI).
/// Requires: cargo build --features openvino-whisper
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenVinoConfig {
    /// Model name or path to a directory containing OpenVINO IR model files.
    /// Quantized names include "base.en-int8", "small.en-fp16", and "tiny-int4".
    /// Short names such as "base.en" use `quantized` to select int8 or fp16.
    pub model: String,

    /// OpenVINO inference device: "NPU", "CPU", "GPU", or "AUTO".
    #[serde(default = "default_openvino_device")]
    pub device: String,

    /// Prefer int8 quantized model variants.
    #[serde(default = "default_true")]
    pub quantized: bool,

    /// Number of CPU threads (ignored for NPU/GPU).
    #[serde(default)]
    pub threads: Option<usize>,

    /// Whisper language code.
    #[serde(default = "default_openvino_language")]
    pub language: String,

    /// Translate non-English speech to English.
    #[serde(default)]
    pub translate: bool,

    /// Load the model when recording starts instead of at daemon startup.
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,

    /// OpenVINO GenAI installation directory containing the shared library.
    /// Also settable through VOXTYPE_OPENVINO_DIR.
    #[serde(default)]
    pub openvino_dir: Option<String>,

    // --- Sliding-window streaming settings ---
    /// Enable live streaming transcription via the shared sliding-window engine.
    #[serde(default)]
    pub streaming: bool,

    /// DEPRECATED: these `streaming_*` tuning fields used to be OpenVINO's
    /// own copy of the sliding-window knobs (identical to `[whisper]`'s).
    /// They're kept only as a fallback for old configs — set them via the
    /// shared `[streaming]` section instead. See `StreamingConfig::resolve`.
    #[serde(default = "default_streaming_interval_secs")]
    pub streaming_interval_secs: f32,

    /// DEPRECATED, see `streaming_interval_secs`.
    #[serde(default = "default_streaming_max_buffer_secs")]
    pub streaming_max_buffer_secs: f32,

    /// DEPRECATED, see `streaming_interval_secs`.
    #[serde(default = "default_streaming_min_speech_rms")]
    pub streaming_min_speech_rms: f32,

    /// DEPRECATED, see `streaming_interval_secs`.
    #[serde(default = "default_streaming_min_audio_secs")]
    pub streaming_min_audio_secs: f32,

    /// DEPRECATED, see `streaming_interval_secs`.
    #[serde(default = "default_streaming_partial_min_words")]
    pub streaming_partial_min_words: usize,

    /// DEPRECATED, see `streaming_interval_secs`.
    #[serde(default = "default_streaming_type_partials")]
    pub streaming_type_partials: bool,

    /// DEPRECATED, see `streaming_interval_secs`.
    #[serde(default)]
    pub streaming_revision_mode: bool,
}

fn default_streaming_interval_secs() -> f32 {
    0.8
}

fn default_streaming_max_buffer_secs() -> f32 {
    29.0
}

fn default_streaming_min_speech_rms() -> f32 {
    0.005
}

fn default_streaming_min_audio_secs() -> f32 {
    1.0
}

fn default_streaming_partial_min_words() -> usize {
    1
}

fn default_streaming_type_partials() -> bool {
    true
}

fn default_openvino_device() -> String {
    "NPU".to_string()
}

fn default_openvino_language() -> String {
    "en".to_string()
}

impl Default for OpenVinoConfig {
    fn default() -> Self {
        Self {
            model: "base.en".to_string(),
            device: default_openvino_device(),
            quantized: true,
            threads: None,
            language: default_openvino_language(),
            translate: false,
            on_demand_loading: false,
            openvino_dir: None,
            streaming: false,
            streaming_interval_secs: default_streaming_interval_secs(),
            streaming_max_buffer_secs: default_streaming_max_buffer_secs(),
            streaming_min_speech_rms: default_streaming_min_speech_rms(),
            streaming_min_audio_secs: default_streaming_min_audio_secs(),
            streaming_partial_min_words: default_streaming_partial_min_words(),
            streaming_type_partials: default_streaming_type_partials(),
            streaming_revision_mode: false,
        }
    }
}

impl OpenVinoConfig {
    /// Runtime and driver guidance tailored to the configured OpenVINO device.
    ///
    /// OpenVINO is runtime-loaded, so binaries compiled with the feature remain
    /// usable for other engines when these packages are not installed.
    pub fn installation_guidance(&self) -> String {
        installation_guidance(&self.device)
    }
}

/// Return actionable Linux setup guidance for an OpenVINO device selection.
pub fn installation_guidance(device: &str) -> String {
    let device = device.trim().to_ascii_uppercase();
    let packages = match device.as_str() {
        "NPU" => "openvino openvino-intel-npu-plugin intel-npu-driver",
        "GPU" => "openvino openvino-intel-gpu-plugin level-zero-loader intel-compute-runtime",
        "CPU" => "openvino",
        "AUTO" => {
            "openvino plus the plugin/driver for each device AUTO may use:\n  \
             NPU: openvino-intel-npu-plugin intel-npu-driver\n  \
             GPU: openvino-intel-gpu-plugin level-zero-loader intel-compute-runtime"
        }
        _ => "openvino and the OpenVINO plugin/driver for the selected device",
    };

    let device_check = match device.as_str() {
        "NPU" => "After installing the NPU driver, reboot and verify /dev/accel/accel* exists.",
        "GPU" => "Verify the Intel GPU is visible with: ls /dev/dri/renderD*",
        "CPU" => "No device-specific driver is required for CPU inference.",
        "AUTO" => "Install the plugin/driver for every accelerator AUTO should be allowed to use.",
        _ => "Valid device values are NPU, GPU, CPU, and AUTO.",
    };

    format!(
        "OpenVINO requirements for device = \"{}\":\n  \
         Arch Linux packages: sudo pacman -S {}\n  \
         Also install Intel's version-matched OpenVINO GenAI C/C++ SDK archive; \
         the Rust backend requires libopenvino_genai_c.so (the pip wheel and \
         openvino-genai-bin package do not provide this C API library).\n  \
         Set openvino_dir in [openvino] to the extracted SDK root, or add its \
         runtime/lib/intel64 directory to LD_LIBRARY_PATH.\n  {}",
        device, packages, device_check
    )
}

#[cfg(test)]
mod tests {
    use super::installation_guidance;

    #[test]
    fn guidance_is_device_specific() {
        let npu = installation_guidance("npu");
        assert!(npu.contains("openvino-intel-npu-plugin"));
        assert!(npu.contains("intel-npu-driver"));
        assert!(!npu.contains("intel-compute-runtime"));

        let gpu = installation_guidance("GPU");
        assert!(gpu.contains("openvino-intel-gpu-plugin"));
        assert!(gpu.contains("intel-compute-runtime"));
        assert!(!gpu.contains("intel-npu-driver"));

        let cpu = installation_guidance("CPU");
        assert!(cpu.contains("No device-specific driver"));
        assert!(!cpu.contains("openvino-intel-npu-plugin"));
        assert!(!cpu.contains("openvino-intel-gpu-plugin"));
    }

    #[test]
    fn guidance_calls_out_required_c_api_library() {
        let guidance = installation_guidance("AUTO");
        assert!(guidance.contains("libopenvino_genai_c.so"));
        assert!(guidance.contains("pip wheel"));
    }
}
