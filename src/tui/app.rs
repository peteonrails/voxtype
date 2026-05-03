//! TUI application state.

use crate::setup::binary::{self, Acceleration, EngineFamily, InstallKind, Inventory, Variant};
use std::path::Path;

use super::audio::AudioState;
use super::engine::EngineState;
use super::hotkey::HotkeyState;
use super::advanced_section::AdvancedState;
use super::meeting_section::MeetingState;
use super::notifications_section::NotificationsState;
use super::output_section::OutputState;
use super::section::Section;
use super::text_section::TextState;
use super::vad_section::VadState;
use super::waybar_section::WaybarState;

/// What the event handler asks the run-loop to do next.
pub enum Action {
    None,
    /// User pressed q / Ctrl-C / Esc-from-sidebar. The run-loop checks
    /// whether any section was visited and routes through the save-on-exit
    /// prompt before terminating.
    Quit,
    /// User has already answered the save-on-exit prompt (Save or Discard);
    /// terminate immediately without re-prompting.
    ForceQuit,
    /// Move /usr/bin/voxtype to the named variant via pkexec.
    SwitchVariant(Variant),
    /// Run `voxtype setup model <model>` to download a missing model. The
    /// engine name is included only for human-readable feedback.
    DownloadModel { engine: String, model: String },
    /// Engine save needed BOTH a binary swap and a model download. Run them
    /// in order — the symlink swap first so the post-download daemon
    /// restart picks the right binary.
    SwitchVariantAndDownload {
        variant: Variant,
        engine: String,
        model: String,
    },
}

/// Result of the most recent variant-switch attempt, displayed as a banner.
pub struct SwitchOutcome {
    pub success: bool,
    pub message: String,
}

/// Layout of the variant matrix: rows = engine family, columns = acceleration.
pub const ROWS: &[EngineFamily] = &[EngineFamily::Whisper, EngineFamily::Onnx];
pub const COLS: &[Acceleration] = &[
    Acceleration::Avx2,
    Acceleration::Avx512,
    Acceleration::Vulkan,
    Acceleration::Cuda,
    Acceleration::Migraphx,
];

pub struct App {
    pub inventory: Inventory,
    /// Cursor position in the variant matrix (row, col).
    pub cursor: (usize, usize),
    /// True after the active variant changes; daemon should be restarted.
    pub restart_needed: bool,
    /// Result of the last switch attempt, if any.
    pub last_switch: Option<SwitchOutcome>,
    pub daemon_running: bool,
    /// Hidden testing flag: render as Package install even when running from a
    /// source build, so the variant matrix can be exercised in dev.
    pub force_package_mode: bool,
    /// Section currently rendered in the right pane.
    pub current_section: Section,
    /// Index into Section::ALL of the section being hovered in the sidebar.
    /// Independent of `current_section` so the user can scroll the sidebar
    /// without committing.
    pub sidebar_cursor: usize,
    /// True when keyboard input is steered at the sidebar (Tab toggles).
    pub sidebar_focused: bool,
    /// `?` toggles a centered help overlay listing every keybinding.
    pub help_open: bool,
    /// True while the save-on-exit prompt is showing. Cleared when the user
    /// picks Save (s), Discard (d), or Cancel (Esc/c).
    pub quit_pending: bool,
    /// If the configured engine's model isn't downloaded, this holds the
    /// model name so the General banner can prompt the user to fetch it.
    /// Computed at load time and on `refresh_inventory()`.
    pub missing_model: Option<MissingModel>,
    /// Lazily loaded Hotkey section state. None until the user opens Hotkey
    /// for the first time (or load fails).
    pub hotkey: Option<HotkeyState>,
    pub audio: Option<AudioState>,
    pub engine: Option<EngineState>,
    pub output: Option<OutputState>,
    pub text: Option<TextState>,
    pub vad: Option<VadState>,
    pub meeting: Option<MeetingState>,
    pub notifications: Option<NotificationsState>,
    pub waybar: Option<WaybarState>,
    pub advanced: Option<AdvancedState>,
}

#[derive(Debug, Clone)]
pub struct MissingModel {
    pub engine: String,
    pub model: String,
    pub setup_command: &'static str,
}

/// Build the inventory and, if `force_package_mode` is set, override the
/// install_kind so the TUI exercises the variant-matrix code path during
/// development without needing to install the binary.
fn build_inventory(force_package_mode: bool) -> Inventory {
    let mut inv = binary::inventory();
    if force_package_mode && inv.install_kind == InstallKind::Source {
        inv.install_kind = InstallKind::Package;
        if inv.package_lib_dir.is_none() {
            inv.package_lib_dir = Some(Path::new(binary::LIB_DIR).to_path_buf());
        }
        // If `enumerate_installed()` was skipped because we resolved as Source,
        // populate the matrix now so cells render with real on-disk state.
        if inv.variants.is_empty() {
            inv.variants = Variant::ALL
                .iter()
                .map(|&v| binary::VariantStatus {
                    variant: v,
                    binary_name: v.binary_name().to_string(),
                    installed: Path::new(binary::LIB_DIR).join(v.binary_name()).exists(),
                    runs_on_this_cpu: variant_runs_on_cpu(v, &inv.cpu),
                    gpu_available: variant_gpu_available(v, &inv.gpus),
                    active: inv.active_variant == Some(v),
                })
                .collect();
        }
    }
    inv
}

fn variant_runs_on_cpu(v: Variant, cpu: &binary::Cpu) -> bool {
    match v.acceleration() {
        Acceleration::Avx512 | Acceleration::Cuda | Acceleration::Migraphx => cpu.avx512,
        _ => cpu.avx2,
    }
}

fn variant_gpu_available(v: Variant, g: &binary::Gpus) -> bool {
    match v.acceleration() {
        Acceleration::Cuda => g.nvidia,
        Acceleration::Migraphx => g.amd,
        _ => true,
    }
}

impl App {
    pub fn new(force_package_mode: bool) -> Self {
        let inventory = build_inventory(force_package_mode);
        let cursor = initial_cursor(&inventory);
        Self {
            inventory,
            cursor,
            restart_needed: false,
            last_switch: None,
            daemon_running: is_daemon_running(),
            force_package_mode,
            current_section: Section::General,
            sidebar_cursor: 0,
            sidebar_focused: true,
            help_open: false,
            quit_pending: false,
            missing_model: detect_missing_model(),
            hotkey: None,
            audio: None,
            engine: None,
            output: None,
            text: None,
            vad: None,
            meeting: None,
            notifications: None,
            waybar: None,
            advanced: None,
        }
    }

    /// Ensure section-specific state is loaded the first time a section opens.
    pub fn ensure_section_loaded(&mut self) {
        match self.current_section {
            Section::Hotkey if self.hotkey.is_none() => {
                self.hotkey = HotkeyState::load().ok();
            }
            Section::Audio if self.audio.is_none() => {
                self.audio = AudioState::load().ok();
            }
            Section::Engine if self.engine.is_none() => {
                self.engine = EngineState::load().ok();
            }
            Section::Output if self.output.is_none() => {
                self.output = OutputState::load().ok();
            }
            Section::Text if self.text.is_none() => {
                self.text = TextState::load().ok();
            }
            Section::Vad if self.vad.is_none() => {
                self.vad = VadState::load().ok();
            }
            Section::Meeting if self.meeting.is_none() => {
                self.meeting = MeetingState::load().ok();
            }
            Section::Notifications if self.notifications.is_none() => {
                self.notifications = NotificationsState::load().ok();
            }
            Section::Waybar if self.waybar.is_none() => {
                self.waybar = WaybarState::load().ok();
            }
            Section::Advanced if self.advanced.is_none() => {
                self.advanced = AdvancedState::load().ok();
            }
            _ => {}
        }
    }

    pub fn move_sidebar(&mut self, delta: i32) {
        let len = Section::ALL.len() as i32;
        if len == 0 {
            return;
        }
        let new = (self.sidebar_cursor as i32 + delta).clamp(0, len - 1);
        self.sidebar_cursor = new as usize;
    }

    pub fn open_hovered_section(&mut self) {
        if let Some(section) = Section::ALL.get(self.sidebar_cursor).copied() {
            self.current_section = section;
            self.ensure_section_loaded();
        }
    }

    pub fn focus_sidebar(&mut self) {
        self.sidebar_focused = true;
        // Keep cursor in sync with the active section so the user lands on the
        // currently-open section when they Tab back to the sidebar.
        if let Some(idx) = Section::ALL
            .iter()
            .position(|s| *s == self.current_section)
        {
            self.sidebar_cursor = idx;
        }
    }

    pub fn focus_content(&mut self) {
        self.sidebar_focused = false;
    }

    /// True when a section is in inline-edit mode and should swallow keys
    /// instead of letting global shortcuts (Esc, Tab, q) act on them.
    pub fn is_editing(&self) -> bool {
        match self.current_section {
            Section::Engine => self.engine.as_ref().is_some_and(|s| s.editing.is_some()),
            Section::Output => self.output.as_ref().is_some_and(|s| s.editing.is_some()),
            Section::Hotkey => self.hotkey.as_ref().is_some_and(|s| s.editing.is_some()),
            Section::Audio => self.audio.as_ref().is_some_and(|s| s.editing.is_some()),
            Section::Waybar => self.waybar.as_ref().is_some_and(|s| s.editing.is_some()),
            _ => false,
        }
    }

    pub fn refresh_inventory(&mut self) {
        self.inventory = build_inventory(self.force_package_mode);
        self.daemon_running = is_daemon_running();
        self.missing_model = detect_missing_model();
    }

    /// True when at least one section state has been loaded — the user has
    /// visited that section, so it might hold unsaved field edits. Used to
    /// gate the save-on-exit prompt so users who only browse don't get
    /// asked.
    pub fn any_section_loaded(&self) -> bool {
        self.hotkey.is_some()
            || self.audio.is_some()
            || self.engine.is_some()
            || self.output.is_some()
            || self.text.is_some()
            || self.vad.is_some()
            || self.meeting.is_some()
            || self.notifications.is_some()
            || self.waybar.is_some()
            || self.advanced.is_some()
    }

    /// Save every loaded section to disk. Walks each Option<State> and calls
    /// the same `save()` path the `s` keybinding uses. Each section reloads
    /// the on-disk config, applies its current field values, validates, and
    /// atomically renames — sequential calls compose because each reload sees
    /// the prior save's output. Returns the count of sections saved so the
    /// caller can show a feedback line.
    pub fn save_all_loaded_sections(&mut self) -> usize {
        let mut count = 0;
        if let Some(s) = self.hotkey.as_mut() {
            s.save();
            count += 1;
        }
        if let Some(s) = self.audio.as_mut() {
            s.save();
            count += 1;
        }
        if let Some(s) = self.engine.as_mut() {
            s.save();
            count += 1;
        }
        if let Some(s) = self.output.as_mut() {
            s.save();
            count += 1;
        }
        if let Some(s) = self.text.as_mut() {
            s.save();
            count += 1;
        }
        if let Some(s) = self.vad.as_mut() {
            s.save();
            count += 1;
        }
        if let Some(s) = self.meeting.as_mut() {
            s.save();
            count += 1;
        }
        if let Some(s) = self.notifications.as_mut() {
            s.save();
            count += 1;
        }
        if let Some(s) = self.waybar.as_mut() {
            s.save();
            count += 1;
        }
        if let Some(s) = self.advanced.as_mut() {
            s.save();
            count += 1;
        }
        count
    }

    /// Map a (row, col) cell to a Variant if one exists for that combination.
    /// Returns None for invalid pairs (e.g. Whisper × CUDA).
    pub fn variant_at(&self, row: usize, col: usize) -> Option<Variant> {
        let family = *ROWS.get(row)?;
        let accel = *COLS.get(col)?;
        Variant::ALL
            .iter()
            .copied()
            .find(|v| v.family() == family && v.acceleration() == accel)
    }

    pub fn move_cursor(&mut self, drow: i32, dcol: i32) {
        let (r, c) = self.cursor;
        let new_r = clamp_signed(r as i32 + drow, ROWS.len());
        let new_c = clamp_signed(c as i32 + dcol, COLS.len());
        self.cursor = (new_r, new_c);
    }

    pub fn record_switch_attempt(&mut self, variant: Variant, result: Result<(), String>) {
        let (success, message) = match result {
            Ok(()) => (true, format!("Switched to {}.", variant.display())),
            Err(e) => (false, e),
        };
        if success {
            self.restart_needed = true;
        }
        self.last_switch = Some(SwitchOutcome { success, message });
        let _ = variant;
        self.refresh_inventory();
    }

    /// Record the outcome of a `voxtype setup model` invocation onto the
    /// same banner the variant-switch reuses, so the user sees it on the
    /// General screen the next time they focus it.
    pub fn record_download_attempt(
        &mut self,
        engine: &str,
        model: &str,
        result: Result<(), String>,
    ) {
        let (success, message) = match result {
            Ok(()) => (
                true,
                format!("Downloaded {} model `{}`.", engine, model),
            ),
            Err(e) => (false, format!("Download `{}`: {}", model, e)),
        };
        self.last_switch = Some(SwitchOutcome { success, message });
        self.refresh_inventory();
    }
}

fn clamp_signed(v: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    v.clamp(0, (len - 1) as i32) as usize
}

fn initial_cursor(inv: &Inventory) -> (usize, usize) {
    if let Some(active) = inv.active_variant {
        let row = ROWS.iter().position(|f| *f == active.family()).unwrap_or(0);
        let col = COLS
            .iter()
            .position(|a| *a == active.acceleration())
            .unwrap_or(0);
        (row, col)
    } else {
        (0, 0)
    }
}

/// Detect whether the configured engine's active model file is on disk.
/// Returns the engine + model name + a setup command hint when it's missing,
/// or None when the model is present (or we can't determine it).
fn detect_missing_model() -> Option<MissingModel> {
    use crate::config;
    let cfg = config::load_config(None).ok()?;
    let dir = config::Config::models_dir();
    let (engine_name, model, setup_command) = match cfg.engine {
        config::TranscriptionEngine::Whisper => (
            "whisper",
            cfg.whisper.model.clone(),
            "voxtype setup model",
        ),
        config::TranscriptionEngine::Parakeet => (
            "parakeet",
            cfg.parakeet
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
            "voxtype setup model",
        ),
        config::TranscriptionEngine::Moonshine => (
            "moonshine",
            cfg.moonshine
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
            "voxtype setup model",
        ),
        config::TranscriptionEngine::SenseVoice => (
            "sensevoice",
            cfg.sensevoice
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
            "voxtype setup model",
        ),
        config::TranscriptionEngine::Paraformer => (
            "paraformer",
            cfg.paraformer
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
            "voxtype setup model",
        ),
        config::TranscriptionEngine::Dolphin => (
            "dolphin",
            cfg.dolphin
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
            "voxtype setup model",
        ),
        config::TranscriptionEngine::Omnilingual => (
            "omnilingual",
            cfg.omnilingual
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
            "voxtype setup model",
        ),
        // Cohere — checked but model layout differs by rc/0.7.0; skip the
        // disk probe rather than emit a false-positive missing warning.
        config::TranscriptionEngine::Cohere => return None,
    };

    if model.is_empty() {
        return None;
    }

    let installed = if engine_name == "whisper" {
        dir.join(format!("ggml-{}.bin", model)).exists()
    } else {
        let p = dir.join(&model);
        p.exists()
    };
    if installed {
        None
    } else {
        Some(MissingModel {
            engine: engine_name.to_string(),
            model,
            setup_command,
        })
    }
}

/// Mirrors the check in main.rs; we duplicate it here to avoid a circular
/// dependency on a private helper.
fn is_daemon_running() -> bool {
    let pid_path = crate::config::Config::runtime_dir().join("pid");
    let pid_str = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let pid: u32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => return false,
    };
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_at_finds_known_pairs() {
        let app = App::new(false);
        // Whisper × AVX2 = WhisperAvx2
        assert_eq!(app.variant_at(0, 0), Some(Variant::WhisperAvx2));
        // ONNX × CUDA = OnnxCuda
        assert_eq!(app.variant_at(1, 3), Some(Variant::OnnxCuda));
    }

    #[test]
    fn variant_at_returns_none_for_invalid_pairs() {
        let app = App::new(false);
        // Whisper × CUDA — no such variant
        assert_eq!(app.variant_at(0, 3), None);
        // ONNX × Vulkan — no such variant
        assert_eq!(app.variant_at(1, 2), None);
    }

    #[test]
    fn move_cursor_clamps_at_edges() {
        let mut app = App::new(false);
        app.cursor = (0, 0);
        app.move_cursor(-1, -1);
        assert_eq!(app.cursor, (0, 0));
        app.move_cursor(10, 10);
        assert_eq!(app.cursor, (ROWS.len() - 1, COLS.len() - 1));
    }
}
