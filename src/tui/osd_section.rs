//! On-Screen Display (OSD) configuration section.
//!
//! Surfaces the scalar fields on `[osd]` from `OsdConfig` (src/osd/config.rs)
//! so users can tune the floating waveform/level surface without leaving the
//! TUI. The `[osd.frame]` and `[osd.visual]` recipe tables stay config-file
//! (and, for frame, `voxtype config set`) territory; the guard test at the
//! bottom pins which schema keys are deliberately absent here. The OSD binaries (voxtype-osd, voxtype-osd-gtk4, voxtype-osd-native)
//! re-read this table at launch — the daemon doesn't load it, so a save
//! here only takes effect for OSD restarts, not the next dictation.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FeedbackLevel, FormRowSpec, TextInput, TextInputResult};
use super::config_editor::{ConfigEditor, EditorError};
use super::theme;
use crate::osd::style::{list_installed_styles, InstalledStyle};

#[derive(Debug, Clone)]
pub struct OsdState {
    pub enabled: bool,
    pub frontend: String,
    pub style: String,
    /// Styles discovered on disk at section load, cycled by the Style field.
    pub styles: Vec<InstalledStyle>,
    /// Development package directory; empty string means unset.
    pub plugin_path: String,
    pub palette: String,
    pub layout: String,
    pub position: String,
    pub width_px: i64,
    pub height_px: i64,
    pub margin_px: i64,
    pub top_margin: f64,
    pub opacity: f64,
    pub waveform_window_secs: f64,
    pub peak_decay_db_per_sec: f64,
    pub waveform_gain: f64,
    pub field: Field,
    pub editing: Option<TextEdit>,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
    /// True when neither voxtype-osd-gtk4 nor voxtype-osd-native is on
    /// PATH. The config-form still works (you can edit the table), but the
    /// daemon has nothing to launch and the OSD won't render. Surfaced as
    /// a yellow banner above the form so users don't save in vain.
    pub binary_missing: bool,
}

#[derive(Debug, Clone)]
pub struct TextEdit {
    pub field: Field,
    pub input: TextInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Enabled,
    Frontend,
    Style,
    PluginPath,
    Palette,
    Layout,
    Position,
    WidthPx,
    HeightPx,
    MarginPx,
    TopMargin,
    Opacity,
    WaveformWindowSecs,
    PeakDecayDbPerSec,
    WaveformGain,
}

impl Field {
    const ALL: &'static [Field] = &[
        Field::Enabled,
        Field::Frontend,
        Field::Style,
        Field::PluginPath,
        Field::Palette,
        Field::Layout,
        Field::Position,
        Field::WidthPx,
        Field::HeightPx,
        Field::MarginPx,
        Field::TopMargin,
        Field::Opacity,
        Field::WaveformWindowSecs,
        Field::PeakDecayDbPerSec,
        Field::WaveformGain,
    ];
}

const TABLE: &str = "osd";

const FRONTEND_CHOICES: &[&str] = &["gtk4", "native", "quickshell"];
const PALETTE_CHOICES: &[&str] = &["auto", "omarchy", "fallback", "package", "custom"];
const LAYOUT_CHOICES: &[&str] = &["compact", "wide", "minimal", "tile", "orb", "custom"];
const POSITION_CHOICES: &[&str] = &[
    "bottom-center",
    "top-center",
    "bottom-left",
    "bottom-right",
    "top-left",
    "top-right",
];

// Preset cycles for numeric fields. Picked to cover the common range
// without forcing inline text edit; users who need a value off the cycle
// list can still hand-edit ~/.config/voxtype/config.toml.
const WIDTH_CHOICES: &[i64] = &[200, 300, 400, 500, 600, 800, 1000];
const HEIGHT_CHOICES: &[i64] = &[32, 40, 48, 56, 64, 80, 96];
const MARGIN_CHOICES: &[i64] = &[0, 8, 16, 24, 32, 48, 64];
// Mirrors swayosd's `--top-margin` semantics. Default 0.85 matches the
// position users already see for volume/brightness panels.
const TOP_MARGIN_CHOICES: &[f64] = &[0.50, 0.60, 0.70, 0.75, 0.80, 0.85, 0.90, 0.95];
const OPACITY_CHOICES: &[f64] = &[0.5, 0.6, 0.7, 0.8, 0.85, 0.9, 0.95, 1.0];
const WAVEFORM_SECS_CHOICES: &[f64] = &[1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0];
const PEAK_DECAY_CHOICES: &[f64] = &[1.0, 2.0, 4.0, 6.0, 8.0, 12.0, 20.0];
const GAIN_CHOICES: &[f64] = &[1.0, 2.0, 4.0, 6.0, 8.0, 10.0, 15.0, 20.0];

impl OsdState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            enabled: ed.get_bool(TABLE, "enabled").unwrap_or(true),
            frontend: ed
                .get_string(TABLE, "frontend")
                .unwrap_or_else(|| "gtk4".to_string()),
            style: ed
                .get_string(TABLE, "style")
                .unwrap_or_else(|| "default".to_string()),
            styles: list_installed_styles(),
            plugin_path: ed.get_string(TABLE, "plugin_path").unwrap_or_default(),
            palette: ed
                .get_string(TABLE, "palette")
                .unwrap_or_else(|| "auto".to_string()),
            layout: ed
                .get_string(TABLE, "layout")
                .unwrap_or_else(|| "compact".to_string()),
            position: ed
                .get_string(TABLE, "position")
                .unwrap_or_else(|| "bottom-center".to_string()),
            width_px: ed.get_int(TABLE, "width_px").unwrap_or(400),
            height_px: ed.get_int(TABLE, "height_px").unwrap_or(48),
            margin_px: ed.get_int(TABLE, "margin_px").unwrap_or(24),
            top_margin: ed.get_float(TABLE, "top_margin").unwrap_or(0.85),
            opacity: ed.get_float(TABLE, "opacity").unwrap_or(0.95),
            waveform_window_secs: ed.get_float(TABLE, "waveform_window_secs").unwrap_or(3.0),
            peak_decay_db_per_sec: ed.get_float(TABLE, "peak_decay_db_per_sec").unwrap_or(6.0),
            waveform_gain: ed.get_float(TABLE, "waveform_gain").unwrap_or(10.0),
            field: Field::Enabled,
            editing: None,
            feedback: None,
            dirty_since_load: false,
            binary_missing: !osd_binary_available(),
        })
    }

    pub fn save(&mut self) -> Action {
        let mut ed = match ConfigEditor::load() {
            Ok(e) => e,
            Err(e) => {
                self.feedback = Some((
                    FeedbackLevel::Err,
                    format!("Couldn't read the config: {}", e),
                ));
                return Action::None;
            }
        };
        ed.set_bool(TABLE, "enabled", self.enabled);
        ed.set_string(TABLE, "frontend", &self.frontend);
        ed.set_string(TABLE, "style", &self.style);
        if self.plugin_path.is_empty() {
            ed.unset(TABLE, "plugin_path");
        } else {
            ed.set_string(TABLE, "plugin_path", &self.plugin_path);
        }
        if self.palette == "auto" {
            ed.unset(TABLE, "palette");
        } else {
            ed.set_string(TABLE, "palette", &self.palette);
        }
        ed.set_string(TABLE, "layout", &self.layout);
        ed.set_string(TABLE, "position", &self.position);
        ed.set_int(TABLE, "width_px", self.width_px);
        ed.set_int(TABLE, "height_px", self.height_px);
        ed.set_int(TABLE, "margin_px", self.margin_px);
        ed.set_float(TABLE, "top_margin", self.top_margin);
        ed.set_float(TABLE, "opacity", self.opacity);
        ed.set_float(TABLE, "waveform_window_secs", self.waveform_window_secs);
        ed.set_float(TABLE, "peak_decay_db_per_sec", self.peak_decay_db_per_sec);
        ed.set_float(TABLE, "waveform_gain", self.waveform_gain);

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.feedback = Some((
                    FeedbackLevel::Ok,
                    format!("Saved to {}", ed.path().display()),
                ));
            }
            Err(e) => self.feedback = Some((FeedbackLevel::Err, format!("Not saved: {}", e))),
        }
        Action::None
    }

    pub fn reset(&mut self) {
        if let Ok(fresh) = Self::load() {
            let field = self.field;
            *self = fresh;
            self.field = field;
            self.feedback = Some((FeedbackLevel::Ok, "Put back the saved settings".to_string()));
        }
    }

    fn move_field(&mut self, delta: i32) {
        let len = Field::ALL.len() as i32;
        let cur = Field::ALL
            .iter()
            .position(|f| *f == self.field)
            .unwrap_or(0) as i32;
        self.field = Field::ALL[((cur + delta).rem_euclid(len)) as usize];
    }

    fn cycle(&mut self, delta: i32) {
        match self.field {
            Field::Enabled => self.enabled = !self.enabled,
            Field::Frontend => self.frontend = cycle_str(FRONTEND_CHOICES, &self.frontend, delta),
            Field::Style => {
                // Only cycle between discovered styles. A literal package
                // path isn't in the list, and cycling would replace it with
                // no way to get it back; Enter/i edits it instead.
                let names: Vec<&str> = self.styles.iter().map(|s| s.name.as_str()).collect();
                if names.contains(&self.style.as_str()) {
                    self.style = cycle_str(&names, &self.style, delta);
                }
            }
            // Free-text only; Enter/i opens the inline editor instead.
            Field::PluginPath => return,
            Field::Palette => self.palette = cycle_str(PALETTE_CHOICES, &self.palette, delta),
            Field::Layout => self.layout = cycle_str(LAYOUT_CHOICES, &self.layout, delta),
            Field::Position => self.position = cycle_str(POSITION_CHOICES, &self.position, delta),
            Field::WidthPx => self.width_px = cycle_int(WIDTH_CHOICES, self.width_px, delta),
            Field::HeightPx => self.height_px = cycle_int(HEIGHT_CHOICES, self.height_px, delta),
            Field::MarginPx => self.margin_px = cycle_int(MARGIN_CHOICES, self.margin_px, delta),
            Field::TopMargin => {
                self.top_margin = cycle_float(TOP_MARGIN_CHOICES, self.top_margin, delta)
            }
            Field::Opacity => self.opacity = cycle_float(OPACITY_CHOICES, self.opacity, delta),
            Field::WaveformWindowSecs => {
                self.waveform_window_secs =
                    cycle_float(WAVEFORM_SECS_CHOICES, self.waveform_window_secs, delta)
            }
            Field::PeakDecayDbPerSec => {
                self.peak_decay_db_per_sec =
                    cycle_float(PEAK_DECAY_CHOICES, self.peak_decay_db_per_sec, delta)
            }
            Field::WaveformGain => {
                self.waveform_gain = cycle_float(GAIN_CHOICES, self.waveform_gain, delta)
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }

    fn is_text_field(field: Field) -> bool {
        matches!(field, Field::Style | Field::PluginPath)
    }

    fn start_edit_if_text_field(&mut self) -> bool {
        if !Self::is_text_field(self.field) {
            return false;
        }
        let initial = match self.field {
            Field::Style => self.style.clone(),
            Field::PluginPath => self.plugin_path.clone(),
            _ => unreachable!("guarded by is_text_field"),
        };
        self.editing = Some(TextEdit {
            field: self.field,
            input: TextInput::new(initial),
        });
        true
    }

    fn commit_text_edit(&mut self, field: Field, buffer: String) {
        let trimmed = buffer.trim();
        match field {
            Field::Style => {
                self.style = if trimmed.is_empty() {
                    "default".to_string()
                } else {
                    trimmed.to_string()
                };
            }
            // Empty is meaningful here: it unsets plugin_path on save.
            Field::PluginPath => self.plugin_path = trimmed.to_string(),
            _ => return,
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

/// True when at least one OSD frontend binary is on PATH or in the
/// canonical install directories. Either is enough &mdash; the
/// `voxtype-osd` launcher resolves the configured frontend by binary
/// name. Returns false when the user has the configure TUI installed
/// (typically via `voxtype-bin` or a source build) but doesn't have the
/// optional OSD frontends, so the section can flag that saving the OSD
/// config won't actually produce on-screen feedback.
fn osd_binary_available() -> bool {
    use std::path::Path;
    const CANDIDATES: &[&str] = &[
        "/usr/bin/voxtype-osd-gtk4",
        "/usr/bin/voxtype-osd-native",
        "/usr/bin/voxtype-osd-quickshell",
        "/usr/lib/voxtype/voxtype-osd-gtk4",
        "/usr/lib/voxtype/voxtype-osd-native",
        "/usr/lib/voxtype/voxtype-osd-quickshell",
        "/usr/local/bin/voxtype-osd-gtk4",
        "/usr/local/bin/voxtype-osd-native",
        "/usr/local/bin/voxtype-osd-quickshell",
    ];
    if CANDIDATES.iter().any(|p| Path::new(p).exists()) {
        return true;
    }
    // Fall back to a PATH search for environments that install the
    // launchers somewhere unusual (Nix, Homebrew on macOS, dev shells).
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            if dir.is_empty() {
                continue;
            }
            if Path::new(dir).join("voxtype-osd-gtk4").exists()
                || Path::new(dir).join("voxtype-osd-native").exists()
                || Path::new(dir).join("voxtype-osd-quickshell").exists()
            {
                return true;
            }
        }
    }
    false
}

fn cycle_str(choices: &[&str], current: &str, delta: i32) -> String {
    let idx = choices
        .iter()
        .position(|c| *c == current)
        .map(|i| i as i32)
        .unwrap_or(-1);
    let n = (idx + delta).rem_euclid(choices.len() as i32);
    choices[n as usize].to_string()
}

fn cycle_int(choices: &[i64], current: i64, delta: i32) -> i64 {
    let idx = choices
        .iter()
        .position(|c| *c == current)
        .map(|i| i as i32)
        .unwrap_or(-1);
    let n = (idx + delta).rem_euclid(choices.len() as i32);
    choices[n as usize]
}

fn cycle_float(choices: &[f64], current: f64, delta: i32) -> f64 {
    // Tolerate small float drift from prior writes by comparing within an
    // epsilon — `0.949999...` should still snap to the `0.95` slot.
    let idx = choices
        .iter()
        .position(|c| (c - current).abs() < 1e-3)
        .map(|i| i as i32)
        .unwrap_or(-1);
    let n = (idx + delta).rem_euclid(choices.len() as i32);
    choices[n as usize]
}

/// `bottom-center` reads as `Bottom center`, `gtk4` as `GTK 4`.
fn words(value: &str) -> String {
    match value {
        "gtk4" => return "GTK 4".to_string(),
        "auto" => return "Automatic".to_string(),
        _ => {}
    }
    let spaced = value.replace('-', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => spaced,
    }
}

/// The page's rows, from its in-memory settings. Everything after the
/// switch is dimmed while the display is off.
pub fn rows(state: &OsdState) -> Vec<FormRowSpec> {
    let off = !state.enabled;
    let frontends: Vec<String> = FRONTEND_CHOICES.iter().map(|c| words(c)).collect();
    vec![
        FormRowSpec::new(
            state.field == Field::Enabled,
            "On-screen display",
            yesno(state.enabled),
        ),
        FormRowSpec::new(
            state.field == Field::Frontend,
            "Frontend",
            words(&state.frontend),
        )
        .choices(&frontends)
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::Style,
            "Style",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::Style => e.input.caret_string(),
                _ => state.style.clone(),
            },
        )
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::PluginPath,
            "Plugin path",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::PluginPath => e.input.caret_string(),
                _ if state.plugin_path.is_empty() => common::NOT_SET.to_string(),
                _ => state.plugin_path.clone(),
            },
        )
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::Palette,
            "Palette",
            words(&state.palette),
        )
        .dimmed(off),
        FormRowSpec::new(state.field == Field::Layout, "Layout", words(&state.layout)).dimmed(off),
        FormRowSpec::new(
            state.field == Field::Position,
            "Position",
            words(&state.position),
        )
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::WidthPx,
            "Width",
            format!("{} px", state.width_px),
        )
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::HeightPx,
            "Height",
            format!("{} px", state.height_px),
        )
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::MarginPx,
            "Margin",
            format!("{} px", state.margin_px),
        )
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::TopMargin,
            "Distance from top",
            format!("{:.0}% of the screen", state.top_margin * 100.0),
        )
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::Opacity,
            "Opacity",
            format!("{:.0}%", state.opacity * 100.0),
        )
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::WaveformWindowSecs,
            "Waveform length",
            format!("{:.1} s", state.waveform_window_secs),
        )
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::PeakDecayDbPerSec,
            "Peak fall speed",
            format!("{:.1} dB/s", state.peak_decay_db_per_sec),
        )
        .dimmed(off),
        FormRowSpec::new(
            state.field == Field::WaveformGain,
            "Waveform gain",
            format!("{:.1}×", state.waveform_gain),
        )
        .dimmed(off),
    ]
}

/// Put the cursor on the row labelled `label`, for Basics.
pub fn focus_label(state: &mut OsdState, label: &str) {
    if let Some(i) = rows(state).iter().position(|r| r.label == label) {
        state.field = Field::ALL[i];
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.osd else {
        common::render_load_error(f, area, app);
        return;
    };
    // Saving works without the display programs, but nothing would show,
    // so say so before the first edit.
    let feedback_pair = match state.feedback.as_ref() {
        Some((lvl, msg)) => Some((*lvl, msg.as_str())),
        None if state.binary_missing && state.enabled => Some((
            FeedbackLevel::Warn,
            "No on-screen display program is installed, so nothing will show",
        )),
        None => None,
    };
    common::render_form_with_guidance(
        f,
        area,
        app,
        feedback_pair,
        &rows(state),
        guidance_for_field(state),
    );
}

fn yesno(b: bool) -> String {
    common::on_off(b)
}

fn dim<'a>(text: &'a str) -> Line<'a> {
    Line::from(Span::styled(text, theme::muted()))
}

fn dim_owned(text: String) -> Line<'static> {
    Line::from(Span::styled(text, theme::muted()))
}

fn guidance_for_field(state: &OsdState) -> Vec<Line<'_>> {
    let text = |s: &'static str| Line::from(s);
    match state.field {
        Field::Enabled => vec![
            text("A small overlay with a live waveform while voxtype records."),
            dim("It shows while recording and transcribing, and hides when idle."),
        ],
        Field::Frontend => vec![
            text(
                "The program that draws it. GTK 4 is the default. Native avoids GTK. \
                 Quickshell supports style packages and the Omarchy palette.",
            ),
            dim("If the chosen one isn't installed, voxtype uses another it finds."),
        ],
        Field::Style => {
            let mut lines = vec![text(
                "Quickshell only. ←→ goes through the installed styles; Enter types \
                 a package name or folder.",
            )];
            if let Some(desc) = state
                .styles
                .iter()
                .find(|s| s.name == state.style)
                .and_then(|s| s.description.clone())
            {
                lines.push(dim_owned(desc));
            }
            lines.push(dim(
                "Styles live in ~/.config/voxtype/osd/. List them with: voxtype info styles",
            ));
            lines
        }
        Field::PluginPath => vec![
            text(
                "Quickshell only. A style package folder you are working on; it wins \
                 over Style so edits show without reinstalling.",
            ),
            dim("Leave empty to use Style."),
        ],
        Field::Palette => vec![
            text(
                "Quickshell only. Where colors come from: the style, the Omarchy \
                 theme, voxtype's own dark palette, or the recipe.",
            ),
            dim("Automatic lets the style choose, else Omarchy."),
        ],
        Field::Layout => vec![text(
            "Quickshell only. The card's shape: compact, wide, minimal, tile, orb, \
             or custom for package layouts.",
        )],
        Field::Position => vec![text(
            "Where on the focused screen it appears. Corners keep Margin from the edges.",
        )],
        Field::WidthPx => vec![text("Wider shows more of the recent waveform.")],
        Field::HeightPx => vec![text("Taller gives the level meter more range.")],
        Field::MarginPx => vec![text(
            "Space from the screen edge, for the corner positions.",
        )],
        Field::TopMargin => vec![
            text(
                "How far down the screen the centered positions sit, matching the \
                 volume and brightness popups.",
            ),
            dim("85% is the default."),
        ],
        Field::Opacity => vec![text(
            "The background's opacity. The waveform is always solid.",
        )],
        Field::WaveformWindowSecs => {
            vec![text("How many seconds of recent audio the waveform shows.")]
        }
        Field::PeakDecayDbPerSec => vec![text(
            "How fast the peak marker falls after a loud sound. Lower holds peaks longer.",
        )],
        Field::WaveformGain => vec![text(
            "Scales the waveform to fill the height. Lower it for a loud microphone. \
             It doesn't change what is transcribed.",
        )],
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.osd.as_mut() {
        Some(s) => s,
        None => return Action::None,
    };

    if state.editing.is_some() {
        return handle_edit_key(state, key);
    }

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.move_field(-1);
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.move_field(1);
            Action::None
        }
        KeyCode::Left | KeyCode::Char('h') => {
            state.cycle(-1);
            Action::None
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
            state.cycle(1);
            Action::None
        }
        KeyCode::Enter | KeyCode::Char('i') => {
            state.start_edit_if_text_field();
            Action::None
        }
        KeyCode::Char('s') => state.save(),
        KeyCode::Char('r') => {
            state.reset();
            Action::None
        }
        _ => Action::None,
    }
}

fn handle_edit_key(state: &mut OsdState, key: KeyEvent) -> Action {
    let Some(editing) = state.editing.as_mut() else {
        return Action::None;
    };
    match editing.input.handle_key(key) {
        TextInputResult::Continue => Action::None,
        TextInputResult::Commit => {
            let buf = editing.input.buffer().to_string();
            let field = editing.field;
            state.editing = None;
            state.commit_text_edit(field, buf);
            Action::None
        }
        TextInputResult::Cancel => {
            state.editing = None;
            Action::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{scalar_keys, CONFIG_KEYS};

    /// The OSD form is a hand-written transcription of the schema's `OSD`
    /// section, so the two drift silently whenever a key is added in one
    /// place and not the other. That is exactly how `osd.plugin_path`
    /// reached a release settable only by hand-editing config.toml.
    ///
    /// If a key genuinely does not belong in the TUI, add it to `NOT_IN_TUI`
    /// with a reason rather than deleting the test.
    #[test]
    fn every_osd_schema_key_has_a_form_field() {
        // The [osd.frame] recipe keys are Quickshell styling detail; the
        // cycle-based form has nowhere sensible to put free-form color
        // strings. They stay reachable via `voxtype config set` and the
        // config file.
        const NOT_IN_TUI: &[&str] = &[
            "osd.frame.background",
            "osd.frame.border",
            "osd.frame.glow",
            "osd.frame.halo",
        ];

        let expected: Vec<&str> = scalar_keys()
            .filter(|s| s.section == "OSD")
            .map(|s| s.key)
            .filter(|k| !NOT_IN_TUI.contains(k))
            .collect();

        // Field -> the schema key it edits. Adding a Field without extending
        // this map fails to compile, which is the point.
        let covered: Vec<&str> = Field::ALL
            .iter()
            .map(|f| match f {
                Field::Enabled => "osd.enabled",
                Field::Frontend => "osd.frontend",
                Field::Style => "osd.style",
                Field::PluginPath => "osd.plugin_path",
                Field::Palette => "osd.palette",
                Field::Layout => "osd.layout",
                Field::Position => "osd.position",
                Field::WidthPx => "osd.width_px",
                Field::HeightPx => "osd.height_px",
                Field::MarginPx => "osd.margin_px",
                Field::TopMargin => "osd.top_margin",
                Field::Opacity => "osd.opacity",
                Field::WaveformWindowSecs => "osd.waveform_window_secs",
                Field::PeakDecayDbPerSec => "osd.peak_decay_db_per_sec",
                Field::WaveformGain => "osd.waveform_gain",
            })
            .collect();

        let missing: Vec<&&str> = expected.iter().filter(|k| !covered.contains(k)).collect();
        assert!(
            missing.is_empty(),
            "schema keys in section OSD with no TUI field: {missing:?}"
        );

        // Guard the other direction too: a field editing a key the schema
        // does not know about would be invisible to `config set`.
        let unknown: Vec<&&str> = covered
            .iter()
            .filter(|k| !CONFIG_KEYS.iter().any(|s| s.key == **k))
            .collect();
        assert!(
            unknown.is_empty(),
            "TUI fields editing keys absent from the schema: {unknown:?}"
        );
    }

    #[test]
    fn plugin_path_commit_trims_and_allows_empty() {
        let mut state = OsdState {
            enabled: true,
            frontend: "quickshell".to_string(),
            style: "default".to_string(),
            styles: vec![InstalledStyle {
                name: "default".to_string(),
                dir: None,
                description: None,
            }],
            plugin_path: String::new(),
            palette: "auto".to_string(),
            layout: "compact".to_string(),
            position: "bottom-center".to_string(),
            width_px: 400,
            height_px: 48,
            margin_px: 24,
            top_margin: 0.85,
            opacity: 0.95,
            waveform_window_secs: 3.0,
            peak_decay_db_per_sec: 6.0,
            waveform_gain: 10.0,
            field: Field::PluginPath,
            editing: None,
            feedback: None,
            dirty_since_load: false,
            binary_missing: false,
        };

        state.commit_text_edit(Field::PluginPath, "  ~/dev/my-style  ".to_string());
        assert_eq!(state.plugin_path, "~/dev/my-style");
        assert!(state.dirty_since_load);

        // Committing an empty buffer clears the path so save() unsets it.
        state.commit_text_edit(Field::PluginPath, "   ".to_string());
        assert_eq!(state.plugin_path, "");

        // Style falls back to "default" rather than saving an empty name.
        state.commit_text_edit(Field::Style, "".to_string());
        assert_eq!(state.style, "default");
    }

    #[test]
    fn style_field_cycles_discovered_styles_but_not_custom_paths() {
        let mut state = OsdState {
            enabled: true,
            frontend: "quickshell".to_string(),
            style: "default".to_string(),
            styles: vec![
                InstalledStyle {
                    name: "default".to_string(),
                    dir: None,
                    description: None,
                },
                InstalledStyle {
                    name: "aegis-hud".to_string(),
                    dir: Some(std::path::PathBuf::from("/tmp/aegis-hud")),
                    description: Some("HUD showcase".to_string()),
                },
            ],
            plugin_path: String::new(),
            palette: "auto".to_string(),
            layout: "compact".to_string(),
            position: "bottom-center".to_string(),
            width_px: 400,
            height_px: 48,
            margin_px: 24,
            top_margin: 0.85,
            opacity: 0.95,
            waveform_window_secs: 3.0,
            peak_decay_db_per_sec: 6.0,
            waveform_gain: 10.0,
            field: Field::Style,
            editing: None,
            feedback: None,
            dirty_since_load: false,
            binary_missing: false,
        };

        state.cycle(1);
        assert_eq!(state.style, "aegis-hud");
        state.cycle(1);
        assert_eq!(state.style, "default");

        // A hand-entered package path is not in the discovered list;
        // cycling must not replace it.
        state.style = "~/dev/my-style".to_string();
        state.cycle(1);
        assert_eq!(state.style, "~/dev/my-style");
    }
}
