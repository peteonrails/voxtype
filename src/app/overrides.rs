//! Apply top-level CLI flags onto a loaded `Config`. Every flag the user can
//! pass at the top level (e.g. `--clipboard`, `--model`, `--vad-threshold`)
//! that overrides a config field gets a stanza here.
//!
//! This is a Rule-3 hotspot per `docs/REFACTORING.md` — every new top-level
//! flag adds another `if let Some(...)` block. Don't extract a derive-macro or
//! builder yet; the axis of variation isn't stable. Land the move first, then
//! revisit dedup in a follow-up commit once the patterns are visible in one
//! place.

use voxtype::{config, setup, Cli};

/// Parse a comma-separated list of driver names into OutputDriver vec
fn parse_driver_order(s: &str) -> Result<Vec<config::OutputDriver>, String> {
    s.split(',')
        .map(|d| d.trim().parse::<config::OutputDriver>())
        .collect()
}

/// Apply every `cli.<flag>` onto `config` in place. Returns `top_level_model`
/// (a clone of `cli.model`) which is consumed downstream by
/// `send_record_command` so a subcommand-level `--model` can still defer to
/// the global flag when the subcommand didn't set its own.
pub(crate) fn apply_cli_overrides(config: &mut config::Config, cli: &Cli) -> Option<String> {
    let top_level_model = cli.model.clone();

    if cli.clipboard {
        config.output.mode = config::OutputMode::Clipboard;
    }
    if cli.paste {
        config.output.mode = config::OutputMode::Paste;
    }
    if cli.restore_clipboard {
        config.output.restore_clipboard = true;
    }
    if let Some(delay) = cli.restore_clipboard_delay_ms {
        config.output.restore_clipboard_delay_ms = delay;
    }
    if let Some(ref model) = cli.model {
        if setup::model::is_valid_model(model) {
            config.whisper.model = model.clone();
        } else {
            let default_model = &config.whisper.model;
            tracing::warn!(
                "Unknown model '{}', using default model '{}'",
                model,
                default_model
            );
            // Send desktop notification
            voxtype::notification::send_sync(
                "Voxtype: Invalid Model",
                &format!("Unknown model '{}', using '{}'", model, default_model),
            );
        }
    }
    if let Some(ref engine) = cli.engine {
        match engine.to_lowercase().as_str() {
            "whisper" => config.engine = config::TranscriptionEngine::Whisper,
            "parakeet" => config.engine = config::TranscriptionEngine::Parakeet,
            "moonshine" => config.engine = config::TranscriptionEngine::Moonshine,
            "sensevoice" => config.engine = config::TranscriptionEngine::SenseVoice,
            "paraformer" => config.engine = config::TranscriptionEngine::Paraformer,
            "dolphin" => config.engine = config::TranscriptionEngine::Dolphin,
            "omnilingual" => config.engine = config::TranscriptionEngine::Omnilingual,
            "cohere" => config.engine = config::TranscriptionEngine::Cohere,
            "soniox" => config.engine = config::TranscriptionEngine::Soniox,
            _ => {
                eprintln!(
                    "Error: Invalid engine '{}'. Valid options: whisper, parakeet, moonshine, sensevoice, paraformer, dolphin, omnilingual, cohere, soniox",
                    engine
                );
                std::process::exit(1);
            }
        }
    }

    // Hotkey overrides
    if let Some(ref hotkey) = cli.hotkey {
        config.hotkey.key = hotkey.clone();
    }
    if cli.toggle {
        config.hotkey.mode = config::ActivationMode::Toggle;
    }
    if cli.no_hotkey {
        config.hotkey.enabled = false;
    }
    if let Some(ref cancel_key) = cli.cancel_key {
        config.hotkey.cancel_key = Some(cancel_key.clone());
    }
    if let Some(ref model_modifier) = cli.model_modifier {
        config.hotkey.model_modifier = Some(model_modifier.clone());
    }

    // Whisper overrides
    if let Some(delay) = cli.pre_type_delay {
        config.output.pre_type_delay_ms = delay;
    }
    if let Some(delay) = cli.wtype_delay {
        tracing::warn!("--wtype-delay is deprecated, use --pre-type-delay instead");
        config.output.pre_type_delay_ms = delay;
    }
    if cli.no_whisper_context_optimization {
        config.whisper.context_window_optimization = false;
    }
    if let Some(ref prompt) = cli.initial_prompt {
        config.whisper.initial_prompt = Some(prompt.clone());
    }
    if let Some(ref lang) = cli.language {
        config.whisper.language = config::LanguageConfig::from_comma_separated(lang);
    }
    if cli.translate {
        config.whisper.translate = true;
    }
    if let Some(threads) = cli.threads {
        config.whisper.threads = Some(threads);
    }
    if cli.gpu_isolation {
        config.whisper.gpu_isolation = true;
    }
    if let Some(gpu_device) = cli.gpu_device {
        config.whisper.gpu_device = Some(gpu_device);
    }
    if cli.flash_attention {
        config.whisper.flash_attention = true;
    }
    if cli.on_demand_loading {
        config.whisper.on_demand_loading = true;
    }
    if let Some(ref mode) = cli.whisper_mode {
        match mode.to_lowercase().as_str() {
            "local" => config.whisper.mode = Some(config::WhisperMode::Local),
            "remote" => config.whisper.mode = Some(config::WhisperMode::Remote),
            "cli" => config.whisper.mode = Some(config::WhisperMode::Cli),
            _ => {
                eprintln!(
                    "Error: Invalid whisper mode '{}'. Valid options: local, remote, cli",
                    mode
                );
                std::process::exit(1);
            }
        }
    }
    if let Some(ref model) = cli.secondary_model {
        config.whisper.secondary_model = Some(model.clone());
    }
    if cli.eager_processing {
        config.whisper.eager_processing = true;
    }
    if let Some(ref endpoint) = cli.remote_endpoint {
        config.whisper.remote_endpoint = Some(endpoint.clone());
    }
    if let Some(ref model) = cli.remote_model {
        config.whisper.remote_model = Some(model.clone());
    }
    if let Some(ref key) = cli.remote_api_key {
        config.whisper.remote_api_key = Some(key.clone());
    }

    // Soniox overrides
    if let Some(ref key) = cli.soniox_api_key {
        config
            .soniox
            .get_or_insert_with(config::SonioxConfig::default)
            .api_key = Some(key.clone());
    }

    // Audio overrides
    if let Some(ref device) = cli.audio_device {
        config.audio.device = device.clone();
    }
    if let Some(max_dur) = cli.max_duration {
        config.audio.max_duration_secs = max_dur;
    }
    if cli.audio_feedback {
        config.audio.feedback.enabled = true;
    }
    if cli.no_audio_feedback {
        config.audio.feedback.enabled = false;
    }
    if cli.pause_media {
        config.audio.pause_media = true;
    }

    // Output overrides
    if let Some(ref append_text) = cli.append_text {
        config.output.append_text = Some(append_text.clone());
    }
    if cli.wtype_shift_prefix {
        config.output.wtype_shift_prefix = true;
    }
    if let Some(ref driver_str) = cli.driver {
        match parse_driver_order(driver_str) {
            Ok(drivers) => {
                config.output.driver_order = Some(drivers);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
    if cli.auto_submit {
        config.output.auto_submit = true;
    }
    if cli.no_auto_submit {
        config.output.auto_submit = false;
    }
    if cli.shift_enter_newlines {
        config.output.shift_enter_newlines = true;
    }
    if cli.no_shift_enter_newlines {
        config.output.shift_enter_newlines = false;
    }
    if cli.smart_auto_submit {
        config.text.smart_auto_submit = true;
    }
    if cli.no_smart_auto_submit {
        config.text.smart_auto_submit = false;
    }
    if let Some(delay) = cli.type_delay {
        config.output.type_delay_ms = delay;
    }
    if cli.fallback_to_clipboard {
        config.output.fallback_to_clipboard = true;
    }
    if cli.no_fallback_to_clipboard {
        config.output.fallback_to_clipboard = false;
    }
    if cli.spoken_punctuation {
        config.text.spoken_punctuation = true;
    }
    if cli.filter_fillers {
        config.text.filter_filler_words = true;
    }
    if cli.no_filter_fillers {
        config.text.filter_filler_words = false;
    }
    if let Some(ref keys) = cli.paste_keys {
        config.output.paste_keys = Some(keys.clone());
    }
    if let Some(ref layout) = cli.dotool_xkb_layout {
        config.output.dotool_xkb_layout = Some(layout.clone());
    }
    if let Some(ref variant) = cli.dotool_xkb_variant {
        config.output.dotool_xkb_variant = Some(variant.clone());
    }
    if let Some(ref layout) = cli.eitype_xkb_layout {
        config.output.eitype_xkb_layout = Some(layout.clone());
    }
    if let Some(ref variant) = cli.eitype_xkb_variant {
        config.output.eitype_xkb_variant = Some(variant.clone());
    }
    if let Some(ref path) = cli.file_path {
        config.output.file_path = Some(path.clone());
    }
    if let Some(ref mode) = cli.file_mode {
        match mode.to_lowercase().as_str() {
            "overwrite" => config.output.file_mode = config::FileMode::Overwrite,
            "append" => config.output.file_mode = config::FileMode::Append,
            _ => {
                eprintln!(
                    "Error: Invalid file mode '{}'. Valid options: overwrite, append",
                    mode
                );
                std::process::exit(1);
            }
        }
    }
    if let Some(ref cmd) = cli.pre_output_command {
        config.output.pre_output_command = Some(cmd.clone());
    }
    if let Some(ref cmd) = cli.post_output_command {
        config.output.post_output_command = Some(cmd.clone());
    }
    if let Some(ref cmd) = cli.pre_recording_command {
        config.output.pre_recording_command = Some(cmd.clone());
    }
    if cli.wait_for_modifier_release {
        config.output.wait_for_modifier_release = true;
    }
    if cli.no_wait_for_modifier_release {
        config.output.wait_for_modifier_release = false;
    }
    if let Some(ms) = cli.modifier_release_timeout_ms {
        config.output.modifier_release_timeout_ms = ms;
    }

    // VAD overrides
    if cli.vad {
        config.vad.enabled = true;
    }
    if let Some(threshold) = cli.vad_threshold {
        config.vad.threshold = threshold.clamp(0.0, 1.0);
    }
    if let Some(ref backend) = cli.vad_backend {
        config.vad.backend = match backend.to_lowercase().as_str() {
            "auto" => config::VadBackend::Auto,
            "energy" => config::VadBackend::Energy,
            "whisper" => config::VadBackend::Whisper,
            _ => {
                eprintln!(
                    "Unknown VAD backend '{}'. Valid options: auto, energy, whisper",
                    backend
                );
                std::process::exit(1);
            }
        };
    }
    if let Some(min_speech) = cli.vad_min_speech_ms {
        config.vad.min_speech_duration_ms = min_speech;
    }

    top_level_model
}
