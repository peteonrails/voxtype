//! `voxtype status` — read the daemon's state file, optionally render as
//! Waybar-flavoured JSON, optionally follow with inotify.

use voxtype::{config, daemon_status::is_daemon_running, setup};

/// Extended status info for JSON output
pub(crate) struct ExtendedStatusInfo {
    pub(crate) model: String,
    pub(crate) device: String,
    pub(crate) backend: String,
}

impl ExtendedStatusInfo {
    pub(crate) fn from_config(config: &config::Config) -> Self {
        // Resolve the actual backend through the inventory machinery — this is
        // wrapper-script aware (see setup::binary::active_variant) so it
        // reports correctly whether /usr/bin/voxtype is a plain symlink or the
        // exec-wrapper used by GPU/ONNX variants. The legacy
        // setup::gpu::detect_current_backend() path was Whisper-focused: it
        // would treat any wrapper script as Backend::Native, which is why
        // waybar previously showed "CPU (native)" while MIGraphX or CUDA was
        // actually doing the work.
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
fn backend_display_for_variant(v: setup::binary::Variant) -> &'static str {
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

/// Run the status command - show current daemon state
pub(crate) async fn run_status(
    config: &config::Config,
    follow: bool,
    format: &str,
    extended: bool,
    icon_theme_override: Option<String>,
) -> anyhow::Result<()> {
    let state_file = config.resolve_state_file();

    if state_file.is_none() {
        eprintln!("Error: state_file is not configured.");
        eprintln!();
        eprintln!("To enable status monitoring, add to your config.toml:");
        eprintln!();
        eprintln!("  state_file = \"auto\"");
        eprintln!();
        eprintln!("This enables external integrations like Waybar to monitor voxtype state.");
        std::process::exit(1);
    }

    let state_path = state_file.unwrap();
    let ext_info = if extended {
        Some(ExtendedStatusInfo::from_config(config))
    } else {
        None
    };

    // Use CLI override if provided, otherwise use config
    let icons = if let Some(ref theme) = icon_theme_override {
        let mut status_config = config.status.clone();
        status_config.icon_theme = theme.clone();
        status_config.resolve_icons()
    } else {
        config.status.resolve_icons()
    };

    if !follow {
        // One-shot: just read and print current state
        // First check if daemon is actually running to avoid stale state
        let state = if !is_daemon_running() {
            "stopped".to_string()
        } else {
            std::fs::read_to_string(&state_path).unwrap_or_else(|_| "stopped".to_string())
        };
        let state = state.trim();

        if format == "json" {
            println!("{}", format_state_json(state, &icons, ext_info.as_ref()));
        } else {
            println!("{}", state);
        }
        return Ok(());
    }

    // Follow mode: watch for changes using inotify
    use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    // Print initial state (check if daemon is running to avoid stale state)
    let state = if !is_daemon_running() {
        "stopped".to_string()
    } else {
        std::fs::read_to_string(&state_path).unwrap_or_else(|_| "stopped".to_string())
    };
    let state = state.trim();
    if format == "json" {
        println!("{}", format_state_json(state, &icons, ext_info.as_ref()));
    } else {
        println!("{}", state);
    }

    // Set up file watcher
    let (tx, rx) = channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        NotifyConfig::default().with_poll_interval(Duration::from_millis(100)),
    )?;

    // Watch the state file's parent directory (file may not exist yet)
    if let Some(parent) = state_path.parent() {
        std::fs::create_dir_all(parent)?;
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
    }

    // Also try to watch the file directly if it exists
    if state_path.exists() {
        let _ = watcher.watch(&state_path, RecursiveMode::NonRecursive);
    }

    let mut last_state = state.to_string();

    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(_event)) => {
                // File changed, read new state
                if let Ok(new_state) = std::fs::read_to_string(&state_path) {
                    let new_state = new_state.trim().to_string();
                    if new_state != last_state {
                        if format == "json" {
                            println!(
                                "{}",
                                format_state_json(&new_state, &icons, ext_info.as_ref())
                            );
                        } else {
                            println!("{}", new_state);
                        }
                        last_state = new_state;
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("Watch error: {:?}", e);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Check if daemon stopped (file deleted or process died)
                if (!state_path.exists() || !is_daemon_running()) && last_state != "stopped" {
                    if format == "json" {
                        println!(
                            "{}",
                            format_state_json("stopped", &icons, ext_info.as_ref())
                        );
                    } else {
                        println!("stopped");
                    }
                    last_state = "stopped".to_string();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }
    }

    Ok(())
}

/// Format state as JSON for Waybar consumption.
///
/// The `alt` field enables Waybar's format-icons feature for custom icon
/// mapping. The output format (key order, space-after-colon, embedded `\n`
/// in the tooltip) is part of the contract with waybar consumers; the
/// `format_state_json_pins_byte_exact_output` test locks it.
///
/// Values are escaped via `serde_json::to_string` so a device name or model
/// containing `"` or `\` can't produce malformed JSON. The outer template
/// stays hand-rolled to preserve the existing whitespace shape.
pub(crate) fn format_state_json(
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
/// double-quotes (e.g. `foo` → `"foo"`, `a"b` → `"a\"b"`). Used by
/// `format_state_json` so the outer template can keep its hand-rolled
/// whitespace shape while still getting correct escaping for free.
fn json_str(s: &str) -> String {
    serde_json::to_string(s).expect("serde_json never fails on &str")
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxtype::config;

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
    /// with and without extended info. Waybar consumers parse this JSON via
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
}
