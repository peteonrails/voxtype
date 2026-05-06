//! `[osd]` configuration block.
//!
//! Parsed from the user's config file alongside the rest of the daemon
//! config; can be overridden via CLI flags or `VOXTYPE_OSD_*` env vars on
//! either OSD binary. The full config layering is wired up in Commit 6.

use serde::{Deserialize, Serialize};

/// Position anchor for the OSD surface on the focused output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdPosition {
    #[default]
    BottomCenter,
    TopCenter,
    BottomLeft,
    BottomRight,
    TopLeft,
    TopRight,
}

/// Selects which OSD frontend the `voxtype-osd` wrapper launches.
///
/// The wrapper treats this as a *preference*: if the chosen frontend's
/// binary isn't on PATH (e.g. the user built voxtype with only one of
/// `osd-gtk4`/`osd-native`), the wrapper falls back to whichever it can
/// find and logs a warning. Default is `Gtk4` because GTK4 ships with
/// most Hyprland setups already (Omarchy pulls it in via swayosd, walker,
/// etc.) so there's no extra runtime cost for the typical user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdFrontend {
    #[default]
    Gtk4,
    Native,
}

impl OsdFrontend {
    /// Name of the binary that implements this frontend, suitable for a
    /// PATH lookup or `Command::new`.
    pub fn binary_name(self) -> &'static str {
        match self {
            OsdFrontend::Gtk4 => "voxtype-osd-gtk4",
            OsdFrontend::Native => "voxtype-osd-native",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "gtk4" | "gtk" => Some(OsdFrontend::Gtk4),
            "native" | "wgpu" | "egui" => Some(OsdFrontend::Native),
            _ => None,
        }
    }
}

/// All user-facing OSD options. Defaults match BRIEF.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdConfig {
    /// Run the OSD at all. When `false`, both binaries exit immediately.
    pub enabled: bool,
    /// Surface width in physical pixels.
    pub width_px: u32,
    /// Surface height in physical pixels.
    pub height_px: u32,
    /// Anchor on the focused output.
    pub position: OsdPosition,
    /// Margin from the screen edge in physical pixels. Used for corner
    /// anchors (`top-left`, `bottom-right`, etc.) and as a fallback for
    /// centered anchors. Centered anchors (`bottom-center`, `top-center`)
    /// prefer `top_margin` (fractional) so v0.7.0 ships swayosd-aligned
    /// vertical positioning out of the box; this field still sets the
    /// horizontal margin for corner anchors.
    pub margin_px: u32,
    /// Vertical position of the OSD's top edge as a fraction of the
    /// monitor's height, mirroring `swayosd-server --top-margin`. Default
    /// 0.85 puts the panel just above the bottom of the screen, matching
    /// the swayosd default so the voxtype OSD lands in the same band as
    /// volume/brightness/media-key feedback users are used to. Only
    /// applied when `position` is `bottom-center` or `top-center` —
    /// corner anchors keep using `margin_px`.
    pub top_margin: f32,
    /// Background opacity, 0.0..=1.0.
    pub opacity: f32,
    /// Visible waveform window in seconds (3.0 per BRIEF).
    pub waveform_window_secs: f32,
    /// Held-peak decay rate in dB/sec (6.0 per BRIEF).
    pub peak_decay_db_per_sec: f32,
    /// Visual gain applied to audio samples before drawing the waveform.
    /// Mic-level voice typically peaks at ~0.1..=0.3 of full-scale; gain
    /// scales that up so the envelope fills the available height. 10.0 is
    /// the default; reduce for hot mics, increase for quiet sources.
    pub waveform_gain: f32,
    /// Which OSD frontend the `voxtype-osd` wrapper launches. Defaults to
    /// `Gtk4` since GTK4 ships with most Hyprland setups already.
    pub frontend: OsdFrontend,
}

impl Default for OsdConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            width_px: 400,
            height_px: 48,
            position: OsdPosition::BottomCenter,
            margin_px: 24,
            top_margin: 0.85,
            opacity: 0.95,
            waveform_window_secs: 3.0,
            peak_decay_db_per_sec: 6.0,
            waveform_gain: 10.0,
            frontend: OsdFrontend::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_brief() {
        let c = OsdConfig::default();
        assert!(c.enabled);
        assert_eq!(c.width_px, 400);
        assert_eq!(c.height_px, 48);
        assert_eq!(c.position, OsdPosition::BottomCenter);
        assert_eq!(c.margin_px, 24);
        assert!((c.opacity - 0.95).abs() < 1e-6);
        assert!((c.waveform_window_secs - 3.0).abs() < 1e-6);
        assert!((c.peak_decay_db_per_sec - 6.0).abs() < 1e-6);
        assert!((c.waveform_gain - 10.0).abs() < 1e-6);
    }

    #[test]
    fn position_serde_kebab_case() {
        let v: OsdPosition = serde_json::from_str("\"bottom-center\"").unwrap();
        assert_eq!(v, OsdPosition::BottomCenter);
        let v: OsdPosition = serde_json::from_str("\"top-right\"").unwrap();
        assert_eq!(v, OsdPosition::TopRight);
    }

    #[test]
    fn frontend_default_is_gtk4() {
        assert_eq!(OsdFrontend::default(), OsdFrontend::Gtk4);
        assert_eq!(OsdConfig::default().frontend, OsdFrontend::Gtk4);
    }

    #[test]
    fn frontend_binary_names() {
        assert_eq!(OsdFrontend::Gtk4.binary_name(), "voxtype-osd-gtk4");
        assert_eq!(OsdFrontend::Native.binary_name(), "voxtype-osd-native");
    }

    #[test]
    fn frontend_parse_str_accepts_aliases() {
        assert_eq!(OsdFrontend::parse_str("gtk4"), Some(OsdFrontend::Gtk4));
        assert_eq!(OsdFrontend::parse_str("GTK"), Some(OsdFrontend::Gtk4));
        assert_eq!(OsdFrontend::parse_str("native"), Some(OsdFrontend::Native));
        assert_eq!(OsdFrontend::parse_str("wgpu"), Some(OsdFrontend::Native));
        assert_eq!(OsdFrontend::parse_str("egui"), Some(OsdFrontend::Native));
        assert_eq!(OsdFrontend::parse_str("nope"), None);
    }

    #[test]
    fn frontend_serde_kebab_case() {
        let v: OsdFrontend = serde_json::from_str("\"gtk4\"").unwrap();
        assert_eq!(v, OsdFrontend::Gtk4);
        let v: OsdFrontend = serde_json::from_str("\"native\"").unwrap();
        assert_eq!(v, OsdFrontend::Native);
    }

    #[test]
    fn config_partial_toml_uses_defaults() {
        let toml_src = "width_px = 800\n";
        let c: OsdConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(c.width_px, 800);
        // All other fields default
        assert_eq!(c.height_px, 48);
        assert!(c.enabled);
    }
}
