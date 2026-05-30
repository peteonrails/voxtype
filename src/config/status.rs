//! Status display configuration (Waybar/tray icons).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Status display configuration for Waybar/tray integrations
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatusConfig {
    /// Icon theme: "emoji", "nerd-font", "omarchy", "minimal", or path to custom theme
    #[serde(default = "default_icon_theme")]
    pub icon_theme: String,

    /// Per-state icon overrides (optional, takes precedence over theme)
    #[serde(default)]
    pub icons: StatusIconOverrides,
}

fn default_icon_theme() -> String {
    "emoji".to_string()
}

impl Default for StatusConfig {
    fn default() -> Self {
        Self {
            icon_theme: default_icon_theme(),
            icons: StatusIconOverrides::default(),
        }
    }
}

/// Per-state icon overrides for status display
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct StatusIconOverrides {
    pub idle: Option<String>,
    pub recording: Option<String>,
    pub streaming: Option<String>,
    pub transcribing: Option<String>,
    pub stopped: Option<String>,
}

/// Resolved icons for each state (after applying theme + overrides)
#[derive(Debug, Clone)]
pub struct ResolvedIcons {
    pub idle: String,
    pub recording: String,
    pub streaming: String,
    pub transcribing: String,
    pub stopped: String,
}

impl StatusConfig {
    /// Resolve icons by loading theme and applying any overrides
    pub fn resolve_icons(&self) -> ResolvedIcons {
        // Start with theme defaults
        let mut icons = load_icon_theme(&self.icon_theme);

        // Apply per-state overrides
        if let Some(ref icon) = self.icons.idle {
            icons.idle = icon.clone();
        }
        if let Some(ref icon) = self.icons.recording {
            icons.recording = icon.clone();
        }
        if let Some(ref icon) = self.icons.streaming {
            icons.streaming = icon.clone();
        }
        if let Some(ref icon) = self.icons.transcribing {
            icons.transcribing = icon.clone();
        }
        if let Some(ref icon) = self.icons.stopped {
            icons.stopped = icon.clone();
        }

        icons
    }
}

/// Load an icon theme by name or from a custom file path
pub(super) fn load_icon_theme(theme: &str) -> ResolvedIcons {
    match theme {
        "emoji" => ResolvedIcons {
            idle: "🎙️".to_string(),
            recording: "🎤".to_string(),
            streaming: "📡".to_string(), // satellite antenna — live broadcast
            transcribing: "⏳".to_string(),
            stopped: "".to_string(),
        },
        "nerd-font" => ResolvedIcons {
            // Nerd Font icons: microphone, circle, spinner, microphone-slash
            idle: "\u{f130}".to_string(),         // nf-fa-microphone
            recording: "\u{f111}".to_string(),    // nf-fa-circle (filled)
            streaming: "\u{f519}".to_string(),    // nf-fa-broadcast_tower
            transcribing: "\u{f110}".to_string(), // nf-fa-spinner
            stopped: "\u{f131}".to_string(),      // nf-fa-microphone_slash
        },
        "omarchy" => ResolvedIcons {
            // Material Design icons matching Omarchy waybar config
            idle: "\u{ec12}".to_string(), // nf-md-microphone_outline
            recording: "\u{f036c}".to_string(), // nf-md-microphone
            streaming: "\u{f048b}".to_string(), // nf-md-access_point — broadcasting/live
            transcribing: "\u{f051f}".to_string(), // nf-md-timer_sand
            stopped: "\u{ec12}".to_string(), // nf-md-microphone_outline
        },
        "minimal" => ResolvedIcons {
            idle: "○".to_string(),
            recording: "●".to_string(),
            streaming: "⊙".to_string(), // U+2299 circled dot — active/live
            transcribing: "◐".to_string(),
            stopped: "×".to_string(),
        },
        "material" => ResolvedIcons {
            // Material Design Icons (requires MDI font)
            idle: "\u{f036c}".to_string(),         // mdi-microphone
            recording: "\u{f040a}".to_string(),    // mdi-record
            streaming: "\u{f048b}".to_string(),    // mdi-access-point
            transcribing: "\u{f04ce}".to_string(), // mdi-sync
            stopped: "\u{f036d}".to_string(),      // mdi-microphone-off
        },
        "phosphor" => ResolvedIcons {
            // Phosphor Icons (requires Phosphor font)
            idle: "\u{e43a}".to_string(),         // ph-microphone
            recording: "\u{e438}".to_string(),    // ph-record
            streaming: "\u{e7ee}".to_string(),    // ph-broadcast
            transcribing: "\u{e225}".to_string(), // ph-circle-notch (spinner)
            stopped: "\u{e43b}".to_string(),      // ph-microphone-slash
        },
        "codicons" => ResolvedIcons {
            // VS Code Codicons (requires Codicons font)
            idle: "\u{eb51}".to_string(),         // codicon-mic
            recording: "\u{ebfc}".to_string(),    // codicon-record
            streaming: "\u{ebba}".to_string(),    // codicon-radio-tower
            transcribing: "\u{eb4c}".to_string(), // codicon-sync
            stopped: "\u{eb52}".to_string(),      // codicon-mute
        },
        "text" => ResolvedIcons {
            // Plain text labels (no special fonts required)
            idle: "[MIC]".to_string(),
            recording: "[REC]".to_string(),
            streaming: "[LIVE]".to_string(),
            transcribing: "[...]".to_string(),
            stopped: "[OFF]".to_string(),
        },
        "dots" => ResolvedIcons {
            // Unicode geometric shapes (no special fonts required)
            idle: "◯".to_string(),         // U+25EF white circle
            recording: "⬤".to_string(),    // U+2B24 black large circle
            streaming: "⊙".to_string(),    // U+2299 circled dot operator
            transcribing: "◔".to_string(), // U+25D4 circle with upper right quadrant black
            stopped: "◌".to_string(),      // U+25CC dotted circle
        },
        "arrows" => ResolvedIcons {
            // Media player style (no special fonts required)
            idle: "▶".to_string(),         // U+25B6 play
            recording: "●".to_string(),    // U+25CF black circle
            streaming: "⇉".to_string(),    // U+21C9 paired rightward arrows — flow
            transcribing: "↻".to_string(), // U+21BB clockwise arrow
            stopped: "■".to_string(),      // U+25A0 black square
        },
        path => load_custom_icon_theme(path).unwrap_or_else(|e| {
            tracing::warn!(
                "Failed to load custom icon theme '{}': {}, using emoji",
                path,
                e
            );
            load_icon_theme("emoji")
        }),
    }
}

/// Load a custom icon theme from a TOML file
fn load_custom_icon_theme(path: &str) -> Result<ResolvedIcons, String> {
    let path = PathBuf::from(path);
    if !path.exists() {
        return Err(format!("Theme file not found: {}", path.display()));
    }

    let contents =
        std::fs::read_to_string(&path).map_err(|e| format!("Failed to read theme file: {}", e))?;

    #[derive(Deserialize)]
    struct ThemeFile {
        idle: Option<String>,
        recording: Option<String>,
        streaming: Option<String>,
        transcribing: Option<String>,
        stopped: Option<String>,
    }

    let theme: ThemeFile =
        toml::from_str(&contents).map_err(|e| format!("Invalid theme file: {}", e))?;

    // Start with emoji defaults, override with file values
    let base = load_icon_theme("emoji");
    Ok(ResolvedIcons {
        idle: theme.idle.unwrap_or(base.idle),
        recording: theme.recording.unwrap_or(base.recording),
        streaming: theme.streaming.unwrap_or(base.streaming),
        transcribing: theme.transcribing.unwrap_or(base.transcribing),
        stopped: theme.stopped.unwrap_or(base.stopped),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_builtin_icon_themes() {
        // Test all built-in themes load correctly
        let themes = [
            "emoji",
            "nerd-font",
            "material",
            "phosphor",
            "codicons",
            "omarchy",
            "minimal",
            "dots",
            "arrows",
            "text",
        ];

        for theme in themes {
            let icons = load_icon_theme(theme);
            assert!(
                !icons.idle.is_empty() || theme == "emoji",
                "Theme {} should have idle icon",
                theme
            );
            assert!(
                !icons.recording.is_empty(),
                "Theme {} should have recording icon",
                theme
            );
            assert!(
                !icons.transcribing.is_empty(),
                "Theme {} should have transcribing icon",
                theme
            );
            // stopped can be empty for some themes
        }
    }

    #[test]
    fn test_emoji_theme_icons() {
        let icons = load_icon_theme("emoji");
        assert!(icons.idle.contains("🎙"));
        assert!(icons.recording.contains("🎤"));
        assert!(icons.transcribing.contains("⏳"));
        assert!(icons.stopped.is_empty());
    }

    #[test]
    fn test_text_theme_icons() {
        let icons = load_icon_theme("text");
        assert_eq!(icons.idle, "[MIC]");
        assert_eq!(icons.recording, "[REC]");
        assert_eq!(icons.transcribing, "[...]");
        assert_eq!(icons.stopped, "[OFF]");
    }

    #[test]
    fn test_minimal_theme_icons() {
        let icons = load_icon_theme("minimal");
        assert_eq!(icons.idle, "○");
        assert_eq!(icons.recording, "●");
        assert_eq!(icons.transcribing, "◐");
        assert_eq!(icons.stopped, "×");
    }

    #[test]
    fn test_status_config_default() {
        let status = StatusConfig::default();
        assert_eq!(status.icon_theme, "emoji");
        assert!(status.icons.idle.is_none());
        assert!(status.icons.recording.is_none());
    }

    #[test]
    fn test_status_config_resolve_icons() {
        let status = StatusConfig {
            icon_theme: "text".to_string(),
            icons: StatusIconOverrides::default(),
        };
        let icons = status.resolve_icons();
        assert_eq!(icons.idle, "[MIC]");
        assert_eq!(icons.recording, "[REC]");
    }

    #[test]
    fn test_status_config_icon_overrides() {
        let status = StatusConfig {
            icon_theme: "emoji".to_string(),
            icons: StatusIconOverrides {
                idle: None,
                recording: Some("🔴".to_string()),
                streaming: None,
                transcribing: None,
                stopped: Some("⚫".to_string()),
            },
        };
        let icons = status.resolve_icons();
        // idle should be from emoji theme
        assert!(icons.idle.contains("🎙"));
        // recording should be overridden
        assert_eq!(icons.recording, "🔴");
        // transcribing should be from emoji theme
        assert!(icons.transcribing.contains("⏳"));
        // stopped should be overridden
        assert_eq!(icons.stopped, "⚫");
    }

    #[test]
    fn test_parse_status_config_from_toml() {
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

            [status]
            icon_theme = "nerd-font"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.status.icon_theme, "nerd-font");
    }

    #[test]
    fn test_parse_status_icon_overrides_from_toml() {
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

            [status]
            icon_theme = "emoji"

            [status.icons]
            recording = "🔴"
            stopped = "⚫"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.status.icon_theme, "emoji");
        assert!(config.status.icons.idle.is_none());
        assert_eq!(config.status.icons.recording, Some("🔴".to_string()));
        assert!(config.status.icons.transcribing.is_none());
        assert_eq!(config.status.icons.stopped, Some("⚫".to_string()));

        let icons = config.status.resolve_icons();
        assert_eq!(icons.recording, "🔴");
        assert_eq!(icons.stopped, "⚫");
    }

    #[test]
    fn test_invalid_theme_falls_back_to_emoji() {
        // Non-existent file path should fall back to emoji
        let icons = load_icon_theme("/nonexistent/path/theme.toml");
        assert!(icons.idle.contains("🎙"));
    }

    #[test]
    fn test_custom_theme_file() {
        use std::io::Write;

        // Create a temporary theme file
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"
            idle = "IDLE"
            recording = "REC"
            transcribing = "BUSY"
            stopped = "OFF"
        "#
        )
        .unwrap();

        let icons = load_icon_theme(temp_file.path().to_str().unwrap());
        assert_eq!(icons.idle, "IDLE");
        assert_eq!(icons.recording, "REC");
        assert_eq!(icons.transcribing, "BUSY");
        assert_eq!(icons.stopped, "OFF");
    }

    #[test]
    fn test_custom_theme_file_partial() {
        use std::io::Write;

        // Create a theme file with only some icons (others should default to emoji)
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"
            recording = "🔴"
        "#
        )
        .unwrap();

        let icons = load_icon_theme(temp_file.path().to_str().unwrap());
        // Only recording is overridden, others fall back to emoji
        assert!(icons.idle.contains("🎙"));
        assert_eq!(icons.recording, "🔴");
        assert!(icons.transcribing.contains("⏳"));
    }
}
