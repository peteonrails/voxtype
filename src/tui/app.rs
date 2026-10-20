//! TUI application state.

use crate::setup::binary::{self, InstallKind, Inventory, Variant};
use crate::setup::variant_check::{self, VariantMismatch};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use super::advanced_section::{self, AdvancedState};
use super::audio::{self, AudioState};
use super::common::{FeedbackLevel, FormRowSpec, CARET};
use super::engine::{self, EngineState};
use super::hotkey::{self, HotkeyState};
use super::meeting_section::{self, MeetingState};
use super::notifications_section::{self, NotificationsState};
use super::osd_section::{self, OsdState};
use super::output_section::{self, OutputState};
use super::section::Section;
use super::text_section::{self, TextState};
use super::vad_section::{self, VadState};
use super::waybar_section::{self, WaybarState};

/// What the event handler asks the run-loop to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    /// q / Ctrl-C / Esc from the page list. Asks first when anything is
    /// unsaved.
    Quit,
    /// The quit question was answered; leave without asking again.
    ForceQuit,
    /// `s`: save every page, then do whatever the saves need, then restart
    /// voxtype so it uses them.
    Save,
    /// The quit question's "save": save and apply, then quit if every page
    /// saved.
    SaveAndQuit,
    /// Switch the installed build via pkexec, then restart voxtype.
    SwitchVariant(Variant),
    /// Run `voxtype setup --download --model <model>`, then restart voxtype.
    DownloadModel {
        engine: String,
        model: String,
    },
    /// An engine change needs both a different build and a model download.
    /// The build switches first so the restart afterwards uses it.
    SwitchVariantAndDownload {
        variant: Variant,
        engine: String,
        model: String,
    },
}

pub struct App {
    pub inventory: Inventory,
    /// Row under the cursor on the Build page.
    pub build_cursor: usize,
    /// Row under the cursor on the Basics page.
    pub basics_cursor: usize,
    /// Hidden testing flag: render as Package install even when running from a
    /// source build, so the Build page can be exercised in dev.
    pub force_package_mode: bool,
    /// Page shown on the right.
    pub current_section: Section,
    /// Index into Section::ALL of the page under the cursor in the list.
    pub sidebar_cursor: usize,
    /// True when keys go to the page list rather than the open page.
    pub sidebar_focused: bool,
    pub help_open: bool,
    /// First visible line of the help overlay.
    pub help_scroll: std::cell::Cell<u16>,
    /// First visible line of the explanation under the settings.
    pub guidance_scroll: std::cell::Cell<u16>,
    /// Each page's values as last saved (or loaded), keyed by row label, so
    /// a row that differs can carry the unsaved marker.
    baselines: RefCell<HashMap<&'static str, HashMap<String, String>>>,
    /// The open page's own message, handed over while it renders so the
    /// status line can show it.
    page_message: RefCell<Option<(FeedbackLevel, String)>>,
    /// True while the quit question is showing.
    pub quit_pending: bool,
    /// Result of the last save, switch, download or restart. Stays until
    /// something changes again.
    pub notice: Option<(FeedbackLevel, String)>,
    /// The configured engine's model when it isn't downloaded.
    pub missing_model: Option<MissingModel>,
    /// `Some` when the installed build can't run the configured engine
    /// (#450).
    pub variant_mismatch: Option<VariantMismatch>,
    pub hotkey: Option<HotkeyState>,
    pub audio: Option<AudioState>,
    pub engine: Option<EngineState>,
    pub output: Option<OutputState>,
    pub text: Option<TextState>,
    pub vad: Option<VadState>,
    pub meeting: Option<MeetingState>,
    pub notifications: Option<NotificationsState>,
    pub osd: Option<OsdState>,
    pub waybar: Option<WaybarState>,
    pub advanced: Option<AdvancedState>,
}

#[derive(Debug, Clone)]
pub struct MissingModel {
    pub engine: String,
    pub model: String,
}

/// Build the inventory and, if `force_package_mode` is set, override the
/// install_kind so the TUI exercises the Build page during development
/// without needing to install the binary.
fn build_inventory(force_package_mode: bool) -> Inventory {
    let mut inv = binary::inventory();
    if force_package_mode && inv.install_kind == InstallKind::Source {
        inv.install_kind = InstallKind::Package;
        if inv.package_lib_dir.is_none() {
            inv.package_lib_dir = Some(Path::new(binary::LIB_DIR).to_path_buf());
        }
        if inv.variants.is_empty() {
            inv.variants = binary::variant_statuses(
                Path::new(binary::LIB_DIR),
                &inv.cpu,
                &inv.gpus,
                inv.active_variant,
            );
        }
    }
    inv
}

/// Run `$body` with `$state` bound to each loaded section's state.
macro_rules! for_each_loaded {
    ($app:expr, |$state:ident| $body:expr) => {{
        if let Some($state) = $app.engine.as_mut() {
            $body
        }
        if let Some($state) = $app.hotkey.as_mut() {
            $body
        }
        if let Some($state) = $app.audio.as_mut() {
            $body
        }
        if let Some($state) = $app.vad.as_mut() {
            $body
        }
        if let Some($state) = $app.output.as_mut() {
            $body
        }
        if let Some($state) = $app.text.as_mut() {
            $body
        }
        if let Some($state) = $app.notifications.as_mut() {
            $body
        }
        if let Some($state) = $app.osd.as_mut() {
            $body
        }
        if let Some($state) = $app.waybar.as_mut() {
            $body
        }
        if let Some($state) = $app.meeting.as_mut() {
            $body
        }
        if let Some($state) = $app.advanced.as_mut() {
            $body
        }
    }};
}

impl App {
    pub fn new(force_package_mode: bool) -> Self {
        let inventory = build_inventory(force_package_mode);
        let build_cursor = super::build::initial_cursor(&inventory);
        let variant_mismatch = detect_variant_mismatch(&inventory);
        let mut app = Self {
            inventory,
            build_cursor,
            basics_cursor: 0,
            force_package_mode,
            current_section: Section::Basics,
            sidebar_cursor: 0,
            sidebar_focused: false,
            help_open: false,
            help_scroll: std::cell::Cell::new(0),
            guidance_scroll: std::cell::Cell::new(0),
            baselines: RefCell::new(HashMap::new()),
            page_message: RefCell::new(None),
            quit_pending: false,
            notice: None,
            missing_model: detect_missing_model(),
            variant_mismatch,
            hotkey: None,
            audio: None,
            engine: None,
            output: None,
            text: None,
            vad: None,
            meeting: None,
            notifications: None,
            osd: None,
            waybar: None,
            advanced: None,
        };
        app.ensure_section_loaded();
        app.capture_baselines();
        app
    }

    /// Load a page's settings the first time it opens. Basics shows settings
    /// from four pages, so it loads all four.
    pub fn ensure_section_loaded(&mut self) {
        let wanted: &[Section] = match self.current_section {
            Section::Basics => &[
                Section::Engine,
                Section::Hotkey,
                Section::Output,
                Section::Osd,
            ],
            Section::Engine => &[Section::Engine],
            Section::Hotkey => &[Section::Hotkey],
            Section::Audio => &[Section::Audio],
            Section::Output => &[Section::Output],
            Section::Text => &[Section::Text],
            Section::Vad => &[Section::Vad],
            Section::Meeting => &[Section::Meeting],
            Section::Notifications => &[Section::Notifications],
            Section::Osd => &[Section::Osd],
            Section::Waybar => &[Section::Waybar],
            Section::Advanced => &[Section::Advanced],
            Section::Build => &[],
        };
        for section in wanted {
            self.load_section(*section);
        }
    }

    fn load_section(&mut self, section: Section) {
        match section {
            Section::Hotkey if self.hotkey.is_none() => self.hotkey = HotkeyState::load().ok(),
            Section::Audio if self.audio.is_none() => self.audio = AudioState::load().ok(),
            Section::Engine if self.engine.is_none() => self.engine = EngineState::load().ok(),
            Section::Output if self.output.is_none() => self.output = OutputState::load().ok(),
            Section::Text if self.text.is_none() => self.text = TextState::load().ok(),
            Section::Vad if self.vad.is_none() => self.vad = VadState::load().ok(),
            Section::Meeting if self.meeting.is_none() => self.meeting = MeetingState::load().ok(),
            Section::Notifications if self.notifications.is_none() => {
                self.notifications = NotificationsState::load().ok()
            }
            Section::Osd if self.osd.is_none() => self.osd = OsdState::load().ok(),
            Section::Waybar if self.waybar.is_none() => self.waybar = WaybarState::load().ok(),
            Section::Advanced if self.advanced.is_none() => {
                self.advanced = AdvancedState::load().ok()
            }
            _ => {}
        }
    }

    pub fn move_sidebar(&mut self, delta: i32) {
        let len = Section::ALL.len() as i32;
        let new = (self.sidebar_cursor as i32 + delta).clamp(0, len - 1);
        self.sidebar_cursor = new as usize;
    }

    pub fn open_hovered_section(&mut self) {
        if let Some(section) = Section::ALL.get(self.sidebar_cursor).copied() {
            if section != self.current_section {
                self.notice = None;
            }
            self.current_section = section;
            self.ensure_section_loaded();
            self.capture_baselines();
        }
    }

    /// Open a page and put the keys on it.
    pub fn jump_to_section(&mut self, section: Section) {
        self.current_section = section;
        if let Some(idx) = Section::ALL.iter().position(|s| *s == section) {
            self.sidebar_cursor = idx;
        }
        self.sidebar_focused = false;
        self.ensure_section_loaded();
        self.capture_baselines();
    }

    pub fn focus_sidebar(&mut self) {
        self.sidebar_focused = true;
        if let Some(idx) = Section::ALL.iter().position(|s| *s == self.current_section) {
            self.sidebar_cursor = idx;
        }
    }

    pub fn focus_content(&mut self) {
        self.sidebar_focused = false;
    }

    /// True when keys go to the open page rather than the list.
    pub fn content_focused(&self) -> bool {
        !self.sidebar_focused
    }

    /// True when `section` holds edits that are not in the config file yet.
    pub fn section_dirty(&self, section: Section) -> bool {
        match section {
            Section::Basics => [
                Section::Engine,
                Section::Hotkey,
                Section::Output,
                Section::Osd,
            ]
            .iter()
            .any(|s| self.section_dirty(*s)),
            Section::Build => false,
            Section::Engine => self.engine.as_ref().is_some_and(|s| s.dirty_since_load),
            Section::Hotkey => self.hotkey.as_ref().is_some_and(|s| s.dirty_since_load),
            Section::Audio => self.audio.as_ref().is_some_and(|s| s.dirty_since_load),
            Section::Output => self.output.as_ref().is_some_and(|s| s.dirty_since_load),
            Section::Text => self.text.as_ref().is_some_and(|s| s.dirty_since_load),
            Section::Vad => self.vad.as_ref().is_some_and(|s| s.dirty_since_load),
            Section::Meeting => self.meeting.as_ref().is_some_and(|s| s.dirty_since_load),
            Section::Notifications => self
                .notifications
                .as_ref()
                .is_some_and(|s| s.dirty_since_load),
            Section::Osd => self.osd.as_ref().is_some_and(|s| s.dirty_since_load),
            Section::Waybar => self.waybar.as_ref().is_some_and(|s| s.dirty_since_load),
            Section::Advanced => self.advanced.as_ref().is_some_and(|s| s.dirty_since_load),
        }
    }

    /// Pages holding unsaved edits, in list order. Basics is left out: its
    /// settings belong to other pages, which are counted instead.
    pub fn dirty_sections(&self) -> Vec<Section> {
        Section::ALL
            .iter()
            .copied()
            .filter(|s| *s != Section::Basics && self.section_dirty(*s))
            .collect()
    }

    /// How many settings differ from the file, across every page.
    pub fn unsaved_count(&self) -> usize {
        self.dirty_sections()
            .into_iter()
            .map(|s| {
                let rows = self.section_rows(s).unwrap_or_default();
                let changed = self
                    .modified_rows(s, &rows)
                    .into_iter()
                    .filter(|m| *m)
                    .count();
                // An edit with no row of its own (a removed replacement)
                // still counts; one undone by hand doesn't.
                match changed {
                    0 if self.matches_baseline(s, &rows) => 0,
                    0 => 1,
                    n => n,
                }
            })
            .sum()
    }

    /// True when `rows` are exactly what `section` had when last saved.
    fn matches_baseline(&self, section: Section, rows: &[FormRowSpec]) -> bool {
        let baselines = self.baselines.borrow();
        let Some(saved) = baselines.get(section.label()) else {
            return false;
        };
        rows.len() == saved.len()
            && rows
                .iter()
                .all(|r| saved.get(&r.label).is_some_and(|v| *v == r.value))
    }

    /// The rows a page shows, from its current in-memory settings.
    pub fn section_rows(&self, section: Section) -> Option<Vec<FormRowSpec>> {
        match section {
            Section::Basics => Some(super::basics::rows(self)),
            Section::Build => None,
            Section::Engine => self.engine.as_ref().map(engine::rows),
            Section::Hotkey => self.hotkey.as_ref().map(hotkey::rows),
            Section::Audio => self.audio.as_ref().map(audio::rows),
            Section::Output => self.output.as_ref().map(output_section::rows),
            Section::Text => self.text.as_ref().map(text_section::rows),
            Section::Vad => self.vad.as_ref().map(vad_section::rows),
            Section::Meeting => self.meeting.as_ref().map(meeting_section::rows),
            Section::Notifications => self.notifications.as_ref().map(notifications_section::rows),
            Section::Osd => self.osd.as_ref().map(osd_section::rows),
            Section::Waybar => self.waybar.as_ref().map(waybar_section::rows),
            Section::Advanced => self.advanced.as_ref().map(advanced_section::rows),
        }
    }

    /// Remember the rows of every loaded page that has nothing unsaved, as
    /// the values a later edit is compared against. Called after every key,
    /// so a save, a revert or a reload resets the markers.
    pub fn capture_baselines(&self) {
        for section in Section::ALL {
            if *section == Section::Basics || self.section_dirty(*section) {
                continue;
            }
            let Some(rows) = self.section_rows(*section) else {
                continue;
            };
            let values: HashMap<String, String> = rows
                .into_iter()
                .filter(|r| !r.value.contains(CARET))
                .map(|r| (r.label, r.value))
                .collect();
            self.baselines.borrow_mut().insert(section.label(), values);
        }
    }

    /// Which of `rows` on `section` differ from what is saved.
    pub fn modified_rows(&self, section: Section, rows: &[FormRowSpec]) -> Vec<bool> {
        if section == Section::Basics {
            return super::basics::modified(self, rows);
        }
        if !self.section_dirty(section) {
            return vec![false; rows.len()];
        }
        let baselines = self.baselines.borrow();
        let saved = baselines.get(section.label());
        rows.iter()
            .map(|row| {
                !row.value.contains(CARET)
                    && saved
                        .and_then(|s| s.get(&row.label))
                        .is_some_and(|before| *before != row.value)
            })
            .collect()
    }

    /// Whether the row labelled `label` on `section` differs from the file.
    pub fn row_modified(&self, section: Section, label: &str, value: &str) -> bool {
        self.section_dirty(section)
            && !value.contains(CARET)
            && self
                .baselines
                .borrow()
                .get(section.label())
                .and_then(|s| s.get(label))
                .is_some_and(|before| before != value)
    }

    pub fn set_page_message(&self, message: Option<(FeedbackLevel, &str)>) {
        *self.page_message.borrow_mut() = message.map(|(l, m)| (l, m.to_string()));
    }

    pub fn page_message(&self) -> Option<(FeedbackLevel, String)> {
        self.page_message.borrow().clone()
    }

    /// True when a field on the open page is being typed into, so every key
    /// goes to it.
    pub fn is_editing(&self) -> bool {
        let section = match self.current_section {
            Section::Basics => super::basics::target_section(self),
            other => other,
        };
        match section {
            Section::Engine => self.engine.as_ref().is_some_and(|s| s.editing.is_some()),
            Section::Output => self.output.as_ref().is_some_and(|s| s.editing.is_some()),
            Section::Hotkey => self.hotkey.as_ref().is_some_and(|s| s.editing.is_some()),
            Section::Audio => self.audio.as_ref().is_some_and(|s| s.editing.is_some()),
            Section::Waybar => self.waybar.as_ref().is_some_and(|s| s.editing.is_some()),
            Section::Osd => self.osd.as_ref().is_some_and(|s| s.editing.is_some()),
            Section::Text => self.text.as_ref().is_some_and(|s| s.editing.is_some()),
            _ => false,
        }
    }

    /// Give pages with background work a chance to fold results in.
    pub fn poll_background(&mut self) {
        if let Some(audio) = self.audio.as_mut() {
            audio.poll_device_scan();
        }
    }

    pub fn refresh_inventory(&mut self) {
        self.inventory = build_inventory(self.force_package_mode);
        self.missing_model = detect_missing_model();
        self.variant_mismatch = detect_variant_mismatch(&self.inventory);
    }

    /// Save every page with unsaved edits. Each save reloads the file,
    /// applies its page and validates before renaming, so the saves
    /// compose. Returns the pages that failed (they keep their error and
    /// their edits) and the follow-up work the saves asked for, such as a
    /// model download.
    pub fn save_all(&mut self) -> (Vec<Section>, Vec<Action>) {
        let mut actions = Vec::new();
        for section in self.dirty_sections() {
            let action = match section {
                Section::Engine => self.engine.as_mut().map(|s| s.save()),
                Section::Hotkey => self.hotkey.as_mut().map(|s| s.save()),
                Section::Audio => self.audio.as_mut().map(|s| s.save()),
                Section::Output => self.output.as_mut().map(|s| s.save()),
                Section::Text => self.text.as_mut().map(|s| s.save()),
                Section::Vad => self.vad.as_mut().map(|s| s.save()),
                Section::Meeting => self.meeting.as_mut().map(|s| s.save()),
                Section::Notifications => self.notifications.as_mut().map(|s| s.save()),
                Section::Osd => self.osd.as_mut().map(|s| s.save()),
                Section::Waybar => self.waybar.as_mut().map(|s| s.save()),
                Section::Advanced => self.advanced.as_mut().map(|s| s.save()),
                Section::Basics | Section::Build => None,
            };
            if let Some(action) = action.filter(|a| *a != Action::None) {
                actions.push(action);
            }
        }
        let failed = self.dirty_sections();
        // One save can write another page's setting (streaming switches the
        // hotkey mode and the Parakeet model), so every clean page re-reads
        // the file. Their per-save messages give way to the one result line.
        for_each_loaded!(self, |state| {
            if !state.dirty_since_load {
                state.reset();
                state.feedback = None;
            }
        });
        self.refresh_inventory();
        self.capture_baselines();
        (failed, actions)
    }

    /// Make voxtype use what was just saved: restart it when it runs from
    /// the file this TUI edits. Sets the result line.
    pub fn apply_saved(&mut self) {
        let path = super::common::display_config_path();
        if !edits_daemon_config() {
            self.notice = Some((FeedbackLevel::Ok, format!("Saved to {}", path)));
            return;
        }
        if !crate::daemon_status::is_daemon_running() {
            self.notice = Some((
                FeedbackLevel::Ok,
                "Saved. voxtype isn't running; it uses these settings when it starts".to_string(),
            ));
            return;
        }
        if self.restart_daemon() {
            self.notice = Some((
                FeedbackLevel::Ok,
                "Saved and applied: voxtype restarted with your changes".to_string(),
            ));
        }
    }

    /// Restart the voxtype service, which restarts the on-screen display it
    /// runs too. On failure the result line says what went wrong and how to
    /// do it by hand.
    pub fn restart_daemon(&mut self) -> bool {
        let result = std::process::Command::new("systemctl")
            .args(["--user", "restart", "voxtype"])
            .output();
        match result {
            Ok(out) if out.status.success() => true,
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let detail = stderr.lines().next().unwrap_or("no error output");
                // voxtype started by hand or by the desktop isn't a unit
                // systemd can restart; only its owner can.
                let not_a_service = detail.contains("not found") || detail.contains("not loaded");
                self.notice = Some((
                    FeedbackLevel::Err,
                    if not_a_service {
                        "Saved. voxtype isn't a systemd service here: quit and start it again \
                         the way you started it"
                            .to_string()
                    } else {
                        format!(
                            "Saved, but voxtype didn't restart ({}). Run: systemctl --user restart voxtype",
                            detail
                        )
                    },
                ));
                false
            }
            Err(_) => {
                self.notice = Some((
                    FeedbackLevel::Err,
                    "Saved, but systemctl isn't available. Restart voxtype the way you started it"
                        .to_string(),
                ));
                false
            }
        }
    }

    pub fn record_switch_attempt(&mut self, variant: Variant, result: Result<(), String>) {
        self.notice = Some(match result {
            Ok(()) => (
                FeedbackLevel::Ok,
                format!("Switched to {}", super::build::variant_name(variant)),
            ),
            Err(e) => (FeedbackLevel::Err, format!("Build not switched: {}", e)),
        });
        self.refresh_inventory();
    }

    pub fn record_download_attempt(&mut self, model: &str, result: Result<(), String>) {
        self.notice = Some(match result {
            Ok(()) => (FeedbackLevel::Ok, format!("Downloaded {}", model)),
            Err(e) => (
                FeedbackLevel::Err,
                format!("{} not downloaded: {}", model, e),
            ),
        });
        self.refresh_inventory();
    }
}

/// True when the file being edited is the one the voxtype service reads,
/// so restarting the service applies it. With `--config` pointing anywhere
/// else, a restart would only reload the old settings.
pub fn edits_daemon_config() -> bool {
    match (
        super::config_editor::tui_config_path(),
        crate::config::Config::default_path(),
    ) {
        (Some(editing), Some(default)) => editing == default,
        _ => false,
    }
}

/// Compute the engine-vs-installed-build mismatch (see #450). `None` when
/// the config can't be read: the TUI must still open to fix it.
fn detect_variant_mismatch(inventory: &Inventory) -> Option<VariantMismatch> {
    let cfg = load_edited_config()?;
    variant_check::detect_mismatch(&cfg, inventory)
}

/// The config this TUI edits, parsed.
fn load_edited_config() -> Option<crate::config::Config> {
    let path = super::config_editor::tui_config_path();
    crate::config::load_config(path.as_deref()).ok()
}

/// The configured engine's model, when it isn't on disk.
fn detect_missing_model() -> Option<MissingModel> {
    use crate::config;
    let cfg = load_edited_config()?;
    let (engine_name, model) = match cfg.engine {
        config::TranscriptionEngine::Whisper => ("whisper", cfg.whisper.model.clone()),
        config::TranscriptionEngine::Parakeet => (
            "parakeet",
            cfg.parakeet
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
        ),
        config::TranscriptionEngine::Moonshine => (
            "moonshine",
            cfg.moonshine
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
        ),
        config::TranscriptionEngine::SenseVoice => (
            "sensevoice",
            cfg.sensevoice
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
        ),
        config::TranscriptionEngine::Paraformer => (
            "paraformer",
            cfg.paraformer
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
        ),
        config::TranscriptionEngine::Dolphin => (
            "dolphin",
            cfg.dolphin
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
        ),
        config::TranscriptionEngine::Omnilingual => (
            "omnilingual",
            cfg.omnilingual
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
        ),
        // Cohere's model layout differs between releases; skip the disk
        // probe rather than warn about a model that is there.
        config::TranscriptionEngine::Cohere => return None,
        // Soniox is cloud-only, no local model to probe.
        config::TranscriptionEngine::Soniox => return None,
        // OpenVINO models are multi-file IR directories.
        config::TranscriptionEngine::OpenVino => return None,
    };

    // A remote Whisper has no local model to download.
    if engine_name == "whisper" && cfg.whisper.mode == Some(config::WhisperMode::Remote) {
        return None;
    }
    if model.is_empty() {
        return None;
    }

    if crate::model_catalog::model_installed(engine_name, &model) {
        None
    } else {
        Some(MissingModel {
            engine: engine_name.to_string(),
            model,
        })
    }
}
