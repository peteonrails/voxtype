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
    /// Build an `ExtendedStatusInfo` from the loaded config.
    ///
    /// The backend label describes the *running daemon* when there is one,
    /// resolved through `/proc/<pid>/exe` (`setup::binary::running_variant`).
    /// The package inventory (`active_variant`) only says what the next
    /// process would run: a daemon started before a variant switch, or from
    /// a systemd `ExecStart=` override pointing at a private build, can be
    /// executing something else entirely (#563). When the daemon's binary is
    /// not a packaged variant and not this CLI's own executable, the honest
    /// answer is "custom", not the package-selected backend.
    ///
    /// With no daemon alive, the label falls back to describing the install:
    /// the inventory machinery (wrapper-script aware, so GPU/ONNX exec
    /// wrappers classify correctly), then the legacy Whisper-focused
    /// detection, then Parakeet.
    pub fn from_config(config: &config::Config) -> Self {
        let inv = setup::binary::inventory();
        let daemon_exe = inv.daemon_pid.and_then(setup::binary::running_binary_path);
        let backend =
            daemon_backend_label(inv.running_variant, daemon_exe.as_deref(), &inv.binary_path)
                .unwrap_or_else(|| install_backend_label(&inv));

        Self {
            model: config.model_name().to_string(),
            device: config.audio.device.clone(),
            backend,
        }
    }
}

/// Backend label settled by the live daemon, or `None` when the caller
/// should describe the install instead.
///
/// `None` covers three cases that all mean "the daemon's executable does not
/// contradict the install": no daemon alive (`daemon_exe` is `None`),
/// `/proc/<pid>/exe` unreadable (same), or the daemon is running this CLI's
/// own binary (a source install, where the install-describing fallback chain
/// examines the very binary that is running).
fn daemon_backend_label(
    running_variant: Option<setup::binary::Variant>,
    daemon_exe: Option<&std::path::Path>,
    cli_exe: &std::path::Path,
) -> Option<String> {
    if let Some(v) = running_variant {
        return Some(backend_display_for_variant(v).to_string());
    }
    match daemon_exe {
        Some(exe) if exe != cli_exe => Some("custom".to_string()),
        _ => None,
    }
}

/// Backend label describing the install rather than a live process: what a
/// daemon started now would run.
fn install_backend_label(inv: &setup::binary::Inventory) -> String {
    if let Some(v) = inv.active_variant {
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
        Variant::WhisperBaseline => "CPU (baseline)",
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
    use std::path::Path;

    /// Regression for #563: the daemon runs a packaged variant, so its label
    /// wins no matter where /usr/bin/voxtype points. This covers both the
    /// systemd `ExecStart=` override to a different installed variant and the
    /// daemon-started-before-a-variant-switch case.
    #[test]
    fn daemon_variant_settles_the_backend_label() {
        let label = daemon_backend_label(
            Some(setup::binary::Variant::WhisperAvx2),
            Some(Path::new("/usr/lib/voxtype/voxtype-avx2")),
            Path::new("/usr/lib/voxtype/cuda-13/voxtype-onnx-cuda-13"),
        );
        assert_eq!(label.as_deref(), Some("CPU (AVX2)"));
    }

    /// Regression for #563: a daemon executing a binary that is neither a
    /// packaged variant nor this CLI must report "custom", never the
    /// package-selected backend.
    #[test]
    fn foreign_daemon_binary_reports_custom() {
        let label = daemon_backend_label(
            None,
            Some(Path::new(
                "/home/user/.local/opt/voxtype-custom/libexec/voxtype",
            )),
            Path::new("/usr/lib/voxtype/cuda-13/voxtype-onnx-cuda-13"),
        );
        assert_eq!(label.as_deref(), Some("custom"));
    }

    /// A source install typically runs the daemon from the same binary as
    /// the CLI. The install-describing fallback chain examines that binary,
    /// so it must stay in charge — no "custom" downgrade.
    #[test]
    fn daemon_running_the_cli_binary_defers_to_install_detection() {
        let exe = Path::new("/home/user/voxtype/target/release/voxtype");
        assert_eq!(daemon_backend_label(None, Some(exe), exe), None);
    }

    /// No daemon alive (or /proc/<pid>/exe unreadable): fall back to
    /// describing the install.
    #[test]
    fn no_daemon_defers_to_install_detection() {
        let cli = Path::new("/usr/lib/voxtype/voxtype-avx2");
        assert_eq!(daemon_backend_label(None, None, cli), None);
    }

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
