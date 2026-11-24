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

/// Apply a paired `--foo` / `--no-foo` flag set onto a boolean config field.
/// `enable` wins over `disable`; both unset leaves the field untouched.
///
/// Mirrors the `override_from_flags` helper that landed in the cli.rs split
/// (commit dece381) — same fact, expressed once.
fn apply_bool_override(target: &mut bool, enable: bool, disable: bool) {
    if enable {
        *target = true;
    } else if disable {
        *target = false;
    }
}

/// Point `config` at the engine and model a `transcribe --model` names.
/// Unknown or undownloaded names are an error with the fix, not a silent
/// fallback to whatever the config already uses.
fn apply_transcribe_model(config: &mut config::Config, model: &str) {
    use voxtype::model_catalog::{override_config, override_route, OverrideRoute};
    match override_route(config, model) {
        Ok(OverrideRoute::WhisperManager) => config.whisper.model = model.to_string(),
        Ok(OverrideRoute::Active) => {}
        Ok(OverrideRoute::Separate { engine, model }) => {
            *config = override_config(config, engine, &model);
        }
        Err(problem) => {
            eprintln!("Error: {problem}");
            std::process::exit(1);
        }
    }
}

/// Apply every `cli.<flag>` onto `config` in place. Returns `top_level_model`
/// (a clone of `cli.model`) which is consumed downstream by
/// `send_record_command` so a subcommand-level `--model` can still defer to
/// the global flag when the subcommand didn't set its own.
pub(crate) fn apply_cli_overrides(config: &mut config::Config, cli: &Cli) -> Option<String> {
    let top_level_model = cli.model.clone();
    if let Some(enabled) = cli.tray {
        config.tray.enabled = enabled;
    }
    if let Some(enabled) = cli.omarchy_link_settings_plugin {
        config.omarchy.link_settings_plugin = enabled;
    }
    if let Some(enabled) = cli.streaming_recovery {
        config
            .streaming_recovery
            .get_or_insert_with(Default::default)
            .enabled = enabled;
    }
    if let Some(timeout_secs) = cli.streaming_stop_timeout_secs {
        config
            .streaming_recovery
            .get_or_insert_with(Default::default)
            .stop_timeout_secs = timeout_secs;
    }

    let history = config::HistoryOverrides {
        enabled: cli.history,
        paused: cli.history_paused,
        retention_days: cli.history_retention_days,
        exclude_apps: cli
            .history_exclude_apps
            .as_deref()
            .map(config::parse_history_app_list),
        record_app: cli.history_record_app,
        record_window_title: cli.history_record_window_title,
    };
    history.apply(&mut config.history);
    // The daemon re-reads [history] from disk when it changes; these still
    // win over the file afterwards.
    history.set_cli();

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
    // Model admission must see the mode selected for this invocation.
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
    if let (Some(model), Some(voxtype::Commands::Transcribe { .. })) = (&cli.model, &cli.command) {
        // `transcribe` runs this one file with the named model, so the name
        // picks its engine exactly as a per-recording override does in the
        // daemon, instead of only setting [whisper] model and leaving a
        // Parakeet config transcribing with Parakeet.
        apply_transcribe_model(config, model);
    } else if let Some(ref model) = cli.model {
        // Another engine's model selects that engine too. A Whisper name
        // only sets [whisper] model, as it always has, so scripts that pass
        // one alongside another engine keep working.
        let other_engine = voxtype::model_catalog::resolve_model(model)
            .filter(|r| r.engine != "whisper")
            .and_then(|r| {
                r.engine
                    .parse::<config::TranscriptionEngine>()
                    .ok()
                    .map(|engine| (engine, r.model))
            });
        if setup::model::is_valid_model(model) {
            config.whisper.model = model.clone();
        } else if let Some((engine, resolved)) = other_engine {
            config.set_model_for(engine, &resolved);
        } else if matches!(cli.command, Some(voxtype::Commands::Record { .. })) {
            // `record` forwards the name to the daemon, which rejects an unknown
            // one with its own error; a "using default" warning would contradict it.
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
        match engine.parse::<config::TranscriptionEngine>() {
            Ok(e) => config.engine = e,
            Err(_) => {
                eprintln!(
                    "Error: Invalid engine '{}'. Valid options: {}",
                    engine,
                    voxtype::cli::ENGINE_NAMES_CSV
                );
                std::process::exit(1);
            }
        }
    }
    if cli.gguf_model.is_some()
        || cli.gguf_architecture.is_some()
        || cli.gguf_language.is_some()
        || cli.gguf_backend.is_some()
        || cli.gguf_streaming
    {
        let gguf = config.gguf.get_or_insert_with(config::GgufConfig::default);
        if let Some(model) = &cli.gguf_model {
            gguf.model = model.clone();
        }
        if let Some(architecture) = &cli.gguf_architecture {
            gguf.architecture = architecture.clone();
        }
        if let Some(language) = &cli.gguf_language {
            gguf.language = language.clone();
        }
        if let Some(backend) = &cli.gguf_backend {
            gguf.gguf_backend = backend.clone();
        }
        if cli.gguf_streaming {
            gguf.streaming = true;
        }
        if let Err(message) = voxtype::model_selection::validate_gguf_choice(
            &gguf.model,
            &gguf.architecture,
            &gguf.language,
            gguf.streaming,
        ) {
            eprintln!("Error: {message}");
            std::process::exit(1);
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
    if let Some(ref paste_last_key) = cli.paste_last_key {
        config.hotkey.paste_last_key = Some(paste_last_key.clone());
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
        if config.engine == config::TranscriptionEngine::Gguf {
            config.gguf.get_or_insert_with(Default::default).language = lang.clone();
        } else {
            config.whisper.language = config::LanguageConfig::from_comma_separated(lang);
        }
    }
    if cli.translate {
        config.whisper.translate = true;
    }
    if let Some(threads) = cli.threads {
        if config.engine == config::TranscriptionEngine::Gguf {
            config.gguf.get_or_insert_with(Default::default).threads = Some(threads);
        } else {
            config.whisper.threads = Some(threads);
        }
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
        if config.engine == config::TranscriptionEngine::Gguf {
            config
                .gguf
                .get_or_insert_with(Default::default)
                .on_demand_loading = true;
        } else {
            config.whisper.on_demand_loading = true;
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
    if let Some(ref region) = cli.soniox_region {
        config
            .soniox
            .get_or_insert_with(config::SonioxConfig::default)
            .region = region.clone();
    }

    // ElevenLabs overrides. Any provider-specific CLI flag materializes the
    // optional section so source builds can be configured without TOML.
    if cli.elevenlabs_api_key.is_some()
        || cli.elevenlabs_region.is_some()
        || cli.elevenlabs_language.is_some()
        || cli.elevenlabs_mode.is_some()
        || cli.elevenlabs_model.is_some()
        || cli.elevenlabs_commit_strategy.is_some()
        || cli.elevenlabs_no_verbatim
        || cli.elevenlabs_verbatim
        || cli.elevenlabs_vad_silence_threshold_secs.is_some()
    {
        let elevenlabs = config
            .elevenlabs
            .get_or_insert_with(config::ElevenLabsConfig::default);
        if let Some(ref key) = cli.elevenlabs_api_key {
            elevenlabs.api_key = Some(key.clone());
        }
        if let Some(ref region) = cli.elevenlabs_region {
            // Clap validates this closed set before overrides are applied.
            elevenlabs.region = region
                .parse::<config::ElevenLabsRegion>()
                .expect("validated ElevenLabs region");
        }
        if let Some(ref language) = cli.elevenlabs_language {
            elevenlabs.set_language_code(language);
        }
        if let Some(ref mode) = cli.elevenlabs_mode {
            elevenlabs.mode = mode
                .parse::<config::ElevenLabsMode>()
                .expect("validated ElevenLabs mode");
        }
        if let Some(ref model) = cli.elevenlabs_model {
            elevenlabs.model = model.clone();
        }
        if let Some(ref strategy) = cli.elevenlabs_commit_strategy {
            elevenlabs.commit_strategy = strategy
                .parse::<config::ElevenLabsCommitStrategy>()
                .expect("validated ElevenLabs commit strategy");
        }
        apply_bool_override(
            &mut elevenlabs.no_verbatim,
            cli.elevenlabs_no_verbatim,
            cli.elevenlabs_verbatim,
        );
        if let Some(value) = cli.elevenlabs_vad_silence_threshold_secs {
            elevenlabs
                .set_vad_silence_threshold_secs(value)
                .expect("validated ElevenLabs VAD silence threshold");
        }
    }

    // Audio overrides
    if let Some(ref device) = cli.audio_device {
        config.audio.device = device.clone();
    }
    if let Some(max_dur) = cli.max_duration {
        config.audio.max_duration_secs = max_dur;
    }
    apply_bool_override(
        &mut config.audio.feedback.enabled,
        cli.audio_feedback,
        cli.no_audio_feedback,
    );
    apply_bool_override(
        &mut config.audio.keep_failed_recordings,
        cli.keep_failed_recordings,
        cli.no_keep_failed_recordings,
    );
    if cli.pause_media {
        config.audio.pause_media = true;
    }
    if cli.duck_media {
        config.audio.duck_media = true;
    }
    if let Some(volume) = cli.duck_media_volume {
        config.audio.duck_media_volume_percent = volume.min(150);
    }
    if let Some(fade_ms) = cli.duck_media_fade_ms {
        config.audio.duck_media_fade_ms = fade_ms;
    }
    apply_bool_override(
        &mut config.audio.dead_input.enabled,
        cli.dead_input_check,
        cli.no_dead_input_check,
    );
    if let Some(window_ms) = cli.dead_input_window {
        config.audio.dead_input.window_ms = window_ms;
    }
    if let Some(threshold) = cli.dead_input_threshold {
        config.audio.dead_input.threshold_dbfs = threshold;
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
    apply_bool_override(
        &mut config.output.auto_submit,
        cli.auto_submit,
        cli.no_auto_submit,
    );
    apply_bool_override(
        &mut config.output.shift_enter_newlines,
        cli.shift_enter_newlines,
        cli.no_shift_enter_newlines,
    );
    apply_bool_override(
        &mut config.text.smart_auto_submit,
        cli.smart_auto_submit,
        cli.no_smart_auto_submit,
    );
    if let Some(delay) = cli.type_delay {
        config.output.type_delay_ms = delay;
    }
    apply_bool_override(
        &mut config.output.fallback_to_clipboard,
        cli.fallback_to_clipboard,
        cli.no_fallback_to_clipboard,
    );
    apply_bool_override(
        &mut config.output.keep_undelivered_text,
        cli.keep_undelivered_text,
        cli.no_keep_undelivered_text,
    );
    apply_bool_override(
        &mut config.text.lard_disfluency,
        cli.lard_disfluency,
        cli.no_lard_disfluency,
    );
    if let Some(path) = &cli.lard_model_path {
        config.text.lard_model_path = Some(path.clone());
    }
    if let Some(language) = &cli.text_language {
        if let Err(message) = config.text.set_language(language) {
            eprintln!("Error: --text-language: {message}");
            std::process::exit(1);
        }
    }
    apply_bool_override(
        &mut config.text.punctuation_casing,
        cli.punctuation_casing,
        cli.no_punctuation_casing,
    );
    apply_bool_override(
        &mut config.text.spoken_punctuation,
        cli.spoken_punctuation,
        cli.no_spoken_punctuation,
    );
    apply_bool_override(
        &mut config.text.collapse_restarts,
        cli.collapse_restarts,
        cli.no_collapse_restarts,
    );
    apply_bool_override(
        &mut config.text.repair_sentence_breaks,
        cli.repair_sentence_breaks,
        cli.no_repair_sentence_breaks,
    );
    apply_bool_override(
        &mut config.text.format_numbers,
        cli.format_numbers,
        cli.no_format_numbers,
    );
    apply_bool_override(
        &mut config.text.filter_filler_words,
        cli.filter_fillers,
        cli.no_filter_fillers,
    );
    if let Some(ref keys) = cli.paste_keys {
        config.output.paste_keys = Some(keys.clone());
    }
    if let Some(ref keys) = cli.paste_keys_terminal {
        config.output.paste_keys_terminal = Some(keys.clone());
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
    apply_bool_override(
        &mut config.output.wait_for_modifier_release,
        cli.wait_for_modifier_release,
        cli.no_wait_for_modifier_release,
    );
    if let Some(ms) = cli.modifier_release_timeout_ms {
        config.output.modifier_release_timeout_ms = ms;
    }

    apply_llm_overrides(config, cli);

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

/// `--post-process-*` and `--summary-*` (and their VOXTYPE_ variables).
fn apply_llm_overrides(config: &mut config::Config, cli: &Cli) {
    let any_post_process = cli.post_process_backend.is_some()
        || cli.post_process_model.is_some()
        || cli.post_process_url.is_some()
        || cli.post_process_api_key_file.is_some()
        || cli.post_process_prompt.is_some();
    if any_post_process {
        let pp = config
            .output
            .post_process
            .get_or_insert_with(Default::default);
        if let Some(backend) = &cli.post_process_backend {
            pp.backend = backend.clone();
        }
        if let Some(model) = &cli.post_process_model {
            pp.model = Some(model.clone());
        }
        if let Some(url) = &cli.post_process_url {
            pp.url = Some(url.clone());
        }
        if let Some(file) = &cli.post_process_api_key_file {
            pp.api_key_file = Some(file.clone());
        }
        if let Some(prompt) = &cli.post_process_prompt {
            pp.prompt = Some(prompt.clone());
        }
    }
    let summary = &mut config.meeting.summary;
    if let Some(backend) = &cli.summary_backend {
        summary.backend = backend.clone();
    }
    if let Some(model) = &cli.summary_omarchy_model {
        summary.omarchy_model = Some(model.clone());
    }
    if let Some(url) = &cli.summary_openai_url {
        summary.openai_url = Some(url.clone());
    }
    if let Some(model) = &cli.summary_openai_model {
        summary.openai_model = Some(model.clone());
    }
    if let Some(file) = &cli.summary_openai_api_key_file {
        summary.openai_api_key_file = Some(file.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::ffi::OsString;
    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());
    const ELEVENLABS_ENV_VARS: [&str; 8] = [
        "ELEVENLABS_API_KEY",
        "VOXTYPE_ELEVENLABS_REGION",
        "VOXTYPE_ELEVENLABS_LANGUAGE",
        "VOXTYPE_ELEVENLABS_MODE",
        "VOXTYPE_ELEVENLABS_MODEL",
        "VOXTYPE_ELEVENLABS_COMMIT_STRATEGY",
        "VOXTYPE_ELEVENLABS_NO_VERBATIM",
        "VOXTYPE_ELEVENLABS_VAD_SILENCE_THRESHOLD_SECS",
    ];

    struct EnvRestore(Vec<(&'static str, Option<OsString>)>);

    impl EnvRestore {
        fn set(values: &[(&'static str, Option<&str>)]) -> Self {
            let originals = ELEVENLABS_ENV_VARS
                .iter()
                .map(|name| (*name, std::env::var_os(name)))
                .collect();

            for name in ELEVENLABS_ENV_VARS {
                std::env::remove_var(name);
            }
            for (name, value) in values {
                if let Some(value) = value {
                    std::env::set_var(name, value);
                }
            }

            Self(originals)
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for (name, value) in self.0.drain(..) {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }

    fn load_without_config_file() -> config::Config {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.toml");
        config::load_config(Some(&missing)).unwrap()
    }

    #[test]
    fn elevenlabs_environment_materializes_and_layers_all_values() {
        let _lock = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _env = EnvRestore::set(&[
            ("ELEVENLABS_API_KEY", Some("env-secret")),
            ("VOXTYPE_ELEVENLABS_REGION", Some("eu")),
            ("VOXTYPE_ELEVENLABS_LANGUAGE", Some("  en  ")),
            ("VOXTYPE_ELEVENLABS_MODE", Some("partials")),
            ("VOXTYPE_ELEVENLABS_MODEL", Some("scribe_env")),
            ("VOXTYPE_ELEVENLABS_COMMIT_STRATEGY", Some("manual")),
            ("VOXTYPE_ELEVENLABS_NO_VERBATIM", Some("true")),
            ("VOXTYPE_ELEVENLABS_VAD_SILENCE_THRESHOLD_SECS", Some("0.8")),
        ]);

        let config = load_without_config_file();
        let elevenlabs = config.elevenlabs.expect("environment creates section");
        assert_eq!(
            (
                elevenlabs.api_key.as_deref(),
                elevenlabs.region,
                elevenlabs.language_code.as_deref(),
                elevenlabs.mode,
                elevenlabs.model.as_str(),
                elevenlabs.commit_strategy,
                elevenlabs.no_verbatim,
                elevenlabs.vad_silence_threshold_secs,
            ),
            (
                Some("env-secret"),
                config::ElevenLabsRegion::Eu,
                Some("en"),
                config::ElevenLabsMode::Partials,
                "scribe_env",
                config::ElevenLabsCommitStrategy::Manual,
                true,
                0.8,
            )
        );
    }

    #[test]
    fn elevenlabs_mode_flag_materializes_absent_section() {
        let _lock = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _env = EnvRestore::set(&[]);
        let mut config = load_without_config_file();
        assert!(config.elevenlabs.is_none());

        let cli = Cli::try_parse_from([
            "voxtype",
            "--elevenlabs-region",
            "us",
            "--elevenlabs-mode",
            "batch",
        ])
        .unwrap();
        apply_cli_overrides(&mut config, &cli);

        let elevenlabs = config.elevenlabs.expect("CLI creates section");
        assert_eq!(
            (elevenlabs.region, elevenlabs.mode),
            (config::ElevenLabsRegion::Us, config::ElevenLabsMode::Batch,)
        );
    }

    #[test]
    fn elevenlabs_cleanup_flags_materialize_absent_section() {
        let _lock = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _env = EnvRestore::set(&[]);
        let mut config = load_without_config_file();
        assert!(config.elevenlabs.is_none());

        let cli = Cli::try_parse_from([
            "voxtype",
            "--elevenlabs-commit-strategy",
            "manual",
            "--elevenlabs-no-verbatim",
        ])
        .unwrap();
        apply_cli_overrides(&mut config, &cli);

        let elevenlabs = config.elevenlabs.expect("CLI creates section");
        assert_eq!(
            (elevenlabs.commit_strategy, elevenlabs.no_verbatim),
            (config::ElevenLabsCommitStrategy::Manual, true)
        );
    }

    #[test]
    fn elevenlabs_cli_overrides_environment_values() {
        let _lock = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _env = EnvRestore::set(&[
            ("ELEVENLABS_API_KEY", Some("env-secret")),
            ("VOXTYPE_ELEVENLABS_REGION", Some("eu")),
            ("VOXTYPE_ELEVENLABS_LANGUAGE", Some("fr")),
            ("VOXTYPE_ELEVENLABS_MODE", Some("batch")),
            ("VOXTYPE_ELEVENLABS_MODEL", Some("scribe_env")),
            ("VOXTYPE_ELEVENLABS_COMMIT_STRATEGY", Some("vad")),
            ("VOXTYPE_ELEVENLABS_NO_VERBATIM", Some("false")),
            ("VOXTYPE_ELEVENLABS_VAD_SILENCE_THRESHOLD_SECS", Some("1.2")),
        ]);

        let mut config = load_without_config_file();
        let cli = Cli::try_parse_from([
            "voxtype",
            "--engine",
            "elevenlabs",
            "--elevenlabs-api-key",
            "cli-secret",
            "--elevenlabs-region",
            "singapore",
            "--elevenlabs-language",
            "  de  ",
            "--elevenlabs-mode",
            "realtime",
            "--elevenlabs-model",
            "scribe_cli",
            "--elevenlabs-commit-strategy",
            "manual",
            "--elevenlabs-no-verbatim",
            "--elevenlabs-vad-silence-threshold-secs",
            "0.6",
        ])
        .unwrap();

        apply_cli_overrides(&mut config, &cli);

        let engine = config.engine;
        let elevenlabs = config.elevenlabs.expect("CLI keeps section materialized");
        assert_eq!(
            (
                engine,
                elevenlabs.api_key.as_deref(),
                elevenlabs.region,
                elevenlabs.language_code.as_deref(),
                elevenlabs.mode,
                elevenlabs.model.as_str(),
                elevenlabs.commit_strategy,
                elevenlabs.no_verbatim,
                elevenlabs.vad_silence_threshold_secs,
            ),
            (
                config::TranscriptionEngine::ElevenLabs,
                Some("cli-secret"),
                config::ElevenLabsRegion::Singapore,
                Some("de"),
                config::ElevenLabsMode::Realtime,
                "scribe_cli",
                config::ElevenLabsCommitStrategy::Manual,
                true,
                0.6,
            )
        );
    }

    #[test]
    fn elevenlabs_verbatim_flag_overrides_true_configuration() {
        let _lock = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _env = EnvRestore::set(&[]);
        let mut config = load_without_config_file();
        config.elevenlabs = Some(config::ElevenLabsConfig {
            no_verbatim: true,
            ..config::ElevenLabsConfig::default()
        });
        let cli = Cli::try_parse_from(["voxtype", "--elevenlabs-verbatim"]).unwrap();

        apply_cli_overrides(&mut config, &cli);

        assert!(!config.elevenlabs.unwrap().no_verbatim);
    }

    #[test]
    fn elevenlabs_cli_rejects_conflicting_cleanup_flags_and_invalid_values() {
        assert!(Cli::try_parse_from([
            "voxtype",
            "--elevenlabs-no-verbatim",
            "--elevenlabs-verbatim",
        ])
        .is_err());
        assert!(
            Cli::try_parse_from(["voxtype", "--elevenlabs-commit-strategy", "automatic",]).is_err()
        );
        assert!(Cli::try_parse_from(["voxtype", "--elevenlabs-mode", "continuous",]).is_err());
        for invalid in ["0", "-0.1", "NaN", "inf"] {
            assert!(Cli::try_parse_from([
                "voxtype",
                "--elevenlabs-vad-silence-threshold-secs",
                invalid,
            ])
            .is_err());
        }
    }

    #[test]
    fn invalid_environment_commit_strategy_retains_default_and_materializes_section() {
        let _lock = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _env = EnvRestore::set(&[("VOXTYPE_ELEVENLABS_COMMIT_STRATEGY", Some("automatic"))]);

        let elevenlabs = load_without_config_file()
            .elevenlabs
            .expect("provider environment materializes section");
        assert_eq!(
            elevenlabs.commit_strategy,
            config::ElevenLabsCommitStrategy::Vad
        );
    }
}
