//! What `voxtype meeting start` may ask for, and which transcriber serves it.
//!
//! The CLI checks a start's engine, model and language here before writing
//! the start trigger, so a meeting that could not transcribe is refused up
//! front with the fix, and the daemon checks again when it reads the trigger.

use crate::config::{Config, MeetingStartOptions, TranscriptionEngine, WhisperMode};

/// The configuration a meeting started with `options` transcribes with, or
/// why it cannot start: an unknown or uncompiled engine, a model that is not
/// installed, a language the checkpoint does not take.
///
/// A start without an engine or model keeps dictation's, which needs no
/// check here: dictation already runs on it.
pub fn meeting_transcription_config(
    config: &Config,
    options: &MeetingStartOptions,
) -> Result<Config, String> {
    let meeting = config.with_meeting_mode_overrides(options)?;
    if options.engine.is_none() && options.model.is_none() {
        return Ok(meeting);
    }

    let engine = meeting.engine.name();
    if !crate::config_set::engine_feature_compiled(engine) {
        let feature =
            crate::setup::variant_check::required_feature(meeting.engine).unwrap_or(engine);
        let hint =
            crate::setup::install_hint::hint(crate::setup::install_hint::Need::Feature(feature));
        return Err(format!(
            "This voxtype binary was built without the {engine} engine. To use it, {hint}."
        ));
    }
    crate::model_selection::validate_config_backend(&meeting)?;

    let model = meeting.model_name().to_string();
    if needs_local_model(&meeting) {
        if crate::model_selection::is_gguf(&model) {
            crate::model_selection::validate_ready(engine, &model)?;
        } else if !crate::model_catalog::model_installed(engine, &model) {
            return Err(format!(
                "The {engine} model '{model}' is not installed. Download it with: {}",
                crate::model_catalog::download_command(engine, &model)
            ));
        }
    }
    Ok(meeting)
}

/// Whether the meeting's engine reads a model from the models directory,
/// rather than a server, a CLI or the cloud.
fn needs_local_model(config: &Config) -> bool {
    match config.engine {
        TranscriptionEngine::Soniox => false,
        TranscriptionEngine::Whisper => config.whisper.effective_mode() == WhisperMode::Local,
        _ => true,
    }
}

/// Whether a meeting configured as `meeting` can transcribe with the
/// transcriber dictation already has loaded for `dictation`: the same
/// engine, model and every other transcription setting. Any difference,
/// including the Soniox async switch meetings make, means a separate load.
pub fn shares_dictation_transcriber(dictation: &Config, meeting: &Config) -> bool {
    if dictation.on_demand_loading() {
        return false;
    }
    // GPU isolation spawns a worker per transcription; sharing its prepared
    // worker between dictation and the meeting would hand one of them a
    // process the other is about to use. Each keeps its own, as before.
    if dictation.engine == TranscriptionEngine::Whisper && dictation.whisper.gpu_isolation {
        return false;
    }
    match (
        serde_json::to_value(dictation),
        serde_json::to_value(meeting),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(
        engine: Option<&str>,
        model: Option<&str>,
        language: Option<&str>,
    ) -> MeetingStartOptions {
        MeetingStartOptions {
            engine: engine.map(str::to_string),
            model: model.map(str::to_string),
            language: language.map(str::to_string),
            url: None,
            ..Default::default()
        }
    }

    fn resident(engine: TranscriptionEngine) -> Config {
        let mut config = Config {
            engine,
            ..Config::default()
        };
        config.whisper.on_demand_loading = false;
        config
    }

    #[test]
    fn no_choices_keep_dictations_transcriber() {
        let dictation = resident(TranscriptionEngine::Whisper);
        let meeting = meeting_transcription_config(&dictation, &opts(None, None, None)).unwrap();
        assert!(shares_dictation_transcriber(&dictation, &meeting));
    }

    #[test]
    fn a_different_model_or_language_loads_separately() {
        let dictation = resident(TranscriptionEngine::Whisper);
        let other_model = dictation
            .with_meeting_mode_overrides(&opts(None, Some("tiny.en"), None))
            .unwrap();
        assert_eq!(other_model.whisper.model, "tiny.en");
        assert!(!shares_dictation_transcriber(&dictation, &other_model));

        let mut multilingual = dictation.clone();
        multilingual.whisper.model = "large-v3-turbo".into();
        let other_language = multilingual
            .with_meeting_mode_overrides(&opts(None, None, Some("de")))
            .unwrap();
        assert!(!shares_dictation_transcriber(
            &multilingual,
            &other_language
        ));
        // Naming dictation's own model is still a match.
        let same = dictation
            .with_meeting_mode_overrides(&opts(
                Some("whisper"),
                Some(&dictation.whisper.model),
                None,
            ))
            .unwrap();
        assert!(shares_dictation_transcriber(&dictation, &same));
    }

    #[test]
    fn nothing_resident_means_nothing_to_share() {
        let mut dictation = resident(TranscriptionEngine::Whisper);
        dictation.whisper.on_demand_loading = true;
        assert!(!shares_dictation_transcriber(
            &dictation,
            &dictation.clone()
        ));

        let mut isolated = resident(TranscriptionEngine::Whisper);
        isolated.whisper.gpu_isolation = true;
        assert!(!shares_dictation_transcriber(&isolated, &isolated.clone()));
    }

    #[test]
    fn soniox_meetings_never_share_the_realtime_transcriber() {
        let mut dictation = resident(TranscriptionEngine::Soniox);
        dictation.soniox = Some(crate::config::SonioxConfig {
            api_key: Some("k".into()),
            ..Default::default()
        });
        let meeting = dictation
            .with_meeting_mode_overrides(&MeetingStartOptions::default())
            .unwrap();
        assert!(!shares_dictation_transcriber(&dictation, &meeting));
    }

    #[test]
    fn unknown_engine_model_and_mismatches_are_refused() {
        let config = Config::default();
        let err =
            meeting_transcription_config(&config, &opts(Some("nope"), None, None)).unwrap_err();
        assert!(err.starts_with("Unknown engine 'nope'"), "{err}");
        let err = meeting_transcription_config(&config, &opts(None, Some("no-such-model"), None))
            .unwrap_err();
        assert!(err.starts_with("Unknown model 'no-such-model'"), "{err}");
        let err =
            meeting_transcription_config(&config, &opts(Some("parakeet"), Some("base.en"), None))
                .unwrap_err();
        assert_eq!(
            err,
            "Model 'base.en' is a whisper model, not a parakeet model"
        );
    }

    #[test]
    fn a_language_the_checkpoint_does_not_take_is_refused() {
        let mut config = Config::default();
        config.whisper.model = "base.en".into();
        let err = meeting_transcription_config(&config, &opts(None, None, Some("de"))).unwrap_err();
        assert!(err.contains("Language 'de' is not available"), "{err}");
        // English on an English-only checkpoint is what it does anyway.
        assert!(meeting_transcription_config(&config, &opts(None, None, Some("en"))).is_ok());
    }

    #[test]
    fn an_engine_this_binary_lacks_is_refused_with_a_hint() {
        if crate::config_set::engine_feature_compiled("omnilingual") {
            return;
        }
        let err = meeting_transcription_config(
            &Config::default(),
            &opts(Some("omnilingual"), None, None),
        )
        .unwrap_err();
        assert!(
            err.starts_with(
                "This voxtype binary was built without the omnilingual engine. To use it,"
            ),
            "{err}"
        );
    }
}
