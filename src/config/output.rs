//! Output configuration (drivers, modes, layout hints).

use serde::{Deserialize, Serialize};

use std::collections::HashMap;
use std::path::PathBuf;

use super::default_true;
use super::{NotificationConfig, PostProcessConfig};

fn default_restore_clipboard_delay() -> u32 {
    200 // 200ms - delay for paste to complete before restoring clipboard
}

/// Text output configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutputConfig {
    /// Primary output mode
    #[serde(default)]
    pub mode: OutputMode,

    /// Fall back to clipboard if typing fails
    #[serde(default = "default_true")]
    pub fallback_to_clipboard: bool,

    /// Custom driver order for type mode (overrides default: wtype -> dotool -> ydotool -> clipboard)
    /// Specify which drivers to try and in what order.
    /// Example: ["ydotool", "wtype"] to prefer ydotool over wtype
    #[serde(default)]
    pub driver_order: Option<Vec<OutputDriver>>,

    /// Notification settings
    #[serde(default)]
    pub notification: NotificationConfig,

    /// Delay between typed characters (ms), 0 for fastest
    #[serde(default)]
    pub type_delay_ms: u32,

    /// Delay before typing starts (ms), allows virtual keyboard to initialize
    /// Helps prevent first character from being dropped on some compositors
    #[serde(default)]
    pub pre_type_delay_ms: u32,

    /// DEPRECATED: Use pre_type_delay_ms instead. Kept for backwards compatibility.
    #[serde(default)]
    pub wtype_delay_ms: u32,

    /// Automatically submit (send Enter key) after outputting transcribed text
    /// Useful for chat applications, command lines, or forms where you want
    /// to auto-submit after dictation
    #[serde(default)]
    pub auto_submit: bool,

    /// Text to append after each transcription (e.g., " " for a space)
    /// Appended after the transcription but before auto_submit
    /// Useful for separating sentences when dictating paragraphs incrementally
    #[serde(default)]
    pub append_text: Option<String>,

    /// Convert newlines to Shift+Enter instead of regular Enter
    /// Useful for applications where Enter submits (e.g., Cursor IDE, Slack, Discord)
    #[serde(default)]
    pub shift_enter_newlines: bool,

    /// Prefix wtype output with a Shift key press/release
    /// Workaround for apps (e.g., Discord) that drop the first CJK character
    #[serde(default)]
    pub wtype_shift_prefix: bool,

    /// Command to run when recording starts (e.g., switch to compositor submap)
    /// Useful for entering a mode where cancel keybindings are effective
    #[serde(default)]
    pub pre_recording_command: Option<String>,

    /// Command to run before typing output (e.g., compositor submap switch)
    /// Useful for blocking modifier keys at the compositor level
    #[serde(default)]
    pub pre_output_command: Option<String>,

    /// Command to run after typing output (e.g., reset compositor submap)
    /// Runs even if typing fails, to ensure cleanup
    #[serde(default)]
    pub post_output_command: Option<String>,

    /// Optional post-processing command configuration
    /// Pipes transcribed text through an external command before output
    #[serde(default)]
    pub post_process: Option<PostProcessConfig>,

    /// Keystroke to simulate for paste mode (e.g., "ctrl+v", "shift+insert", "ctrl+shift+v")
    /// Defaults to "ctrl+v" if not specified
    #[serde(default)]
    pub paste_keys: Option<String>,

    /// Keyboard layout for dotool (e.g., "de" for German, "fr" for French)
    /// Required for non-US keyboard layouts when using dotool
    #[serde(default)]
    pub dotool_xkb_layout: Option<String>,

    /// Keyboard layout variant for dotool (e.g., "nodeadkeys")
    #[serde(default)]
    pub dotool_xkb_variant: Option<String>,

    /// Keyboard layout for eitype (e.g., "de" for German, "ru" for Russian).
    /// Passed to eitype as `-l <layout>`. Overrides the system XKB layout
    /// while eitype is typing, then restores it when eitype exits.
    /// Required when the transcribed language does not match the active
    /// system layout (see issue #180).
    #[serde(default)]
    pub eitype_xkb_layout: Option<String>,

    /// Keyboard layout variant for eitype (e.g., "dvorak", "colemak").
    /// Passed to eitype as `--variant <variant>`.
    #[serde(default)]
    pub eitype_xkb_variant: Option<String>,

    /// Mapping from detected language code (two-letter ISO 639-1) to XKB
    /// keyboard layout. When voxtype's transcriber reports a language for the
    /// current transcription and no explicit `eitype_xkb_layout` /
    /// `dotool_xkb_layout` is set, the layout is looked up here.
    ///
    /// Built-in defaults cover the common cases (en→us, ru→ru, de→de, ...);
    /// see [`default_language_to_layout`]. Users can override or extend the
    /// map in config to handle layouts that differ from the language code
    /// (e.g. `pt = "br"` for Brazilian Portuguese).
    ///
    /// Set to an empty map (or remove all entries) to disable automatic
    /// layout selection from the detected language.
    #[serde(default = "default_language_to_layout")]
    pub language_to_layout: std::collections::HashMap<String, String>,

    /// Mapping from detected language code (two-letter ISO 639-1) to XKB
    /// keyboard layout variant. Applied per transcription, after
    /// `language_to_layout`, when no explicit `eitype_xkb_variant` /
    /// `dotool_xkb_variant` is set.
    ///
    /// This is intentionally empty by default. Variants are user-specific
    /// layout choices (for example, Russian phonetic vs standard).
    #[serde(default)]
    pub language_to_variant: std::collections::HashMap<String, String>,

    /// File path for file output mode (required when mode = "file")
    /// Also used as default path for --output-file CLI flag
    #[serde(default)]
    pub file_path: Option<PathBuf>,

    /// File write mode: "overwrite" (default) or "append"
    /// Applies to both config-based file output and --output-file CLI flag
    #[serde(default)]
    pub file_mode: FileMode,

    /// Restore original clipboard content after paste mode completes
    /// Saves clipboard before transcription, restores it after paste keystroke
    #[serde(default)]
    pub restore_clipboard: bool,

    /// Delay after paste before restoring clipboard content (milliseconds)
    /// Allows time for the paste operation to complete
    #[serde(default = "default_restore_clipboard_delay")]
    pub restore_clipboard_delay_ms: u32,

    /// Wait for modifier keys (Ctrl/Alt/Shift/Super) to be released before
    /// typing transcribed text. Prevents the typed letters from combining
    /// with held modifiers and triggering compositor or application
    /// keybindings (e.g. Super+X, Ctrl+W).
    ///
    /// Requires `/dev/input` access (typically `input` group membership).
    /// Silently disabled when access is unavailable; output proceeds as
    /// before in that case.
    #[serde(default = "default_true")]
    pub wait_for_modifier_release: bool,

    /// Maximum time (milliseconds) to wait for modifier keys to be released
    /// before falling back to clipboard output. Prevents a stuck modifier from
    /// indefinitely blocking transcription delivery.
    #[serde(default = "default_modifier_release_timeout_ms")]
    pub modifier_release_timeout_ms: u64,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            mode: OutputMode::default(),
            fallback_to_clipboard: true,
            driver_order: None,
            notification: NotificationConfig::default(),
            type_delay_ms: 0,
            pre_type_delay_ms: 0,
            wtype_delay_ms: 0,
            auto_submit: false,
            append_text: None,
            shift_enter_newlines: false,
            wtype_shift_prefix: false,
            pre_recording_command: None,
            pre_output_command: None,
            post_output_command: None,
            post_process: None,
            paste_keys: None,
            dotool_xkb_layout: None,
            dotool_xkb_variant: None,
            eitype_xkb_layout: None,
            eitype_xkb_variant: None,
            language_to_layout: default_language_to_layout(),
            language_to_variant: HashMap::new(),
            file_path: None,
            file_mode: FileMode::default(),
            restore_clipboard: false,
            restore_clipboard_delay_ms: default_restore_clipboard_delay(),
            wait_for_modifier_release: true,
            modifier_release_timeout_ms: default_modifier_release_timeout_ms(),
        }
    }
}

fn default_modifier_release_timeout_ms() -> u64 {
    750
}

/// Result of applying a per-language XKB layout/variant hint to output config.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppliedLanguageXkbHint {
    /// Layout found in `language_to_layout`, if any.
    pub layout: Option<String>,
    /// Variant found in `language_to_variant`, if any.
    pub variant: Option<String>,
    /// Whether the layout was applied to eitype for this transcription.
    pub eitype_layout_applied: bool,
    /// Whether the layout was applied to dotool for this transcription.
    pub dotool_layout_applied: bool,
    /// Whether the variant was applied to eitype for this transcription.
    pub eitype_variant_applied: bool,
    /// Whether the variant was applied to dotool for this transcription.
    pub dotool_variant_applied: bool,
}

impl AppliedLanguageXkbHint {
    pub fn is_empty(&self) -> bool {
        self.layout.is_none() && self.variant.is_none()
    }
}

/// Built-in mapping from two-letter ISO 639-1 language codes to XKB layout
/// codes. Used when the transcriber reports a detected language and the user
/// has not set an explicit `eitype_xkb_layout` / `dotool_xkb_layout`.
///
/// Covers the most common cases where layout code matches language code
/// (en→us is the notable exception). Users can extend or override this map
/// in config under `[output] language_to_layout`. To disable automatic
/// layout selection entirely, set `language_to_layout = {}` in config.
pub fn default_language_to_layout() -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    // English uses "us" by convention, not "en".
    m.insert("en".to_string(), "us".to_string());
    // Other common languages where layout name matches ISO 639-1.
    for code in [
        "ru", "de", "fr", "es", "it", "pl", "uk", "cs", "sk", "sv", "no", "fi", "da", "nl", "pt",
        "tr", "gr", "hu", "ro", "bg", "hr", "sr", "sl", "lt", "lv", "et", "is", "ca", "eu",
    ] {
        m.insert(code.to_string(), code.to_string());
    }
    // Greek uses "gr" not "el".
    m.insert("el".to_string(), "gr".to_string());
    // Japanese, Korean, Chinese typically need IMEs rather than XKB layouts,
    // but voxtype passes the hint through so users can map them as they wish.
    m.insert("ja".to_string(), "jp".to_string());
    m.insert("ko".to_string(), "kr".to_string());
    m
}

impl OutputConfig {
    /// Apply per-language XKB layout/variant hints to eitype and dotool.
    ///
    /// Explicit driver-specific settings win independently per field:
    /// `dotool_xkb_layout` prevents only the automatic dotool layout, while
    /// `dotool_xkb_variant` prevents only the automatic dotool variant.
    pub fn apply_language_xkb_hint(&mut self, lang: &str) -> AppliedLanguageXkbHint {
        let layout = self.language_to_layout.get(lang).cloned();
        let variant = self.language_to_variant.get(lang).cloned();
        let mut applied = AppliedLanguageXkbHint {
            layout,
            variant,
            ..AppliedLanguageXkbHint::default()
        };

        if let Some(ref layout) = applied.layout {
            if self.eitype_xkb_layout.is_none() {
                self.eitype_xkb_layout = Some(layout.clone());
                applied.eitype_layout_applied = true;
            }
            if self.dotool_xkb_layout.is_none() {
                self.dotool_xkb_layout = Some(layout.clone());
                applied.dotool_layout_applied = true;
            }
        }

        if let Some(ref variant) = applied.variant {
            if self.eitype_xkb_variant.is_none() {
                self.eitype_xkb_variant = Some(variant.clone());
                applied.eitype_variant_applied = true;
            }
            if self.dotool_xkb_variant.is_none() {
                self.dotool_xkb_variant = Some(variant.clone());
                applied.dotool_variant_applied = true;
            }
        }

        applied
    }

    /// Get the effective pre-type delay, handling deprecated wtype_delay_ms
    pub fn effective_pre_type_delay_ms(&self) -> u32 {
        if self.wtype_delay_ms > 0 {
            if self.pre_type_delay_ms > 0 {
                // Both set - prefer new option, warn about deprecated
                tracing::warn!(
                    "Both pre_type_delay_ms and wtype_delay_ms are set. \
                     Using pre_type_delay_ms={}. wtype_delay_ms is deprecated.",
                    self.pre_type_delay_ms
                );
                self.pre_type_delay_ms
            } else {
                // Only deprecated option set - use it with warning
                tracing::warn!("wtype_delay_ms is deprecated, use pre_type_delay_ms instead");
                self.wtype_delay_ms
            }
        } else {
            self.pre_type_delay_ms
        }
    }
}

/// Output mode selection
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputMode {
    /// Simulate keyboard input (requires ydotool)
    #[default]
    Type,
    /// Copy to clipboard (wl-copy on Wayland, xclip on X11)
    Clipboard,
    /// Copy to clipboard then paste with Ctrl+V (requires wl-copy and ydotool)
    Paste,
    /// Write transcription to a file
    File,
}

/// Output driver for typing text
/// Used to specify preferred drivers in the fallback chain
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputDriver {
    /// wtype - Wayland-native via virtual-keyboard protocol, best Unicode/CJK support
    Wtype,
    /// eitype - Wayland via libei/EI protocol, works on GNOME/KDE
    Eitype,
    /// dotool - Works on X11/Wayland/TTY, supports keyboard layouts
    Dotool,
    /// ydotool - Works on X11/Wayland/TTY, requires daemon
    Ydotool,
    /// Clipboard via wl-copy (Wayland)
    Clipboard,
    /// Clipboard via xclip (X11)
    Xclip,
}

impl std::fmt::Display for OutputDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputDriver::Wtype => write!(f, "wtype"),
            OutputDriver::Eitype => write!(f, "eitype"),
            OutputDriver::Dotool => write!(f, "dotool"),
            OutputDriver::Ydotool => write!(f, "ydotool"),
            OutputDriver::Clipboard => write!(f, "clipboard"),
            OutputDriver::Xclip => write!(f, "xclip"),
        }
    }
}

impl std::str::FromStr for OutputDriver {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "wtype" => Ok(OutputDriver::Wtype),
            "eitype" => Ok(OutputDriver::Eitype),
            "dotool" => Ok(OutputDriver::Dotool),
            "ydotool" => Ok(OutputDriver::Ydotool),
            "clipboard" => Ok(OutputDriver::Clipboard),
            "xclip" => Ok(OutputDriver::Xclip),
            _ => Err(format!(
                "Unknown driver '{}'. Valid options: wtype, eitype, dotool, ydotool, clipboard, xclip",
                s
            )),
        }
    }
}

/// File write mode when using file output
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum FileMode {
    /// Overwrite the file on each transcription (default)
    #[default]
    Overwrite,
    /// Append to the file on each transcription
    Append,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_parse_auto_submit() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
            auto_submit = true
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.output.auto_submit);
    }

    #[test]
    fn test_parse_auto_submit_defaults_false() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.output.auto_submit);
    }

    #[test]
    fn test_output_driver_from_str() {
        assert_eq!(
            "wtype".parse::<OutputDriver>().unwrap(),
            OutputDriver::Wtype
        );
        assert_eq!(
            "dotool".parse::<OutputDriver>().unwrap(),
            OutputDriver::Dotool
        );
        assert_eq!(
            "ydotool".parse::<OutputDriver>().unwrap(),
            OutputDriver::Ydotool
        );
        assert_eq!(
            "clipboard".parse::<OutputDriver>().unwrap(),
            OutputDriver::Clipboard
        );
        assert_eq!(
            "xclip".parse::<OutputDriver>().unwrap(),
            OutputDriver::Xclip
        );
        // Case insensitive
        assert_eq!(
            "WTYPE".parse::<OutputDriver>().unwrap(),
            OutputDriver::Wtype
        );
        assert_eq!(
            "Ydotool".parse::<OutputDriver>().unwrap(),
            OutputDriver::Ydotool
        );
        assert_eq!(
            "XCLIP".parse::<OutputDriver>().unwrap(),
            OutputDriver::Xclip
        );
        // Invalid
        assert!("invalid".parse::<OutputDriver>().is_err());
    }

    #[test]
    fn test_output_driver_display() {
        assert_eq!(OutputDriver::Wtype.to_string(), "wtype");
        assert_eq!(OutputDriver::Dotool.to_string(), "dotool");
        assert_eq!(OutputDriver::Ydotool.to_string(), "ydotool");
        assert_eq!(OutputDriver::Clipboard.to_string(), "clipboard");
        assert_eq!(OutputDriver::Xclip.to_string(), "xclip");
    }

    #[test]
    fn test_parse_driver_order_from_toml() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
            driver_order = ["ydotool", "wtype", "clipboard"]
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let driver_order = config.output.driver_order.unwrap();
        assert_eq!(driver_order.len(), 3);
        assert_eq!(driver_order[0], OutputDriver::Ydotool);
        assert_eq!(driver_order[1], OutputDriver::Wtype);
        assert_eq!(driver_order[2], OutputDriver::Clipboard);
    }

    #[test]
    fn test_restore_clipboard_defaults() {
        let config = Config::default();
        assert!(!config.output.restore_clipboard);
        assert_eq!(config.output.restore_clipboard_delay_ms, 200);
    }

    #[test]
    fn test_restore_clipboard_deserialization() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 30

            [whisper]
            model = "base.en"

            [output]
            mode = "paste"
            restore_clipboard = true
            restore_clipboard_delay_ms = 500
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.output.restore_clipboard);
        assert_eq!(config.output.restore_clipboard_delay_ms, 500);
    }

    #[test]
    fn test_restore_clipboard_missing_uses_defaults() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 30

            [whisper]
            model = "base.en"

            [output]
            mode = "paste"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.output.restore_clipboard);
        assert_eq!(config.output.restore_clipboard_delay_ms, 200);
    }

    #[test]
    fn test_parse_driver_order_from_config() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
            driver_order = ["ydotool", "wtype", "clipboard"]
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let driver_order = config.output.driver_order.unwrap();
        assert_eq!(driver_order.len(), 3);
        assert_eq!(driver_order[0], OutputDriver::Ydotool);
        assert_eq!(driver_order[1], OutputDriver::Wtype);
        assert_eq!(driver_order[2], OutputDriver::Clipboard);
    }

    #[test]
    fn test_driver_order_not_set_by_default() {
        let config = Config::default();
        assert!(config.output.driver_order.is_none());
    }

    #[test]
    fn test_parse_config_without_driver_order() {
        // Ensure backwards compatibility - config without driver_order should work
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.output.driver_order.is_none());
    }

    #[test]
    fn test_parse_single_driver_order() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
            driver_order = ["ydotool"]
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let driver_order = config.output.driver_order.unwrap();
        assert_eq!(driver_order.len(), 1);
        assert_eq!(driver_order[0], OutputDriver::Ydotool);
    }

    #[test]
    fn test_default_language_to_layout_common_cases() {
        let map = default_language_to_layout();
        // English maps to "us", the XKB convention.
        assert_eq!(map.get("en"), Some(&"us".to_string()));
        // Russian, German, French, Spanish are direct passthroughs.
        assert_eq!(map.get("ru"), Some(&"ru".to_string()));
        assert_eq!(map.get("de"), Some(&"de".to_string()));
        assert_eq!(map.get("fr"), Some(&"fr".to_string()));
        assert_eq!(map.get("es"), Some(&"es".to_string()));
        // Greek uses "gr", not "el".
        assert_eq!(map.get("el"), Some(&"gr".to_string()));
        // Japanese / Korean map to common XKB names.
        assert_eq!(map.get("ja"), Some(&"jp".to_string()));
        assert_eq!(map.get("ko"), Some(&"kr".to_string()));
    }

    #[test]
    fn test_output_config_default_includes_language_layout_map() {
        let cfg = Config::default();
        assert!(!cfg.output.language_to_layout.is_empty());
        assert_eq!(
            cfg.output.language_to_layout.get("en"),
            Some(&"us".to_string())
        );
        assert!(cfg.output.language_to_variant.is_empty());
        // New eitype layout fields are unset by default; the layout is
        // inferred from the detected language only when both fields are
        // empty (see daemon::handle_transcription_result).
        assert!(cfg.output.eitype_xkb_layout.is_none());
        assert!(cfg.output.eitype_xkb_variant.is_none());
    }

    #[test]
    fn test_parse_eitype_layout_from_toml() {
        let toml_str = r#"
            [hotkey]
            key = "PAUSE"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
            eitype_xkb_layout = "ru"
            eitype_xkb_variant = "phonetic"
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.output.eitype_xkb_layout, Some("ru".to_string()));
        assert_eq!(
            config.output.eitype_xkb_variant,
            Some("phonetic".to_string())
        );
    }

    #[test]
    fn test_parse_language_to_layout_override() {
        // User can override individual mappings (e.g. Brazilian Portuguese
        // typically needs the `br` layout, not `pt`). Providing the field
        // replaces the built-in defaults; users are expected to copy
        // entries they want to keep (documented in CONFIGURATION.md).
        let toml_str = r#"
            [hotkey]
            key = "PAUSE"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [output.language_to_layout]
            pt = "br"
            en = "dvorak"
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.output.language_to_layout.get("pt"),
            Some(&"br".to_string())
        );
        assert_eq!(
            config.output.language_to_layout.get("en"),
            Some(&"dvorak".to_string())
        );
    }

    #[test]
    fn test_parse_language_to_variant() {
        let toml_str = r#"
            [hotkey]
            key = "PAUSE"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = ["en", "ru"]

            [output]
            mode = "type"

            [output.language_to_layout]
            en = "us"
            ru = "ru"

            [output.language_to_variant]
            ru = "phonetic"
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.output.language_to_variant.get("ru"),
            Some(&"phonetic".to_string())
        );
        assert!(!config.output.language_to_variant.contains_key("en"));
    }

    #[test]
    fn test_apply_language_xkb_hint_applies_layout_and_variant() {
        let mut output = Config::default().output;
        output
            .language_to_variant
            .insert("ru".to_string(), "phonetic".to_string());

        let applied = output.apply_language_xkb_hint("ru");

        assert_eq!(applied.layout, Some("ru".to_string()));
        assert_eq!(applied.variant, Some("phonetic".to_string()));
        assert!(applied.eitype_layout_applied);
        assert!(applied.dotool_layout_applied);
        assert!(applied.eitype_variant_applied);
        assert!(applied.dotool_variant_applied);
        assert_eq!(output.eitype_xkb_layout, Some("ru".to_string()));
        assert_eq!(output.dotool_xkb_layout, Some("ru".to_string()));
        assert_eq!(output.eitype_xkb_variant, Some("phonetic".to_string()));
        assert_eq!(output.dotool_xkb_variant, Some("phonetic".to_string()));
    }

    #[test]
    fn test_apply_language_xkb_hint_does_not_leak_variant_between_languages() {
        let mut output = Config::default().output;
        output
            .language_to_variant
            .insert("ru".to_string(), "phonetic".to_string());

        let applied = output.apply_language_xkb_hint("en");

        assert_eq!(applied.layout, Some("us".to_string()));
        assert_eq!(applied.variant, None);
        assert_eq!(output.eitype_xkb_layout, Some("us".to_string()));
        assert_eq!(output.dotool_xkb_layout, Some("us".to_string()));
        assert_eq!(output.eitype_xkb_variant, None);
        assert_eq!(output.dotool_xkb_variant, None);
    }

    #[test]
    fn test_apply_language_xkb_hint_preserves_explicit_variant() {
        let mut output = Config::default().output;
        output.eitype_xkb_variant = Some("explicit-eitype".to_string());
        output.dotool_xkb_variant = Some("explicit-dotool".to_string());
        output
            .language_to_variant
            .insert("ru".to_string(), "phonetic".to_string());

        let applied = output.apply_language_xkb_hint("ru");

        assert_eq!(applied.variant, Some("phonetic".to_string()));
        assert!(!applied.eitype_variant_applied);
        assert!(!applied.dotool_variant_applied);
        assert_eq!(
            output.eitype_xkb_variant,
            Some("explicit-eitype".to_string())
        );
        assert_eq!(
            output.dotool_xkb_variant,
            Some("explicit-dotool".to_string())
        );
    }
}
