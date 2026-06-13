//! `[osd]` configuration block.
//!
//! Parsed from the user's config file alongside the rest of the daemon
//! config; can be overridden via CLI flags or `VOXTYPE_OSD_*` env vars on
//! either OSD binary. The full config layering is wired up in Commit 6.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

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
/// `osd-gtk4`/`osd-native`/`osd-quickshell`), the wrapper falls back to
/// whichever it can find and logs a warning. Default is `Gtk4` because
/// GTK4 ships with most Hyprland setups already (Omarchy pulls it in via
/// swayosd, walker, etc.) so there's no extra runtime cost for the
/// typical user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdFrontend {
    #[default]
    Gtk4,
    Native,
    Quickshell,
}

impl OsdFrontend {
    /// Name of the binary that implements this frontend, suitable for a
    /// PATH lookup or `Command::new`.
    pub fn binary_name(self) -> &'static str {
        match self {
            OsdFrontend::Gtk4 => "voxtype-osd-gtk4",
            OsdFrontend::Native => "voxtype-osd-native",
            OsdFrontend::Quickshell => "voxtype-osd-quickshell",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "gtk4" | "gtk" => Some(OsdFrontend::Gtk4),
            "native" | "wgpu" | "egui" => Some(OsdFrontend::Native),
            "quickshell" | "qml" | "qs" => Some(OsdFrontend::Quickshell),
            _ => None,
        }
    }
}

/// Palette source for Quickshell OSD recipes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdPaletteSource {
    /// Resolve semantic colors from the active Omarchy theme.
    #[default]
    Omarchy,
    /// Use VoxType's built-in fallback palette.
    Fallback,
    /// Use colors provided by the selected OSD package.
    Package,
    /// Use literal colors from the visual recipe.
    Custom,
}

/// Layout preset for the Quickshell OSD host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdLayout {
    #[default]
    Compact,
    Wide,
    Minimal,
    Tile,
    Orb,
    Custom,
}

/// Declarative layer kind for no-code Quickshell OSD recipes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdLayerKind {
    Shadow,
    Background,
    Waveform,
    Bars,
    Pulse,
    Ring,
    Meter,
    Icon,
    Label,
}

/// One declarative visual layer in the Quickshell renderer.
///
/// Tunables are `Option` and skipped from the runtime JSON when unset, so
/// the QML renderer can distinguish "not configured" (apply the layer
/// type's own default) from an explicit value, including explicit zeros.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdLayerConfig {
    /// Layer type. Serialized as `type` in TOML manifests/recipes.
    #[serde(rename = "type")]
    pub kind: OsdLayerKind,
    /// Input signal: peak, rms, vad, state, or none.
    pub source: String,
    /// Semantic color role or literal color. Unset lets each layer type
    /// pick its own default (accent for most, black for shadow).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Secondary color role or literal color for gradients/ticks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_color: Option<String>,
    /// Z-order. Lower layers render first.
    pub order: i32,
    /// Normalized or pixel position hint consumed by QML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    /// Normalized or pixel position hint consumed by QML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
    /// Width hint. Values <= 1.0 are treated as a fraction by QML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// Height hint. Values <= 1.0 are treated as a fraction by QML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    /// Audio visual gain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gain: Option<f32>,
    /// Opacity, 0.0..=1.0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    /// Corner/ring radius hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    /// Initial size multiplier for scalable shapes such as rings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_scale: Option<f32>,
    /// Audio response multiplier for scalable shapes such as rings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_scale: Option<f32>,
    /// Audio response curve; values below 1.0 make low/mid signals more visible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_curve: Option<f32>,
    /// Idle breathing multiplier for scalable shapes such as rings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breath_scale: Option<f32>,
    /// Maximum size multiplier for scalable shapes such as rings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_scale: Option<f32>,
    /// Mirror bars/waveform around the center line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror: Option<bool>,
    /// Animation speed multiplier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
}

impl Default for OsdLayerConfig {
    fn default() -> Self {
        Self {
            kind: OsdLayerKind::Waveform,
            source: "peak".to_string(),
            color: None,
            secondary_color: None,
            order: 0,
            x: None,
            y: None,
            width: None,
            height: None,
            gain: None,
            opacity: None,
            radius: None,
            base_scale: None,
            response_scale: None,
            response_curve: None,
            breath_scale: None,
            max_scale: None,
            mirror: None,
            speed: None,
        }
    }
}

/// Declarative visual recipe for the Quickshell OSD.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdVisualConfig {
    pub layers: Vec<OsdLayerConfig>,
}

impl Default for OsdVisualConfig {
    fn default() -> Self {
        Self {
            layers: default_visual_layers(),
        }
    }
}

fn default_visual_layers() -> Vec<OsdLayerConfig> {
    vec![
        OsdLayerConfig {
            kind: OsdLayerKind::Waveform,
            source: "peak".to_string(),
            color: Some("accent".to_string()),
            order: 10,
            height: Some(0.82),
            mirror: Some(true),
            ..OsdLayerConfig::default()
        },
        OsdLayerConfig {
            kind: OsdLayerKind::Meter,
            source: "peak".to_string(),
            color: Some("success".to_string()),
            secondary_color: Some("foreground".to_string()),
            order: 20,
            y: Some(0.86),
            height: Some(0.14),
            ..OsdLayerConfig::default()
        },
    ]
}

/// Quickshell host-frame styling for no-code recipes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdFrameConfig {
    /// Background color role, literal color, or `none`.
    pub background: String,
    /// Border color role, `state`, literal color, or `none`.
    pub border: String,
    /// Whether the voice-reactive soft frame glow is rendered.
    pub glow: bool,
    /// Whether the orb layout renders its extra outline halo.
    pub halo: bool,
}

impl Default for OsdFrameConfig {
    fn default() -> Self {
        Self {
            background: "background".to_string(),
            border: "state".to_string(),
            glow: true,
            halo: true,
        }
    }
}

/// Manifest for a shareable Quickshell OSD customization package.
///
/// `palette`, `layout`, `frame`, and `visual` are `Option` on purpose:
/// a manifest only overrides the user's `[osd]` config for fields it
/// explicitly sets. An unset field must never reset the user's choice
/// back to a built-in default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdPackageManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub compatibility: Option<String>,
    pub palette: Option<OsdPaletteSource>,
    pub layout: Option<OsdLayout>,
    pub colors: BTreeMap<String, String>,
    pub qml_entry: Option<PathBuf>,
    pub frame: Option<OsdFrameConfig>,
    pub visual: Option<OsdVisualConfig>,
}

impl Default for OsdPackageManifest {
    fn default() -> Self {
        Self {
            name: "custom".to_string(),
            version: "0.1.0".to_string(),
            description: None,
            compatibility: None,
            palette: None,
            layout: None,
            colors: BTreeMap::new(),
            qml_entry: None,
            frame: None,
            visual: None,
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
    /// Quickshell style name, package name, or package path.
    pub style: String,
    /// Palette source for Quickshell OSD recipes.
    ///
    /// `None` means the selected package manifest may choose a palette.
    /// `Some(Omarchy)` is an explicit user choice and overrides manifests.
    pub palette: Option<OsdPaletteSource>,
    /// Layout preset for the Quickshell OSD host.
    pub layout: OsdLayout,
    /// Explicit third-party package path. QML code in this path is trusted.
    pub plugin_path: Option<PathBuf>,
    /// Quickshell host-frame styling for no-code recipes.
    pub frame: OsdFrameConfig,
    /// Declarative no-code visual recipe for the Quickshell OSD.
    pub visual: OsdVisualConfig,
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
            style: "default".to_string(),
            palette: None,
            layout: OsdLayout::default(),
            plugin_path: None,
            frame: OsdFrameConfig::default(),
            visual: OsdVisualConfig::default(),
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
        assert_eq!(c.style, "default");
        assert_eq!(c.palette, None);
        assert_eq!(c.layout, OsdLayout::Compact);
        assert!(c.plugin_path.is_none());
        assert_eq!(c.frame.background, "background");
        assert_eq!(c.frame.border, "state");
        assert!(c.frame.glow);
        assert!(c.frame.halo);
        assert_eq!(c.visual.layers.len(), 2);
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
        assert_eq!(
            OsdFrontend::Quickshell.binary_name(),
            "voxtype-osd-quickshell"
        );
    }

    #[test]
    fn frontend_parse_str_accepts_aliases() {
        assert_eq!(OsdFrontend::parse_str("gtk4"), Some(OsdFrontend::Gtk4));
        assert_eq!(OsdFrontend::parse_str("GTK"), Some(OsdFrontend::Gtk4));
        assert_eq!(OsdFrontend::parse_str("native"), Some(OsdFrontend::Native));
        assert_eq!(OsdFrontend::parse_str("wgpu"), Some(OsdFrontend::Native));
        assert_eq!(OsdFrontend::parse_str("egui"), Some(OsdFrontend::Native));
        assert_eq!(
            OsdFrontend::parse_str("quickshell"),
            Some(OsdFrontend::Quickshell)
        );
        assert_eq!(OsdFrontend::parse_str("qml"), Some(OsdFrontend::Quickshell));
        assert_eq!(OsdFrontend::parse_str("QS"), Some(OsdFrontend::Quickshell));
        assert_eq!(OsdFrontend::parse_str("nope"), None);
    }

    #[test]
    fn frontend_serde_kebab_case() {
        let v: OsdFrontend = serde_json::from_str("\"gtk4\"").unwrap();
        assert_eq!(v, OsdFrontend::Gtk4);
        let v: OsdFrontend = serde_json::from_str("\"native\"").unwrap();
        assert_eq!(v, OsdFrontend::Native);
        let v: OsdFrontend = serde_json::from_str("\"quickshell\"").unwrap();
        assert_eq!(v, OsdFrontend::Quickshell);
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

    #[test]
    fn visual_layer_serde_uses_type_key() {
        let toml_src = r#"
            [[layers]]
            type = "bars"
            source = "rms"
            color = "accent"
            order = 5
        "#;
        let v: OsdVisualConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(v.layers.len(), 1);
        assert_eq!(v.layers[0].kind, OsdLayerKind::Bars);
        assert_eq!(v.layers[0].source, "rms");
        assert_eq!(v.layers[0].order, 5);
    }

    #[test]
    fn visual_layer_accepts_shadow_kind() {
        let toml_src = r#"
            [[layers]]
            type = "shadow"
            source = "peak"
            opacity = 0.8
        "#;
        let v: OsdVisualConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(v.layers.len(), 1);
        assert_eq!(v.layers[0].kind, OsdLayerKind::Shadow);
        assert_eq!(v.layers[0].source, "peak");
        assert!((v.layers[0].opacity.unwrap() - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn visual_layer_distinguishes_explicit_zero_from_unset() {
        let toml_src = r#"
            [[layers]]
            type = "pulse"
            gain = 0.0
            radius = 0.0
        "#;
        let v: OsdVisualConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(v.layers[0].gain, Some(0.0));
        assert_eq!(v.layers[0].radius, Some(0.0));
        assert_eq!(v.layers[0].speed, None);

        // Unset tunables must be absent from the runtime JSON so QML can
        // apply per-layer-type defaults without falsy-zero confusion.
        let json = serde_json::to_value(&v.layers[0]).unwrap();
        assert!(json.get("speed").is_none());
        assert!(json.get("opacity").is_none());
        assert_eq!(json["gain"], 0.0);
    }

    #[test]
    fn frame_config_accepts_none_values() {
        let toml_src = r#"
            [frame]
            background = "none"
            border = "none"
            glow = false
            halo = false
        "#;
        let c: OsdConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(c.frame.background, "none");
        assert_eq!(c.frame.border, "none");
        assert!(!c.frame.glow);
        assert!(!c.frame.halo);
    }

    #[test]
    fn example_recipes_parse_with_current_schema() {
        #[derive(serde::Deserialize)]
        struct Doc {
            osd: OsdConfig,
        }
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/osd-recipes");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "toml") {
                let content = std::fs::read_to_string(&path).unwrap();
                let doc: Doc = toml::from_str(&content)
                    .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));
                assert_eq!(
                    doc.osd.frontend,
                    OsdFrontend::Quickshell,
                    "{}",
                    path.display()
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 4,
            "expected the showcase recipes, found {checked}"
        );
    }

    #[test]
    fn package_manifest_leaves_unset_fields_to_user_config() {
        let manifest: OsdPackageManifest = toml::from_str(
            r#"
                name = "bars-plus"
                version = "1.0.0"
            "#,
        )
        .unwrap();
        assert_eq!(manifest.name, "bars-plus");
        assert_eq!(manifest.palette, None);
        assert_eq!(manifest.layout, None);
        assert!(manifest.colors.is_empty());
        assert!(manifest.frame.is_none());
        assert!(manifest.visual.is_none());
    }
}
