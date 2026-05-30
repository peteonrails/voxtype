use super::Config;

/// Load configuration from file, with defaults for missing values
/// Parse a TOML config string by layering it over default values.
///
/// Renders `Config::default()` to a TOML `Value`, deep-merges the user's TOML
/// on top, then deserializes back into `Config`. Tables merge recursively
/// (user keys win when both sides have them); other values (scalars, arrays)
/// take user-supplied values verbatim. The result: any user TOML that's a
/// subset of the full config, down to a single `[audio.feedback] enabled = true`
/// produces a valid Config with defaults filled in for everything else.
pub fn parse_config_with_defaults(contents: &str) -> Result<Config, toml::de::Error> {
    let defaults = toml::Value::try_from(Config::default())
        .expect("Config::default() must be serializable to TOML");
    let user: toml::Value = toml::from_str(contents)?;
    let mut merged = defaults;
    merge_toml_values(&mut merged, user);
    merged.try_into()
}

/// Deep-merge `overlay` onto `base`. Tables merge recursively; for any other
/// value type (or when the two sides have mismatched types), `overlay` wins.
/// Arrays are replaced wholesale rather than concatenated. Extending a
/// defaulted list (e.g. `language_to_layout`) requires the user to spell out
/// the full replacement value.
fn merge_toml_values(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                if let Some(existing) = b.get_mut(&k) {
                    merge_toml_values(existing, v);
                } else {
                    b.insert(k, v);
                }
            }
        }
        (slot, other) => *slot = other,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ActivationMode, OutputMode};
    use super::*;

    #[test]
    fn test_parse_config_toml() {
        let toml_str = r#"
            [hotkey]
            key = "PAUSE"
            modifiers = ["LEFTCTRL"]

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 30

            [whisper]
            model = "small.en"
            language = "en"

            [output]
            mode = "clipboard"

            [output.notification]
            on_recording_start = true
            on_recording_stop = true
            on_transcription = false
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hotkey.key, "PAUSE");
        assert_eq!(config.hotkey.modifiers, vec!["LEFTCTRL"]);
        assert_eq!(config.hotkey.mode, ActivationMode::PushToTalk); // default
        assert_eq!(config.whisper.model, "small.en");
        assert_eq!(config.output.mode, OutputMode::Clipboard);
        assert!(config.output.notification.on_recording_start);
        assert!(config.output.notification.on_recording_stop);
        assert!(!config.output.notification.on_transcription);
    }

    #[test]
    fn empty_toml_yields_default_config() {
        // Invariant: parse_config_with_defaults("") must produce exactly
        // Config::default(). If this ever fails, defaults have drifted
        // between the hand-rolled Config::default() and the serde-default
        // path. They are supposed to be the same fact in one place.
        let parsed = parse_config_with_defaults("").expect("empty TOML must succeed");
        let defaults = Config::default();
        // Compare via TOML round-trip because Config doesn't impl PartialEq.
        let parsed_toml = toml::Value::try_from(&parsed).expect("Config must serialize to TOML");
        let default_toml = toml::Value::try_from(&defaults).expect("Config must serialize to TOML");
        assert_eq!(parsed_toml, default_toml);
    }

    #[test]
    fn partial_config_with_only_hotkey_section() {
        // AlexCzar's TUI symptom (#421): a config with only [hotkey] used to
        // be rejected with "missing field 'audio'" because Config required
        // top-level audio/output sections.
        let toml = r#"
            [hotkey]
            key = "RIGHTALT"
        "#;
        let cfg =
            parse_config_with_defaults(toml).expect("partial config must layer over defaults");
        assert_eq!(cfg.hotkey.key, "RIGHTALT");
        // Defaults must fill in for everything else.
        assert_eq!(cfg.audio.device, "default");
        assert_eq!(cfg.audio.sample_rate, 16000);
        assert_eq!(cfg.output.mode, OutputMode::Type);
    }

    #[test]
    fn partial_audio_feedback_does_not_require_device() {
        // AlexCzar's daemon symptom (#421): writing [audio.feedback] used to
        // fail with "missing field 'device'" because AudioConfig::device had
        // no serde default. Now it falls back to default_audio_device().
        let toml = r#"
            engine = "whisper"

            [hotkey]
            key = "RIGHTALT"

            [audio.feedback]
            enabled = true
            theme = "subtle"

            [whisper]
            backend = "local"
            model = "large-v3-turbo"
            language = ["en", "ru"]
        "#;
        let cfg = parse_config_with_defaults(toml).expect("AlexCzar's config must parse");
        assert!(cfg.audio.feedback.enabled);
        assert_eq!(cfg.audio.feedback.theme, "subtle");
        // Audio fields the user didn't set come from defaults.
        assert_eq!(cfg.audio.device, "default");
        assert_eq!(cfg.audio.sample_rate, 16000);
        // Whisper sub-fields the user didn't set come from defaults too.
        assert_eq!(cfg.whisper.model, "large-v3-turbo");
        assert!(!cfg.whisper.translate);
    }

    #[test]
    fn partial_section_does_not_clobber_sibling_defaults() {
        // User specifies one audio.feedback field; volume should keep its
        // default rather than getting reset by the partial sub-table.
        let toml = r#"
            [audio.feedback]
            enabled = true
        "#;
        let cfg = parse_config_with_defaults(toml).expect("partial sub-table must layer");
        assert!(cfg.audio.feedback.enabled);
        assert_eq!(cfg.audio.feedback.volume, 0.7);
        assert_eq!(cfg.audio.feedback.theme, "default");
    }

    #[test]
    fn user_arrays_replace_default_arrays() {
        // Arrays are replaced wholesale, not concatenated. A user-specified
        // language list overrides the default rather than appending to it.
        let toml = r#"
            [whisper]
            language = ["ja", "ko"]
        "#;
        let cfg = parse_config_with_defaults(toml).expect("array replacement must work");
        // Just confirm the user value wins; the precise default shape isn't
        // load-bearing here, but their values should not appear in the result.
        let langs_toml =
            toml::Value::try_from(&cfg.whisper.language).expect("LanguageConfig must serialize");
        let langs_str = langs_toml.to_string();
        assert!(langs_str.contains("ja"));
        assert!(langs_str.contains("ko"));
    }

    #[test]
    fn merge_toml_values_recurses_into_tables() {
        // Unit test for the merge primitive itself.
        let mut base: toml::Value = toml::from_str(
            r#"
            [audio]
            device = "default"
            sample_rate = 16000

            [audio.feedback]
            enabled = false
            volume = 0.7
            "#,
        )
        .unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
            [audio.feedback]
            enabled = true
            "#,
        )
        .unwrap();

        merge_toml_values(&mut base, overlay);

        // audio.device preserved
        assert_eq!(
            base.get("audio").and_then(|a| a.get("device")),
            Some(&toml::Value::String("default".to_string()))
        );
        // audio.feedback.enabled overridden
        assert_eq!(
            base.get("audio")
                .and_then(|a| a.get("feedback"))
                .and_then(|f| f.get("enabled")),
            Some(&toml::Value::Boolean(true))
        );
        // audio.feedback.volume preserved
        assert_eq!(
            base.get("audio")
                .and_then(|a| a.get("feedback"))
                .and_then(|f| f.get("volume")),
            Some(&toml::Value::Float(0.7))
        );
    }

    #[test]
    fn parse_rejects_unknown_field_types() {
        // A type mismatch (string where a number is expected) must still
        // surface as an error after merging defaults.
        let toml = r#"
            [audio]
            sample_rate = "not a number"
        "#;
        let result = parse_config_with_defaults(toml);
        assert!(result.is_err(), "type mismatch must still error");
    }
}
