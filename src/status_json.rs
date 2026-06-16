//! Waybar / external-consumer status-JSON contract.
//!
//! The shape emitted by `format_state_json` is the API voxtype exposes to
//! every downstream status consumer — Waybar via `voxtype status --follow`,
//! Quickshell, the planned audio bridge, a hypothetical future
//! StatusNotifierItem tray, or anything else that wants to render daemon
//! state without re-implementing the polling logic. It lives in the
//! library (rather than the binary-only `src/app/`) so any caller can
//! emit the same shape.
//!
//! ## Contract
//!
//! - Key order: `text, alt, class, tooltip` (then `model, device, backend`
//!   when extended).
//! - Whitespace: a single space after each `:` between key and value.
//! - The tooltip is a JSON string with `\n` (the two-byte escape) between
//!   lines, not a real `0x0a` newline — Waybar renders these client-side.
//! - String values are escaped via `serde_json::to_string`, so `"` and `\`
//!   in device or model names cannot break consumer parsers.
//!
//! `format_state_json_pins_byte_exact_output` and
//! `format_state_json_escapes_quotes_and_backslashes` lock this contract.
//! Changing the shape is a breaking change for every consumer.

use crate::config;
use crate::setup;

/// Extended status info for JSON output. Three fields a status consumer
/// typically wants in tooltips alongside the base state: which model,
/// which audio device, and which compute backend.
#[derive(Debug, Clone)]
pub struct ExtendedStatusInfo {
    pub model: String,
    pub device: String,
    pub backend: String,
}

impl ExtendedStatusInfo {
    /// Build an `ExtendedStatusInfo` from the loaded config. Resolves the
    /// backend through the inventory machinery (wrapper-script aware via
    /// `setup::binary::active_variant`) so it reports correctly whether
    /// `/usr/bin/voxtype` is a plain symlink or the exec-wrapper used by
    /// GPU/ONNX variants. The legacy `setup::gpu::detect_current_backend()`
    /// path was Whisper-focused: it would treat any wrapper script as
    /// `Backend::Native`, which is why waybar previously showed
    /// "CPU (native)" while MIGraphX or CUDA was actually doing the work.
    pub fn from_config(config: &config::Config) -> Self {
        let inv = setup::binary::inventory();
        let backend = if let Some(v) = inv.active_variant {
            backend_display_for_variant(v).to_string()
        } else if let Some(b) = setup::gpu::detect_current_backend() {
            match b {
                setup::gpu::Backend::Cpu => "CPU (legacy)",
                setup::gpu::Backend::Native => "CPU (native)",
                setup::gpu::Backend::Avx2 => "CPU (AVX2)",
                setup::gpu::Backend::Avx512 => "CPU (AVX-512)",
                setup::gpu::Backend::Vulkan => "GPU (Vulkan)",
            }
            .to_string()
        } else if let Some(pb) = setup::parakeet::detect_current_parakeet_backend() {
            pb.display_name().to_string()
        } else {
            "unknown".to_string()
        };

        Self {
            model: config.model_name().to_string(),
            device: config.audio.device.clone(),
            backend,
        }
    }
}

/// User-facing backend label for an active variant. Combines engine family
/// (Whisper vs ONNX) with the EP/acceleration so both pieces of info land in
/// waybar tooltips and `voxtype info` output. Whisper variants get a "CPU"/"GPU"
/// prefix that matches the legacy display strings; ONNX variants spell out the
/// EP name explicitly so users can tell a CUDA-12 install apart from CUDA-13.
pub fn backend_display_for_variant(v: setup::binary::Variant) -> &'static str {
    use setup::binary::Variant;
    match v {
        Variant::WhisperAvx2 => "CPU (AVX2)",
        Variant::WhisperAvx512 => "CPU (AVX-512)",
        Variant::WhisperVulkan => "GPU (Vulkan)",
        Variant::WhisperNative => "CPU (native)",
        Variant::OnnxAvx2 => "ONNX CPU (AVX2)",
        Variant::OnnxAvx512 => "ONNX CPU (AVX-512)",
        Variant::OnnxCuda12 => "ONNX GPU (CUDA 12)",
        Variant::OnnxCuda13 => "ONNX GPU (CUDA 13)",
        Variant::OnnxCuda => "ONNX GPU (CUDA)",
        Variant::OnnxMigraphx => "ONNX GPU (MIGraphX)",
        Variant::OnnxNative => "ONNX CPU (native)",
    }
}

/// Format state as JSON for Waybar consumption.
///
/// The `alt` field enables Waybar's format-icons feature for custom icon
/// mapping. The output format (key order, space-after-colon, embedded `\n`
/// in the tooltip) is part of the contract with status consumers; the
/// `format_state_json_pins_byte_exact_output` test locks it.
///
/// Values are escaped via `serde_json::to_string` so a device name or model
/// containing `"` or `\` can't produce malformed JSON. The outer template
/// stays hand-rolled to preserve the existing whitespace shape that Waybar's
/// example configs and several user dotfiles match against.
pub fn format_state_json(
    state: &str,
    icons: &config::ResolvedIcons,
    extended: Option<&ExtendedStatusInfo>,
) -> String {
    let (text, base_tooltip) = match state {
        "recording" => (&icons.recording, "Recording..."),
        "streaming" => (&icons.streaming, "Streaming live..."),
        "transcribing" => (&icons.transcribing, "Transcribing..."),
        "outputting" => (&icons.transcribing, "Outputting..."),
        "idle" => (&icons.idle, "Voxtype ready - hold hotkey to record"),
        "stopped" => (&icons.stopped, "Voxtype not running"),
        _ => (&icons.idle, "Unknown state"),
    };

    // alt = state name (for Waybar format-icons mapping)
    // class = state name (for CSS styling)
    let alt = state;
    let class = state;

    match extended {
        Some(info) => {
            // Use real newlines in the tooltip — serde_json encodes each as
            // the two-byte `\n` escape, which is what waybar expects.
            let tooltip = format!(
                "{}\nModel: {}\nDevice: {}\nBackend: {}",
                base_tooltip, info.model, info.device, info.backend
            );
            format!(
                r#"{{"text": {}, "alt": {}, "class": {}, "tooltip": {}, "model": {}, "device": {}, "backend": {}}}"#,
                json_str(text),
                json_str(alt),
                json_str(class),
                json_str(&tooltip),
                json_str(&info.model),
                json_str(&info.device),
                json_str(&info.backend),
            )
        }
        None => format!(
            r#"{{"text": {}, "alt": {}, "class": {}, "tooltip": {}}}"#,
            json_str(text),
            json_str(alt),
            json_str(class),
            json_str(base_tooltip),
        ),
    }
}

/// JSON-encode a single string value, returning it with the surrounding
/// double-quotes (e.g. `foo` → `"foo"`, `a"b` → `"a\"b"`). Lets the outer
/// template in `format_state_json` keep its hand-rolled whitespace shape
/// while still getting correct escaping for free.
fn json_str(s: &str) -> String {
    serde_json::to_string(s).expect("serde_json never fails on &str")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: after `voxtype record cancel` the daemon writes "idle"
    /// to the state file. `format_state_json` must render "idle" as the
    /// idle icon, NOT downgrade or alias it to "stopped". "stopped" is
    /// reserved for "daemon process not running" (state file missing).
    #[test]
    fn record_cancel_leaves_idle_not_stopped() {
        let icons = config::StatusConfig::default().resolve_icons();
        let json = format_state_json("idle", &icons, None);
        assert!(
            json.contains("\"alt\": \"idle\""),
            "format_state_json('idle') must keep alt=idle so Waybar shows \
             the idle icon after `record cancel`. Got: {}",
            json
        );
        assert!(
            !json.contains("\"alt\": \"stopped\""),
            "format_state_json('idle') must not alias to 'stopped'. Got: {}",
            json
        );

        // And stopped should still map distinctly so we don't accidentally
        // collapse the two states in the other direction.
        let stopped_json = format_state_json("stopped", &icons, None);
        assert!(stopped_json.contains("\"alt\": \"stopped\""));
    }

    /// Pin the exact byte output of `format_state_json` for every state,
    /// with and without extended info. Status consumers parse this JSON via
    /// `jq` / `format-icons`; key order, the literal `\n` escape in tooltips
    /// (NOT a real newline), and the space-after-colon style are part of the
    /// contract. If you switch the implementation (e.g. to serde_json), this
    /// test must still pass byte-for-byte.
    #[test]
    fn format_state_json_pins_byte_exact_output() {
        // Deterministic icons so the test doesn't depend on which theme is
        // currently the default. Use ASCII placeholders to keep the literal
        // strings readable.
        let icons = config::ResolvedIcons {
            idle: "I".to_string(),
            recording: "R".to_string(),
            streaming: "S".to_string(),
            transcribing: "T".to_string(),
            stopped: "X".to_string(),
        };

        // --- Without extended info ---
        assert_eq!(
            format_state_json("recording", &icons, None),
            r#"{"text": "R", "alt": "recording", "class": "recording", "tooltip": "Recording..."}"#,
        );
        assert_eq!(
            format_state_json("streaming", &icons, None),
            r#"{"text": "S", "alt": "streaming", "class": "streaming", "tooltip": "Streaming live..."}"#,
        );
        assert_eq!(
            format_state_json("transcribing", &icons, None),
            r#"{"text": "T", "alt": "transcribing", "class": "transcribing", "tooltip": "Transcribing..."}"#,
        );
        assert_eq!(
            format_state_json("outputting", &icons, None),
            r#"{"text": "T", "alt": "outputting", "class": "outputting", "tooltip": "Outputting..."}"#,
        );
        assert_eq!(
            format_state_json("idle", &icons, None),
            r#"{"text": "I", "alt": "idle", "class": "idle", "tooltip": "Voxtype ready - hold hotkey to record"}"#,
        );
        assert_eq!(
            format_state_json("stopped", &icons, None),
            r#"{"text": "X", "alt": "stopped", "class": "stopped", "tooltip": "Voxtype not running"}"#,
        );
        // Unknown state falls back to the idle icon but keeps the literal
        // alt/class for the consumer to inspect.
        assert_eq!(
            format_state_json("bogus", &icons, None),
            r#"{"text": "I", "alt": "bogus", "class": "bogus", "tooltip": "Unknown state"}"#,
        );

        // --- With extended info ---
        // The tooltip embeds literal `\n` characters (the two-byte escape,
        // not 0x0a). Waybar renders these as newlines client-side.
        let ext = ExtendedStatusInfo {
            model: "base.en".to_string(),
            device: "default".to_string(),
            backend: "CPU (AVX2)".to_string(),
        };
        assert_eq!(
            format_state_json("recording", &icons, Some(&ext)),
            r#"{"text": "R", "alt": "recording", "class": "recording", "tooltip": "Recording...\nModel: base.en\nDevice: default\nBackend: CPU (AVX2)", "model": "base.en", "device": "default", "backend": "CPU (AVX2)"}"#,
        );
        assert_eq!(
            format_state_json("idle", &icons, Some(&ext)),
            r#"{"text": "I", "alt": "idle", "class": "idle", "tooltip": "Voxtype ready - hold hotkey to record\nModel: base.en\nDevice: default\nBackend: CPU (AVX2)", "model": "base.en", "device": "default", "backend": "CPU (AVX2)"}"#,
        );
    }

    /// The whole point of the serde_json switch in `format_state_json` is
    /// that a device name or model string containing `"` or `\` can't
    /// break the JSON output. Pin the escaping: round-trip the output
    /// through `serde_json::Value` and assert the raw payload survives.
    #[test]
    fn format_state_json_escapes_quotes_and_backslashes() {
        let icons = config::ResolvedIcons {
            idle: "I".to_string(),
            recording: "R".to_string(),
            streaming: "S".to_string(),
            transcribing: "T".to_string(),
            stopped: "X".to_string(),
        };
        let ext = ExtendedStatusInfo {
            model: r#"large-v3-"turbo""#.to_string(),
            device: r#"PulseAudio "Main" \ Loopback"#.to_string(),
            backend: r#"GPU \\ CUDA"#.to_string(),
        };

        let json = format_state_json("recording", &icons, Some(&ext));

        // Must be valid JSON.
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("format_state_json must emit valid JSON");

        // Each round-tripped value must equal the original input byte-for-byte.
        assert_eq!(parsed["model"], r#"large-v3-"turbo""#);
        assert_eq!(parsed["device"], r#"PulseAudio "Main" \ Loopback"#);
        assert_eq!(parsed["backend"], r#"GPU \\ CUDA"#);

        // And the tooltip — built by splicing newlines into the same
        // strings — must still parse as one well-formed JSON string.
        let tooltip = parsed["tooltip"]
            .as_str()
            .expect("tooltip must be a JSON string");
        assert!(tooltip.contains(r#"large-v3-"turbo""#));
        assert!(tooltip.contains(r#"PulseAudio "Main" \ Loopback"#));
    }
}
