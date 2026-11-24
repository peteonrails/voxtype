//! `voxtype doctor`: one health check for everything voxtype depends on.
//!
//! Each check produces a [`Check`] with a severity and, when something is
//! wrong, the exact command or setting that fixes it. The same list drives
//! three surfaces:
//!
//! - `voxtype doctor` (text or `--json`)
//! - `voxtype setup check`, kept as an alias
//! - the daemon's startup summary, which runs the cheap subset
//!   ([`Scope::Startup`]) and sends one grouped notification for errors
//!
//! The decision logic lives in small pure functions that take what was
//! observed (paths, tool lookups, parsed command output) so it can be tested
//! without the tools installed. The `gather_*` functions do the observing.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::{Config, OutputMode, TranscriptionEngine, VadBackend, WhisperMode};
use crate::model_catalog::{self, ModelHealth};

use super::{DisplayServer, OutputChainStatus};

/// How bad a check result is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Ok,
    /// Works, but degraded or likely to surprise the user.
    Warn,
    /// A feature the config asks for will not work until this is fixed.
    Error,
}

/// One health check result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// Stable identifier, `<area>.<name>`, for scripts reading `--json`.
    pub id: &'static str,
    pub severity: Severity,
    pub message: String,
    /// What to do about it. Always set for warnings and errors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl Check {
    fn ok(id: &'static str, message: impl Into<String>) -> Self {
        Self {
            id,
            severity: Severity::Ok,
            message: message.into(),
            fix: None,
        }
    }

    fn warn(id: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            id,
            severity: Severity::Warn,
            message: message.into(),
            fix: Some(fix.into()),
        }
    }

    fn error(id: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            id,
            severity: Severity::Error,
            message: message.into(),
            fix: Some(fix.into()),
        }
    }
}

/// Which checks to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Everything, for `voxtype doctor`.
    Full,
    /// Only checks that cost a stat, a PATH lookup, an env read or one small
    /// file read, and that nothing else in the daemon already notifies about.
    /// Run once at daemon startup.
    Startup,
}

/// Results of a doctor run, in display order.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn count(&self, severity: Severity) -> usize {
        self.checks
            .iter()
            .filter(|c| c.severity == severity)
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.count(Severity::Error) > 0
    }

    pub fn errors(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.severity == Severity::Error)
    }
}

// ---------------------------------------------------------------------------
// Pure checks
// ---------------------------------------------------------------------------

pub fn cpu_check(warning: Option<String>, in_vm: bool) -> Check {
    match warning {
        Some(w) => Check::warn(
            "system.cpu",
            w,
            if in_vm {
                "Running in a virtual machine: pass the host CPU features through (e.g. host-passthrough)"
            } else {
                "Install a voxtype variant built for this CPU: sudo voxtype setup gpu --disable"
            },
        ),
        None if in_vm => Check::ok(
            "system.cpu",
            "CPU features compatible (virtual machine: keep CPU features passed through)",
        ),
        None => Check::ok("system.cpu", "CPU features compatible"),
    }
}

/// A directory or file voxtype needs to exist.
pub fn path_check(id: &'static str, label: &str, path: &Path, exists: bool) -> Check {
    if exists {
        Check::ok(id, format!("{label}: {}", path.display()))
    } else {
        Check::error(
            id,
            format!("{label} missing: {}", path.display()),
            "Run: voxtype setup",
        )
    }
}

/// Is the configured transcription model on disk and intact?
///
/// `api_key_env` says whether the cloud engine's API key variable
/// (`SONIOX_API_KEY` or `ELEVENLABS_API_KEY`) is set, the alternative to the
/// engine's `api_key` setting.
pub fn engine_model_check(config: &Config, models_dir: &Path, api_key_env: bool) -> Check {
    const ID: &str = "transcription.model";
    let engine = config.engine;
    let name = engine.name();

    let section_missing = match engine {
        TranscriptionEngine::Whisper => false,
        TranscriptionEngine::Parakeet => config.parakeet.is_none(),
        TranscriptionEngine::Moonshine => config.moonshine.is_none(),
        TranscriptionEngine::SenseVoice => config.sensevoice.is_none(),
        TranscriptionEngine::Paraformer => config.paraformer.is_none(),
        TranscriptionEngine::Dolphin => config.dolphin.is_none(),
        TranscriptionEngine::Omnilingual => config.omnilingual.is_none(),
        TranscriptionEngine::Cohere => config.cohere.is_none(),
        TranscriptionEngine::Gguf => config.gguf.is_none(),
        TranscriptionEngine::OpenVino => config.openvino.is_none(),
        TranscriptionEngine::Soniox => config.soniox.is_none(),
        // Every ElevenLabs setting has a default; the key can come from the
        // environment.
        TranscriptionEngine::ElevenLabs => false,
    };
    if section_missing {
        return Check::error(
            ID,
            format!("Engine is '{name}' but the [{name}] config section is missing"),
            format!(
                "Run: voxtype config set {name}.model {}",
                model_catalog::default_model(name)
            ),
        );
    }

    if engine == TranscriptionEngine::Soniox {
        let has_key = config.soniox.as_ref().is_some_and(|s| s.api_key.is_some());
        return if has_key || api_key_env {
            Check::ok(ID, "Soniox cloud engine: API key configured")
        } else {
            Check::error(
                ID,
                "Soniox engine selected but no API key is configured",
                "Set [soniox] api_key in config.toml or export SONIOX_API_KEY",
            )
        };
    }

    if engine == TranscriptionEngine::ElevenLabs {
        let has_key = config
            .elevenlabs
            .as_ref()
            .is_some_and(|e| e.api_key.is_some());
        return if has_key || api_key_env {
            Check::ok(ID, "ElevenLabs cloud engine: API key configured")
        } else {
            Check::error(
                ID,
                "ElevenLabs engine selected but no API key is configured",
                "Set [elevenlabs] api_key in config.toml or export ELEVENLABS_API_KEY",
            )
        };
    }

    if engine == TranscriptionEngine::Whisper
        && config.whisper.effective_mode() == WhisperMode::Remote
    {
        return match config.whisper.remote_endpoint {
            Some(ref endpoint) => Check::ok(ID, format!("Whisper remote mode: {endpoint}")),
            None => Check::error(
                ID,
                "Whisper remote mode selected but no endpoint is configured",
                "Set [whisper] remote_endpoint in config.toml",
            ),
        };
    }

    let model = config.model_name();
    model_presence_check(ID, name, model, models_dir, Severity::Error)
}

/// One model's presence, shared by the primary and secondary model checks.
fn model_presence_check(
    id: &'static str,
    engine: &str,
    model: &str,
    models_dir: &Path,
    severity: Severity,
) -> Check {
    let fail = |message: String, fix: String| match severity {
        Severity::Error => Check::error(id, message, fix),
        _ => Check::warn(id, message, fix),
    };

    // An explicit path (or a bare .bin file for Whisper) bypasses the catalog.
    let explicit = if model.contains('/') {
        Some(PathBuf::from(model))
    } else if engine == "whisper" && model.ends_with(".bin") {
        Some(models_dir.join(model))
    } else {
        None
    };
    if let Some(path) = explicit {
        return if path.exists() {
            Check::ok(id, format!("{engine} model: {}", path.display()))
        } else {
            fail(
                format!("{engine} model not found: {}", path.display()),
                "Fix the model path in config.toml, or pick a model: voxtype setup model"
                    .to_string(),
            )
        };
    }

    let download = model_catalog::download_command(engine, model);
    match model_catalog::model_health_in(models_dir, engine, model) {
        ModelHealth::Present => Check::ok(id, format!("{engine} model '{model}' installed")),
        ModelHealth::Missing => fail(
            format!("{engine} model '{model}' is not downloaded"),
            format!("Run: {download}"),
        ),
        ModelHealth::Corrupt(problems) => fail(
            format!(
                "{engine} model '{model}' is incomplete or damaged ({})",
                problems.join("; ")
            ),
            format!("Re-download it: {download}"),
        ),
    }
}

/// The Whisper secondary model, when one is configured.
pub fn secondary_model_check(config: &Config, models_dir: &Path) -> Option<Check> {
    if config.engine != TranscriptionEngine::Whisper
        || config.whisper.effective_mode() == WhisperMode::Remote
    {
        return None;
    }
    let model = config.whisper.secondary_model.as_deref()?;
    Some(model_presence_check(
        "transcription.secondary_model",
        "whisper",
        model,
        models_dir,
        Severity::Warn,
    ))
}

/// The Silero model, when VAD is on and resolves to the Whisper backend.
pub fn vad_check(config: &Config, models_dir: &Path) -> Option<Check> {
    if !config.vad.enabled {
        return None;
    }
    if crate::vad::preferred_backend(config) != VadBackend::Whisper {
        return Some(Check::ok(
            "vad.model",
            "VAD enabled (energy backend, no model needed)",
        ));
    }
    let path = crate::vad::whisper_vad_model_path(&config.vad, models_dir);
    Some(if path.exists() {
        Check::ok("vad.model", format!("VAD model: {}", path.display()))
    } else {
        Check::error(
            "vad.model",
            format!(
                "VAD is enabled but its model is missing: {}",
                path.display()
            ),
            "Run: voxtype setup vad   (or set [vad] backend = \"energy\")",
        )
    })
}

/// `[whisper] mode = "cli"` needs the whisper-cli binary, which no voxtype
/// package installs. `problem` is [`crate::transcribe::cli::cli_mode_problem`].
///
/// The id sits under `transcription.` so the daemon's startup summary leaves
/// it to the model-load notice, which says the same thing (Omarchy #18694).
pub fn whisper_cli_check(config: &Config, problem: Option<String>) -> Option<Check> {
    const ID: &str = "transcription.whisper_cli";
    if config.engine != TranscriptionEngine::Whisper
        || config.whisper.effective_mode() != WhisperMode::Cli
    {
        return None;
    }
    Some(match problem {
        None => Check::ok(ID, "Whisper cli mode: whisper-cli found"),
        Some(problem) => Check::error(ID, problem, crate::transcribe::cli::MISSING_CLI_FIX),
    })
}

/// LARD disfluency cleanup, when `text.lard_disfluency` is on.
///
/// The stage fails open: with no model, or a language that is not explicit
/// English, dictation carries on without it. Nothing else notifies about
/// that, so before this check a setting that was on and doing nothing showed
/// only as a warning in the journal (#696). `problem` is what
/// [`super::lard::problem`] found.
pub fn lard_check(config: &Config, problem: Option<&super::lard::Problem>) -> Option<Check> {
    const ID: &str = "text.lard_disfluency";
    if !config.text.lard_disfluency {
        return None;
    }
    Some(match problem {
        None => Check::ok(ID, "LARD disfluency cleanup: model installed and loadable"),
        Some(problem) => Check::error(ID, problem.message(), problem.fix()),
    })
}

/// Can the configured output mode deliver text?
pub fn output_check(status: &OutputChainStatus, config: &Config) -> Check {
    const ID: &str = "output.method";
    let primary = super::configured_primary_method(status, &config.output);

    let install_hint = match status.display_server {
        DisplayServer::Wayland => {
            "Install wtype: sudo pacman -S wtype (or eitype: cargo install eitype)"
        }
        DisplayServer::MacOS => "Grant Accessibility permission to voxtype in System Settings",
        _ => "Install dotool, or ydotool and start ydotoold: sudo pacman -S ydotool",
    };

    match primary.as_deref() {
        None => {
            let mode = match config.output.mode {
                OutputMode::Type => "type",
                OutputMode::Clipboard => "clipboard",
                OutputMode::Paste => "paste",
                OutputMode::File => "file",
            };
            Check::error(
                ID,
                match status.display_server {
                    DisplayServer::Unknown => format!(
                        "No text output method available for mode '{mode}' \
                         (no Wayland or X11 session detected)"
                    ),
                    ds => {
                        format!("No text output method available for mode '{mode}' ({ds} session)")
                    }
                },
                install_hint,
            )
        }
        Some("clipboard") if config.output.mode == OutputMode::Type => Check::warn(
            ID,
            "Only clipboard output available: text is copied, not typed",
            install_hint,
        ),
        Some(method) => Check::ok(ID, format!("Output method: {method}")),
    }
}

/// The daemon types through tools that need the graphical session's env.
///
/// `wayland`/`x11` say whether WAYLAND_DISPLAY/DISPLAY are set in the
/// environment that matters: the daemon's own at startup, the systemd user
/// manager's for `voxtype doctor`.
pub fn display_env_check(wayland: bool, x11: bool, under_systemd: bool) -> Check {
    const ID: &str = "session.display_env";
    if wayland || x11 {
        let which = match (wayland, x11) {
            (true, true) => "WAYLAND_DISPLAY and DISPLAY",
            (true, false) => "WAYLAND_DISPLAY",
            _ => "DISPLAY",
        };
        let place = if under_systemd {
            "systemd user environment"
        } else {
            "environment"
        };
        return Check::ok(ID, format!("{which} set in the {place}"));
    }
    if under_systemd {
        Check::error(
            ID,
            "Neither WAYLAND_DISPLAY nor DISPLAY is set in the systemd user environment, \
             so typing and clipboard output fail",
            "Import them from your session: systemctl --user import-environment WAYLAND_DISPLAY DISPLAY \
             (Hyprland: exec-once = dbus-update-activation-environment --systemd WAYLAND_DISPLAY DISPLAY), \
             then: systemctl --user restart voxtype",
        )
    } else {
        Check::error(
            ID,
            "Neither WAYLAND_DISPLAY nor DISPLAY is set, so typing and clipboard output fail",
            "Start voxtype from inside your graphical session",
        )
    }
}

/// Parse `systemctl --user show-environment` into (WAYLAND_DISPLAY set,
/// DISPLAY set).
pub fn parse_show_environment(text: &str) -> (bool, bool) {
    let has = |key: &str| {
        text.lines().any(|l| {
            l.strip_prefix(key)
                .and_then(|rest| rest.strip_prefix('='))
                .is_some_and(|v| !v.trim().is_empty())
        })
    };
    (has("WAYLAND_DISPLAY"), has("DISPLAY"))
}

/// evdev hotkey: the daemon needs to read at least one /dev/input/event*.
///
/// With the built-in hotkey off, the same access only powers the
/// modifier-release guard. Its absence is then informational: compositor
/// submaps do the guard's job, and the `input` group it would need lets every
/// program read every keystroke.
pub fn input_access_check(hotkey_enabled: bool, total: usize, readable: usize) -> Check {
    const ID: &str = "input.evdev";
    match (hotkey_enabled, readable > 0) {
        (true, true) => Check::ok(
            ID,
            format!("Hotkey: {readable} of {total} input devices readable"),
        ),
        (true, false) if total == 0 => Check::error(
            ID,
            "Built-in hotkey is enabled but no /dev/input/event* devices exist",
            "Use compositor keybindings instead: set [hotkey] enabled = false and bind `voxtype record toggle`",
        ),
        (true, false) => Check::error(
            ID,
            "Built-in hotkey is enabled but no input device is readable (not in the 'input' group?)",
            "Bind `voxtype record toggle` in your compositor and set [hotkey] enabled = false. \
             The 'input' group also works but exposes every keystroke to every program \
             (docs/USER_MANUAL.md, \"The input group trade-off\")",
        ),
        (false, true) => Check::ok(
            ID,
            "Input devices readable (modifier-release guard available)",
        ),
        // The normal state with compositor keybindings, and the safer one:
        // no group grant just for the guard. Submaps cover what it did.
        (false, false) => Check::ok(
            ID,
            "Input devices not readable, as expected with compositor keybindings; the \
             modifier-release guard is off. If held modifiers trigger shortcuts while text is \
             typed, `voxtype setup compositor <hyprland|sway|river>` adds submaps that prevent \
             it, with no input group needed",
        ),
    }
}

/// Desktop notifications are how the daemon reports problems.
pub fn notification_check(notify_send: bool, server: Option<bool>) -> Check {
    const ID: &str = "notifications";
    if !notify_send {
        return Check::warn(
            ID,
            "notify-send not found: voxtype cannot show desktop notifications, including error alerts",
            "Install libnotify (Arch: sudo pacman -S libnotify, Debian/Ubuntu: sudo apt install libnotify-bin)",
        );
    }
    match server {
        Some(false) => Check::warn(
            ID,
            "No notification server on the session bus: notifications will not appear",
            "Install and start one, e.g. mako, dunst or swaync",
        ),
        _ => Check::ok(ID, "Desktop notifications available"),
    }
}

/// Is org.freedesktop.Notifications owned or activatable, per
/// `busctl --user list --no-legend`?
pub fn parse_busctl_has_notifications(text: &str) -> bool {
    text.lines()
        .any(|l| l.split_whitespace().next() == Some("org.freedesktop.Notifications"))
}

/// OSD frontend, resolved the way the daemon resolves it
/// ([`crate::osd::frontend::resolve`]), and, for Quickshell, `qs`.
///
/// Warnings only: the OSD is on by default, dictation works without it, and
/// a user who never asked for it should not get an error notification.
pub fn osd_check(config: &Config, probe: &dyn crate::osd::frontend::OsdProbe) -> Option<Check> {
    use crate::osd::config::OsdFrontend;
    use crate::osd::frontend;
    const ID: &str = "osd.frontend";
    const INSTALL: &str =
        "Install a voxtype package that ships the OSD, or set [osd] enabled = false";
    if !config.osd.enabled {
        return None;
    }
    // The daemon always spawns the `voxtype-osd` launcher.
    if probe
        .find_voxtype_binary(OsdFrontend::Auto.binary_name())
        .is_none()
    {
        return Some(Check::warn(
            ID,
            "OSD is enabled but voxtype-osd is not installed",
            INSTALL,
        ));
    }
    let configured = frontend::effective_configured(config.osd.frontend);
    let resolution = frontend::resolve(configured, probe);
    let Some(active) = resolution.active else {
        return Some(Check::warn(
            ID,
            format!(
                "OSD is enabled but no frontend can run ({})",
                resolution.reason
            ),
            INSTALL,
        ));
    };
    // `auto` only picks Quickshell when `qs` is there; an explicit choice
    // does not check.
    if active == OsdFrontend::Quickshell
        && !frontend::QUICKSHELL_PROGRAMS
            .iter()
            .any(|p| probe.on_path(p))
    {
        return Some(Check::warn(
            ID,
            "OSD uses the Quickshell frontend but `qs` (Quickshell) is not installed",
            "Install quickshell, or set [osd] frontend = \"auto\"",
        ));
    }
    Some(if configured == OsdFrontend::Auto || active == configured {
        Check::ok(
            ID,
            format!("OSD frontend: {} ({})", active.as_str(), resolution.reason),
        )
    } else {
        Check::warn(
            ID,
            format!(
                "OSD frontend {} not installed, falling back to {}",
                configured.binary_name(),
                active.binary_name()
            ),
            format!(
                "Install {}, or set [osd] frontend = \"auto\"",
                configured.binary_name()
            ),
        )
    })
}

/// Meeting mode captures the loopback with `parec` and finds it with `pactl`.
pub fn meeting_check(config: &Config, has_tool: impl Fn(&str) -> bool) -> Option<Check> {
    const ID: &str = "meeting.loopback";
    if !config.meeting.enabled {
        return None;
    }
    let loopback = config.meeting.audio.loopback_device.as_str();
    if loopback == "disabled" {
        return Some(Check::ok(
            ID,
            "Meeting mode: loopback disabled (microphone only)",
        ));
    }
    let mut needed = vec!["parec"];
    if loopback == "auto" {
        needed.push("pactl");
    }
    let missing: Vec<&str> = needed.into_iter().filter(|t| !has_tool(t)).collect();
    Some(if missing.is_empty() {
        Check::ok(ID, "Meeting mode: loopback capture tools available")
    } else {
        Check::warn(
            ID,
            format!(
                "Meeting mode: {} not found, so remote participants will not be recorded",
                missing.join(" and ")
            ),
            "Install PulseAudio utilities (Arch: sudo pacman -S libpulse, Debian/Ubuntu: sudo apt install pulseaudio-utils), \
             or set [meeting.audio] loopback_device = \"disabled\"",
        )
    })
}

/// What `voxtype doctor` saw of Omarchy's local text models.
#[derive(Debug, Clone, Default)]
pub struct OmarchyObservation {
    /// `omarchy-models` (or `omarchy`) was found.
    pub detected: bool,
    /// The settings that choose Omarchy's model, e.g. `[meeting.summary] backend`.
    pub used_by: Vec<&'static str>,
    /// The running model (`recipe at endpoint`), or why there is none.
    pub model: Option<Result<String, String>>,
    /// The key file, or why it cannot be opened.
    pub key: Option<Result<PathBuf, String>>,
    /// Whether the model's endpoint answered `/v1/models` with the key.
    pub endpoint: Option<Result<(), String>>,
}

/// Omarchy's local model: found, running, key readable, endpoint answering.
/// Shown on an Omarchy install, or when config chooses Omarchy anywhere.
pub fn omarchy_checks(obs: &OmarchyObservation) -> Vec<Check> {
    let used = !obs.used_by.is_empty();
    if !obs.detected && !used {
        return Vec::new();
    }
    let used_by = obs.used_by.join(" and ");
    let mut checks = Vec::new();
    if !obs.detected {
        checks.push(Check::error(
            "omarchy.detected",
            format!("Omarchy's model command was not found, but {used_by} = \"omarchy\""),
            "Choose another backend there, e.g. `voxtype config set meeting.summary.backend disabled`",
        ));
        return checks;
    }
    checks.push(Check::ok(
        "omarchy.detected",
        if used {
            format!("Omarchy detected; {used_by} uses its local model")
        } else {
            "Omarchy detected; voxtype is not set to use its local model".to_string()
        },
    ));
    match &obs.model {
        Some(Ok(model)) => checks.push(Check::ok(
            "omarchy.model",
            format!("Omarchy text model running: {model}"),
        )),
        Some(Err(why)) if used => checks.push(Check::warn(
            "omarchy.model",
            why.clone(),
            "Start a text model with `omarchy models start <recipe>` or from the local models \
             panel. Until then voxtype falls back as if Omarchy were not chosen.",
        )),
        Some(Err(_)) | None => checks.push(Check::ok(
            "omarchy.model",
            "No Omarchy text model is running",
        )),
    }
    match &obs.key {
        Some(Ok(path)) => checks.push(Check::ok(
            "omarchy.key",
            format!("Omarchy API key file is readable ({})", path.display()),
        )),
        Some(Err(why)) if used => checks.push(Check::warn(
            "omarchy.key",
            why.clone(),
            "Omarchy writes the key when a model is first set up (`omarchy models use best text`)",
        )),
        _ => {}
    }
    match &obs.endpoint {
        Some(Ok(())) => checks.push(Check::ok(
            "omarchy.endpoint",
            "Omarchy model endpoint answers with the key",
        )),
        Some(Err(why)) => checks.push(Check::warn(
            "omarchy.endpoint",
            why.clone(),
            "Restart the model from Omarchy's local models panel, or see `omarchy models status`",
        )),
        None => {}
    }
    checks
}

/// Look at Omarchy's local model the way voxtype uses it. Runs
/// `omarchy-models status --json` and one `/v1/models` request; never
/// starts a model or runs one.
fn observe_omarchy(config: &Config) -> OmarchyObservation {
    use crate::llm::{omarchy, Provider};
    let mut used_by = Vec::new();
    let summary = config.meeting.summary.llm();
    if summary
        .as_ref()
        .is_some_and(|c| c.provider == Provider::Omarchy)
    {
        used_by.push("[meeting.summary] backend");
    }
    let cleanup = config.output.post_process.as_ref().and_then(|p| p.llm());
    if cleanup
        .as_ref()
        .is_some_and(|c| c.provider == Provider::Omarchy)
    {
        used_by.push("[output.post_process] backend");
    }
    let env = omarchy::OmarchyEnv::from_process();
    let detected = env.models_command.is_some() || omarchy::is_omarchy_install();
    if !detected {
        return OmarchyObservation {
            detected,
            used_by,
            ..Default::default()
        };
    }
    let wanted = summary
        .into_iter()
        .chain(cleanup)
        .find(|c| c.provider == Provider::Omarchy)
        .and_then(|c| c.model);
    let key = Some(omarchy::key_readable(&env).map_err(|e| e.to_string()));
    let found = omarchy::discover(&env, wanted.as_deref());
    let endpoint = found.as_ref().ok().map(|e| {
        crate::llm::list_models(e, std::time::Duration::from_secs(3))
            .map_err(|err| err.to_string())
            .and_then(|ids| {
                if ids.contains(&e.model) {
                    Ok(())
                } else {
                    Err(format!(
                        "{} answers but does not list {}",
                        e.label(),
                        e.model
                    ))
                }
            })
    });
    OmarchyObservation {
        detected,
        used_by,
        model: Some(
            found
                .map(|e| format!("{} at {}", e.model, e.base_url))
                .map_err(|e| e.to_string()),
        ),
        key,
        endpoint,
    }
}

/// The configured capture device, matched the way the daemon matches it
/// (exact, case-insensitive, then substring). `devices` is `None` when the
/// probe failed.
pub fn audio_device_check(configured: &str, devices: Option<&[String]>) -> Check {
    const ID: &str = "audio.device";
    let Some(devices) = devices else {
        return Check::warn(
            ID,
            "Could not list audio input devices (probe failed or timed out)",
            "Check that PipeWire or PulseAudio is running: systemctl --user status pipewire",
        );
    };
    let real: Vec<&String> = devices.iter().filter(|d| *d != "default").collect();
    if real.is_empty() {
        return Check::error(
            ID,
            "No audio input devices found",
            "Connect a microphone and check that PipeWire or PulseAudio is running",
        );
    }
    if configured == "default" {
        return Check::ok(
            ID,
            format!("Audio input: default ({} devices available)", real.len()),
        );
    }
    let lower = configured.to_lowercase();
    let found = real.iter().any(|d| {
        *d == configured || d.to_lowercase() == lower || d.to_lowercase().contains(&lower)
    });
    if found {
        Check::ok(ID, format!("Audio input: {configured}"))
    } else {
        Check::error(
            ID,
            format!("Configured audio device '{configured}' not found"),
            "Pick an available device: voxtype info devices, then voxtype config set audio.device <NAME> \
             (or \"default\")",
        )
    }
}

/// The advice `voxtype setup gpu` prints for an unloadable provider, on one
/// line, so doctor and setup say the same thing.
fn provider_fix(variant: &str, missing: &[String]) -> String {
    let lines = super::gpu::provider_fix_lines(variant, missing);
    if lines.is_empty() {
        return "Install the GPU runtime this variant needs".to_string();
    }
    lines
        .iter()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// GPU runtime libraries for the variant `/usr/bin/voxtype` dispatches to.
///
/// `provider_missing` is [`super::gpu::unresolved_provider_deps`] for ONNX
/// GPU variants; `vulkan_ok` is [`super::gpu::check_vulkan_runtime`].
pub fn gpu_check(
    variant: Option<&str>,
    engine: TranscriptionEngine,
    provider_missing: Option<Vec<String>>,
    vulkan_ok: bool,
) -> Option<Check> {
    const ID: &str = "gpu.runtime";
    let variant = variant?;
    if variant == "voxtype-vulkan" {
        if engine != TranscriptionEngine::Whisper {
            return None;
        }
        return Some(if vulkan_ok {
            Check::ok(ID, "Vulkan runtime available")
        } else {
            Check::error(
                ID,
                "GPU (Vulkan) backend selected but libvulkan.so.1 cannot be loaded",
                "Install the Vulkan loader and your GPU's driver (Arch: sudo pacman -S vulkan-icd-loader, \
                 Debian/Ubuntu: sudo apt install libvulkan1), or switch to CPU: sudo voxtype setup gpu --disable",
            )
        });
    }
    let missing = provider_missing?;
    Some(if missing.is_empty() {
        Check::ok(
            ID,
            format!("{variant}: GPU execution provider libraries resolve"),
        )
    } else {
        Check::error(
            ID,
            format!(
                "{variant}: GPU execution provider cannot load (missing {}), so ONNX Runtime falls back to CPU",
                missing.join(", ")
            ),
            provider_fix(variant, &missing),
        )
    })
}

/// Coding agents that have no voxtype-setup skill, or an outdated one
/// ([`super::skill::needs_attention`]). A hint, never an error; `None` when
/// every detected agent is up to date.
pub fn skill_check(stale: &[(super::skill::Target, super::skill::State)]) -> Option<Check> {
    use super::skill::State;
    if stale.is_empty() {
        return None;
    }
    let message = stale
        .iter()
        .map(|(target, state)| {
            let what = match state {
                State::Outdated => "has an outdated voxtype-setup skill",
                _ => "found, voxtype-setup skill not installed",
            };
            format!("{}: {what}", target.label())
        })
        .collect::<Vec<_>>()
        .join("; ");
    Some(Check::warn(
        "agents.skill",
        message,
        "Install or update it with: voxtype setup skill",
    ))
}

/// State of the `voxtype.service` user unit, from
/// `systemctl --user show voxtype.service -p LoadState -p ActiveState -p UnitFileState`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnitState {
    pub load: String,
    pub active: String,
    pub unit_file: String,
}

impl UnitState {
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let v = v.trim().to_string();
                match k.trim() {
                    "LoadState" => s.load = v,
                    "ActiveState" => s.active = v,
                    "UnitFileState" => s.unit_file = v,
                    _ => {}
                }
            }
        }
        s
    }

    pub fn loaded(&self) -> bool {
        self.load == "loaded"
    }
}

pub fn systemd_unit_check(unit: &UnitState, daemon_running: bool) -> Check {
    const ID: &str = "service.systemd";
    if !unit.loaded() {
        return if daemon_running {
            Check::ok(ID, "Daemon running (not managed by systemd)")
        } else {
            Check::warn(
                ID,
                "Daemon is not running and no systemd user service is installed",
                "Run: voxtype setup systemd   (or start `voxtype` from your compositor's autostart)",
            )
        };
    }
    match unit.active.as_str() {
        "failed" => Check::error(
            ID,
            "voxtype.service has failed",
            "See why: journalctl --user -u voxtype -n 50, then: systemctl --user restart voxtype",
        ),
        "active" | "activating" | "reloading" if unit.unit_file == "disabled" => Check::warn(
            ID,
            "voxtype.service is running but will not start at login",
            "Run: systemctl --user enable voxtype",
        ),
        "active" | "activating" | "reloading" => Check::ok(ID, "voxtype.service is active"),
        _ if daemon_running => Check::ok(
            ID,
            "Daemon running outside voxtype.service (the unit is installed but inactive)",
        ),
        _ => Check::warn(
            ID,
            "voxtype.service is installed but not running",
            "Run: systemctl --user enable --now voxtype",
        ),
    }
}

/// Turn a configured engine the running binary can't serve into a check.
pub fn variant_check(mismatch: Option<&super::variant_check::VariantMismatch>) -> Check {
    const ID: &str = "binary.variant";
    match mismatch {
        None => Check::ok(ID, "Installed binary supports the configured engine"),
        Some(m) => {
            use super::variant_check::Remediation;
            // A source build is judged by its own features, whatever the
            // package symlink points at.
            let binary = match (&m.remediation, m.active_variant_name.as_deref()) {
                (Remediation::Rebuild { .. }, _) | (_, None) => "this voxtype build",
                (_, Some(name)) => name,
            };
            Check::error(
                ID,
                format!(
                    "Engine '{}' is configured but {binary} cannot run it",
                    m.configured_engine
                ),
                m.explanation(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Gathering
// ---------------------------------------------------------------------------

fn on_path(tool: &str) -> bool {
    which::which(tool).is_ok()
}

/// (total, readable) `/dev/input/event*` nodes.
fn input_device_access() -> (usize, usize) {
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return (0, 0);
    };
    let events: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("event"))
        })
        .collect();
    let readable = events
        .iter()
        .filter(|p| std::fs::File::open(p).is_ok())
        .count();
    (events.len(), readable)
}

fn command_stdout(cmd: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn env_set(key: &str) -> bool {
    std::env::var_os(key).is_some_and(|v| !v.is_empty())
}

/// Run the checks for `scope`. `config_path` is the `--config` file, if one
/// was given, so the config-file check looks at the file actually in use.
pub async fn run(config: &Config, config_path: Option<&Path>, scope: Scope) -> Report {
    let mut checks = Vec::new();
    let models_dir = Config::models_dir();
    let linux = cfg!(target_os = "linux");

    if scope == Scope::Full {
        checks.push(cpu_check(
            crate::cpu::check_cpu_compatibility(),
            crate::cpu::is_running_in_vm(),
        ));
        if let Some(dir) = Config::config_dir() {
            let exists = dir.exists();
            checks.push(path_check("config.dir", "Config directory", &dir, exists));
        }
        let exists = models_dir.exists();
        checks.push(path_check(
            "config.models_dir",
            "Models directory",
            &models_dir,
            exists,
        ));
        if let Some(path) = config_path
            .map(Path::to_path_buf)
            .or_else(Config::default_path)
        {
            let exists = path.exists();
            checks.push(path_check("config.file", "Config file", &path, exists));
        }
        let inventory = super::binary::inventory();
        checks.push(variant_check(
            super::variant_check::detect_mismatch(config, &inventory).as_ref(),
        ));

        if linux {
            // What `/usr/bin/voxtype` dispatches to. Source builds have no
            // dispatcher; only their Vulkan feature is detectable.
            let variant = inventory
                .active_variant
                .map(|v| v.binary_name().to_string())
                .or_else(|| cfg!(feature = "gpu-vulkan").then(|| "voxtype-vulkan".to_string()));
            let provider_missing = variant
                .as_deref()
                .filter(|v| v.starts_with("voxtype-onnx-"))
                .and_then(super::gpu::unresolved_provider_deps);
            let vulkan_ok =
                variant.as_deref() == Some("voxtype-vulkan") && super::gpu::check_vulkan_runtime();
            checks.extend(gpu_check(
                variant.as_deref(),
                config.engine,
                provider_missing,
                vulkan_ok,
            ));
        }
    }

    let api_key_env = match config.engine {
        TranscriptionEngine::ElevenLabs => "ELEVENLABS_API_KEY",
        _ => "SONIOX_API_KEY",
    };
    checks.push(engine_model_check(
        config,
        &models_dir,
        env_set(api_key_env),
    ));
    checks.extend(secondary_model_check(config, &models_dir));
    checks.extend(vad_check(config, &models_dir));
    checks.extend(lard_check(config, super::lard::problem(config).as_ref()));
    checks.extend(whisper_cli_check(
        config,
        crate::transcribe::cli::cli_mode_problem(&config.whisper),
    ));

    if scope == Scope::Full {
        // Probed in a child process that is killed if the sound server hangs.
        let devices =
            tokio::task::spawn_blocking(|| crate::audio::devices::enumerate_input_devices().ok())
                .await
                .ok()
                .flatten();
        checks.push(audio_device_check(&config.audio.device, devices.as_deref()));
    }

    // `None` when systemctl is unavailable (non-systemd systems, macOS).
    let unit = (scope == Scope::Full && linux)
        .then(|| {
            command_stdout(
                "systemctl",
                &[
                    "--user",
                    "show",
                    "voxtype.service",
                    "-p",
                    "LoadState",
                    "-p",
                    "ActiveState",
                    "-p",
                    "UnitFileState",
                ],
            )
        })
        .flatten()
        .map(|s| UnitState::parse(&s));

    if linux && config.output.mode != OutputMode::File {
        match scope {
            Scope::Startup => checks.push(display_env_check(
                env_set("WAYLAND_DISPLAY"),
                env_set("DISPLAY"),
                env_set("INVOCATION_ID"),
            )),
            // The terminal's env says nothing about the service's; ask the
            // user manager when voxtype runs under it.
            Scope::Full if unit.as_ref().is_some_and(UnitState::loaded) => {
                if let Some(env) = command_stdout("systemctl", &["--user", "show-environment"]) {
                    let (wayland, x11) = parse_show_environment(&env);
                    checks.push(display_env_check(wayland, x11, true));
                }
            }
            Scope::Full => {}
        }
    }

    if config.output.mode != OutputMode::File {
        let status = match scope {
            Scope::Full => super::detect_output_chain().await,
            Scope::Startup => super::detect_output_chain_quick(),
        };
        checks.push(output_check(&status, config));
    }

    // At startup the evdev listener reports its own failure (and exits), so
    // only the full report covers input access and notifications.
    if scope == Scope::Full && linux {
        let (total, readable) = input_device_access();
        checks.push(input_access_check(config.hotkey.enabled, total, readable));

        let server = on_path("busctl")
            .then(|| command_stdout("busctl", &["--user", "list", "--no-legend", "--no-pager"]))
            .flatten()
            .map(|s| parse_busctl_has_notifications(&s));
        checks.push(notification_check(on_path("notify-send"), server));
    }

    checks.extend(osd_check(config, &crate::osd::frontend::SystemProbe));
    if linux {
        checks.extend(meeting_check(config, on_path));
    }
    if scope == Scope::Full && linux {
        let config = config.clone();
        let observed = tokio::task::spawn_blocking(move || observe_omarchy(&config))
            .await
            .unwrap_or_default();
        checks.extend(omarchy_checks(&observed));
    }

    if let Some(unit) = &unit {
        checks.push(systemd_unit_check(
            unit,
            crate::daemon_status::is_daemon_running(),
        ));
    }

    if scope == Scope::Full {
        checks.extend(skill_check(&super::skill::needs_attention()));
    }

    Report { checks }
}

// ---------------------------------------------------------------------------
// Presentation
// ---------------------------------------------------------------------------

/// Section heading for a check id's area.
fn heading(id: &str) -> &'static str {
    match id.split('.').next().unwrap_or("") {
        "system" => "System",
        "config" => "Configuration",
        "binary" | "gpu" => "Binary and GPU",
        "transcription" | "vad" => "Transcription",
        "text" => "Text cleanup",
        "audio" => "Audio",
        "session" | "output" => "Output",
        "input" => "Input",
        "notifications" => "Notifications",
        "osd" => "OSD",
        "meeting" => "Meeting mode",
        "omarchy" => "Omarchy local AI",
        "service" => "Service",
        "agents" => "Coding agents",
        _ => "Other",
    }
}

/// Render the report for a terminal. `color` adds ANSI colors to the marks.
pub fn render_text(report: &Report, color: bool) -> String {
    let paint = |code: &str, s: &str| {
        if color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };
    let mut out = String::from("Voxtype System Check\n====================\n");
    let mut current = "";
    for check in &report.checks {
        let h = heading(check.id);
        if h != current {
            out.push_str(&format!("\n{h}:\n"));
            current = h;
        }
        let mark = match check.severity {
            Severity::Ok => paint("32", "✓"),
            Severity::Warn => paint("33", "!"),
            Severity::Error => paint("31", "✗"),
        };
        out.push_str(&format!("  {mark} {}\n", check.message));
        if let Some(fix) = &check.fix {
            out.push_str(&format!("      Fix: {fix}\n"));
        }
    }

    let errors = report.count(Severity::Error);
    let warnings = report.count(Severity::Warn);
    out.push_str("\n---\n");
    if errors == 0 {
        out.push_str(&paint("32", "✓ All checks passed!"));
        if warnings > 0 {
            out.push_str(&format!(" ({warnings} warning(s) above)"));
        }
    } else {
        out.push_str(&paint("31", "✗ Some checks failed."));
        out.push_str(&format!(
            " {errors} error(s), {warnings} warning(s). Please fix the issues above."
        ));
    }
    out.push('\n');
    out
}

/// Render the report as JSON for scripts.
pub fn render_json(report: &Report) -> String {
    let value = serde_json::json!({
        "ok": !report.has_errors(),
        "errors": report.count(Severity::Error),
        "warnings": report.count(Severity::Warn),
        "checks": report.checks,
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// Checks the startup notification leaves to someone else. Model load
/// failures get their own notification from the transcriber path at startup
/// and at record time, and VAD model failures from VAD setup, so repeating
/// them here would notify twice. They are still logged at startup and shown
/// by `voxtype doctor`.
fn notified_elsewhere(id: &str) -> bool {
    id.starts_with("transcription.") || id.starts_with("vad.")
}

/// The errors the startup notification reports.
fn notifiable_errors(report: &Report) -> Vec<&Check> {
    report
        .errors()
        .filter(|c| !notified_elsewhere(c.id))
        .collect()
}

/// Identity of a set of failures: the sorted, de-duplicated check ids. The
/// marker file stores this so a systemd restart loop with the same failures
/// notifies once, and a different set of failures notifies again.
fn failure_key(errors: &[&Check]) -> String {
    let mut ids: Vec<&str> = errors.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.join(",")
}

/// Marker in the runtime dir holding the last notified [`failure_key`].
/// The runtime dir is per boot (tmpfs), so a reboot notifies again.
fn startup_marker_path() -> PathBuf {
    Config::runtime_dir().join("doctor-startup-notified")
}

/// What to do with the marker and whether to notify, given the previous
/// marker contents and this start's failures.
#[derive(Debug, PartialEq, Eq)]
enum MarkerAction {
    /// Clean start: remove the marker so the next failure notifies.
    Clear,
    /// Same failures as last time: stay quiet.
    Keep,
    /// New or changed failures: notify and record this key.
    Notify(String),
}

fn marker_action(previous: Option<&str>, errors: &[&Check]) -> MarkerAction {
    if errors.is_empty() {
        return MarkerAction::Clear;
    }
    let key = failure_key(errors);
    if previous.map(str::trim) == Some(key.as_str()) {
        MarkerAction::Keep
    } else {
        MarkerAction::Notify(key)
    }
}

/// Title and body of the daemon's grouped startup notification, or `None`
/// when there are no errors to report. Warnings stay in the log and in
/// `voxtype doctor`.
pub fn startup_notification(report: &Report) -> Option<(String, String)> {
    let errors = notifiable_errors(report);
    if errors.is_empty() {
        return None;
    }
    let title = if errors.len() == 1 {
        "Voxtype: 1 problem needs attention".to_string()
    } else {
        format!("Voxtype: {} problems need attention", errors.len())
    };
    let mut body = String::new();
    for check in &errors {
        body.push_str(&format!("• {}", check.message));
        if let Some(fix) = &check.fix {
            body.push_str(&format!("\n  {fix}"));
        }
        body.push('\n');
    }
    body.push_str("Run `voxtype doctor` for details.");
    Some((title, body))
}

/// Daemon startup: run the cheap checks, log every problem, and send one
/// notification for the errors.
pub async fn startup_summary(config: &Config) {
    let report = run(config, None, Scope::Startup).await;
    for check in &report.checks {
        let fix = check.fix.as_deref().unwrap_or("");
        match check.severity {
            Severity::Error => tracing::error!(check = check.id, "{} Fix: {}", check.message, fix),
            Severity::Warn => tracing::warn!(check = check.id, "{} Fix: {}", check.message, fix),
            Severity::Ok => tracing::debug!(check = check.id, "{}", check.message),
        }
    }

    let marker = startup_marker_path();
    let previous = std::fs::read_to_string(&marker).ok();
    match marker_action(previous.as_deref(), &notifiable_errors(&report)) {
        MarkerAction::Clear => {
            let _ = std::fs::remove_file(&marker);
        }
        MarkerAction::Keep => {
            tracing::info!(
                "Startup problems unchanged since the last notification; not notifying again"
            );
        }
        MarkerAction::Notify(key) => {
            if let Some((title, body)) = startup_notification(&report) {
                crate::notification::send_sync(&title, &body);
            }
            if let Some(dir) = marker.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&marker, key) {
                tracing::debug!("Could not write {:?}: {}", marker, e);
            }
        }
    }
}

/// `voxtype doctor` / `voxtype setup check`. Returns whether any check
/// errored, for the caller to turn into an exit code.
pub async fn run_cli(config: &Config, config_path: Option<&Path>, json: bool) -> bool {
    let report = run(config, config_path, Scope::Full).await;
    if json {
        println!("{}", render_json(&report));
    } else {
        use std::io::IsTerminal;
        print!("{}", render_text(&report, std::io::stdout().is_terminal()));
    }
    report.has_errors()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(display: DisplayServer, tools: &[&str]) -> OutputChainStatus {
        super::super::build_output_chain(
            display,
            true,
            |t| tools.contains(&t).then(|| format!("/usr/bin/{t}")),
            true,
        )
    }

    #[test]
    fn severity_orders_ok_warn_error() {
        assert!(Severity::Ok < Severity::Warn && Severity::Warn < Severity::Error);
    }

    #[test]
    fn whisper_model_missing_is_an_error_with_download_fix() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();
        let check = engine_model_check(&config, dir.path(), false);
        assert_eq!(check.severity, Severity::Error);
        assert!(check.message.contains("base.en"), "{}", check.message);
        assert_eq!(
            check.fix.as_deref(),
            Some("Run: voxtype setup --download --model base.en")
        );
    }

    #[test]
    fn whisper_model_with_bad_magic_is_reported_damaged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ggml-base.en.bin"), b"<html>nope</html>").unwrap();
        let check = engine_model_check(&Config::default(), dir.path(), false);
        assert_eq!(check.severity, Severity::Error);
        assert!(check.message.contains("damaged"), "{}", check.message);
    }

    #[test]
    fn explicit_model_path_is_checked_directly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom.bin");
        std::fs::write(&path, b"x").unwrap();
        let mut config = Config::default();
        config.whisper.model = path.display().to_string();
        assert_eq!(
            engine_model_check(&config, dir.path(), false).severity,
            Severity::Ok
        );
    }

    #[test]
    fn engine_without_section_is_an_error() {
        let config = Config {
            engine: TranscriptionEngine::Parakeet,
            parakeet: None,
            ..Default::default()
        };
        let check = engine_model_check(&config, Path::new("/nonexistent"), false);
        assert_eq!(check.severity, Severity::Error);
        assert!(check.message.contains("[parakeet]"));
    }

    #[test]
    fn soniox_needs_a_key_from_config_or_env() {
        let config = Config {
            engine: TranscriptionEngine::Soniox,
            soniox: Some(Default::default()),
            ..Default::default()
        };
        let dir = Path::new("/nonexistent");
        assert_eq!(
            engine_model_check(&config, dir, false).severity,
            Severity::Error
        );
        assert_eq!(
            engine_model_check(&config, dir, true).severity,
            Severity::Ok
        );
    }

    #[test]
    fn whisper_remote_needs_no_model() {
        let mut config = Config::default();
        config.whisper.mode = Some(WhisperMode::Remote);
        config.whisper.remote_endpoint = Some("http://localhost:8080".into());
        assert_eq!(
            engine_model_check(&config, Path::new("/nonexistent"), false).severity,
            Severity::Ok
        );
    }

    #[test]
    fn vad_check_only_runs_when_enabled_and_needs_a_model() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        assert!(vad_check(&config, dir.path()).is_none());

        config.vad.enabled = true;
        let check = vad_check(&config, dir.path()).unwrap();
        assert_eq!(check.severity, Severity::Error);
        assert!(check.fix.unwrap().contains("voxtype setup vad"));

        std::fs::write(dir.path().join("ggml-silero-vad.bin"), b"x").unwrap();
        assert_eq!(
            vad_check(&config, dir.path()).unwrap().severity,
            Severity::Ok
        );

        config.vad.backend = VadBackend::Energy;
        std::fs::remove_file(dir.path().join("ggml-silero-vad.bin")).unwrap();
        assert_eq!(
            vad_check(&config, dir.path()).unwrap().severity,
            Severity::Ok
        );
    }

    fn lard_config(model: &Path) -> Config {
        let mut config = Config::default();
        config.text.lard_disfluency = true;
        config.text.lard_model_path = Some(model.to_path_buf());
        config.whisper.language = crate::config::LanguageConfig::Single("en".into());
        config
    }

    #[test]
    fn lard_check_is_silent_while_the_stage_is_off() {
        let mut config = lard_config(Path::new("/nonexistent/voxtype-test/lard.vxdf"));
        config.text.lard_disfluency = false;
        let problem = crate::setup::lard::problem(&config);
        assert_eq!(lard_check(&config, problem.as_ref()), None);
    }

    #[test]
    fn lard_check_errors_when_the_model_is_missing() {
        let config = lard_config(Path::new("/nonexistent/voxtype-test/lard.vxdf"));
        let problem = crate::setup::lard::problem(&config);
        let check = lard_check(&config, problem.as_ref()).unwrap();
        assert_eq!(check.id, "text.lard_disfluency");
        assert_eq!(check.severity, Severity::Error);
        assert!(check
            .message
            .contains("/nonexistent/voxtype-test/lard.vxdf"));
        assert!(check
            .fix
            .unwrap()
            .contains("voxtype config set text.lard_disfluency false"));
    }

    /// What a user with the setting on and no model sees, to the letter.
    #[test]
    fn lard_check_tells_the_user_to_run_setup_lard() {
        let config = lard_config(Path::new("unused"));
        let problem = crate::setup::lard::Problem::ModelMissing {
            path: PathBuf::from("/home/u/.local/share/voxtype/models/lard-disfluency-v1.vxdf"),
            custom: false,
        };
        let check = lard_check(&config, Some(&problem)).unwrap();
        assert_eq!(check.severity, Severity::Error);
        assert_eq!(
            render_text(&report(vec![check.clone()]), false),
            "Voxtype System Check\n\
             ====================\n\
             \n\
             Text cleanup:\n  \
               ✗ LARD disfluency cleanup is on but its model is not installed \
             (/home/u/.local/share/voxtype/models/lard-disfluency-v1.vxdf)\n      \
                   Fix: Run: voxtype setup lard, or turn it off: \
             voxtype config set text.lard_disfluency false\n\
             \n\
             ---\n\
             ✗ Some checks failed. 1 error(s), 0 warning(s). Please fix the issues above.\n"
        );

        let (title, body) = startup_notification(&report(vec![check])).unwrap();
        assert_eq!(title, "Voxtype: 1 problem needs attention");
        assert_eq!(
            body,
            "• LARD disfluency cleanup is on but its model is not installed \
             (/home/u/.local/share/voxtype/models/lard-disfluency-v1.vxdf)\n  \
               Run: voxtype setup lard, or turn it off: \
             voxtype config set text.lard_disfluency false\n\
             Run `voxtype doctor` for details."
        );
    }

    #[test]
    fn lard_check_errors_when_the_language_is_not_english() {
        let model =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(crate::setup::lard::SOURCE_TREE_PATH);
        let mut config = lard_config(&model);
        let problem = crate::setup::lard::problem(&config);
        assert_eq!(
            lard_check(&config, problem.as_ref()).unwrap().severity,
            Severity::Ok
        );

        // A multilingual model on automatic detection: nothing says the
        // dictation is English, and the fix is the command that says so.
        config.whisper.model = "large-v3-turbo".into();
        config.whisper.language = crate::config::LanguageConfig::Single("auto".into());
        let problem = crate::setup::lard::problem(&config);
        let check = lard_check(&config, problem.as_ref()).unwrap();
        assert_eq!(check.severity, Severity::Error);
        assert_eq!(
            check.message,
            "LARD disfluency cleanup is on but is being skipped: it only runs on English, \
             and nothing says whisper dictation with large-v3-turbo is English"
        );
        assert_eq!(
            check.fix.as_deref(),
            Some(
                "If you dictate in English, declare it: voxtype config set text.language en, \
                 or turn it off: voxtype config set text.lard_disfluency false"
            )
        );
        let (_, body) = startup_notification(&report(vec![check])).unwrap();
        assert!(body.contains("voxtype config set text.language en"));

        // Declaring it, or an English-only model, is all it takes.
        config.text.language = "en".into();
        let problem = crate::setup::lard::problem(&config);
        assert_eq!(
            lard_check(&config, problem.as_ref()).unwrap().severity,
            Severity::Ok
        );
        config.text.language = "auto".into();
        config.whisper.model = "base.en".into();
        let problem = crate::setup::lard::problem(&config);
        assert_eq!(
            lard_check(&config, problem.as_ref()).unwrap().severity,
            Severity::Ok
        );
    }

    /// Nothing else in the daemon notifies about a skipped LARD stage, so it
    /// belongs in the grouped startup notification, once per failure set.
    #[test]
    fn lard_problem_is_in_the_startup_notification_until_fixed() {
        let config = lard_config(Path::new("/nonexistent/voxtype-test/lard.vxdf"));
        let problem = crate::setup::lard::problem(&config);
        let check = lard_check(&config, problem.as_ref()).unwrap();
        let r = report(vec![check.clone()]);

        let (title, body) = startup_notification(&r).unwrap();
        assert_eq!(title, "Voxtype: 1 problem needs attention");
        assert!(body.contains("LARD disfluency cleanup is on but"));
        assert!(body.contains("voxtype config set text.lard_disfluency false"));

        assert_eq!(
            marker_action(None, &notifiable_errors(&r)),
            MarkerAction::Notify("text.lard_disfluency".into())
        );
        assert_eq!(
            marker_action(Some("text.lard_disfluency"), &notifiable_errors(&r)),
            MarkerAction::Keep
        );
        // Installing the model (or turning the stage off) clears the marker,
        // so a later regression notifies again.
        let fixed = report(vec![Check::ok(check.id, "ok")]);
        assert_eq!(
            marker_action(Some("text.lard_disfluency"), &notifiable_errors(&fixed)),
            MarkerAction::Clear
        );
    }

    /// Omarchy #18694: cli mode without whisper-cli must show up in doctor
    /// and `setup check` with both fixes, and only in cli mode.
    #[test]
    fn whisper_cli_check_reports_cli_mode_without_the_binary() {
        let mut config = Config::default();
        assert!(whisper_cli_check(&config, Some("x".into())).is_none());

        config.whisper.mode = Some(WhisperMode::Cli);
        let check = whisper_cli_check(&config, Some("whisper-cli missing".into())).unwrap();
        assert_eq!(check.severity, Severity::Error);
        assert_eq!(heading(check.id), "Transcription");
        let fix = check.fix.unwrap();
        assert!(fix.contains("whisper-cpp"), "{fix}");
        assert!(
            fix.contains("voxtype config set whisper.mode local"),
            "{fix}"
        );
        // The model-load notice reports it at startup, so the grouped
        // startup notification must not repeat it.
        assert!(notified_elsewhere(check.id));

        let ok = whisper_cli_check(&config, None).unwrap();
        assert_eq!(ok.severity, Severity::Ok);

        config.engine = TranscriptionEngine::Parakeet;
        assert!(whisper_cli_check(&config, Some("x".into())).is_none());
    }

    #[test]
    fn lard_check_has_its_own_heading() {
        let r = report(vec![Check::error(
            "text.lard_disfluency",
            "LARD missing",
            "install it",
        )]);
        let text = render_text(&r, false);
        assert!(text.contains("\nText cleanup:\n  ✗ LARD missing\n      Fix: install it"));
    }

    #[test]
    fn output_check_errors_when_nothing_can_type() {
        let config = Config::default();
        let check = output_check(&chain(DisplayServer::Wayland, &[]), &config);
        assert_eq!(check.severity, Severity::Error);
        assert!(check.fix.unwrap().contains("wtype"));
    }

    #[test]
    fn output_check_warns_on_clipboard_only() {
        let config = Config::default();
        let check = output_check(&chain(DisplayServer::Wayland, &["wl-copy"]), &config);
        assert_eq!(check.severity, Severity::Warn);
    }

    #[test]
    fn output_check_ok_with_wtype() {
        let config = Config::default();
        let check = output_check(&chain(DisplayServer::Wayland, &["wtype"]), &config);
        assert_eq!(check.severity, Severity::Ok);
        assert!(check.message.contains("wtype"));
    }

    #[test]
    fn build_output_chain_respects_display_server() {
        // wtype is installed but useless outside Wayland.
        let status = chain(DisplayServer::X11, &["wtype"]);
        assert!(status.wtype.installed && !status.wtype.available);
        assert_eq!(status.primary_method, None);
    }

    #[test]
    fn display_env_check_errors_without_display_vars() {
        assert_eq!(display_env_check(true, false, true).severity, Severity::Ok);
        let check = display_env_check(false, false, true);
        assert_eq!(check.severity, Severity::Error);
        assert!(check.fix.unwrap().contains("import-environment"));
        assert_eq!(
            display_env_check(false, false, false).severity,
            Severity::Error
        );
    }

    #[test]
    fn show_environment_parsing() {
        let text = "HOME=/home/u\nWAYLAND_DISPLAY=wayland-1\nXDG_RUNTIME_DIR=/run/user/1000\n";
        assert_eq!(parse_show_environment(text), (true, false));
        assert_eq!(parse_show_environment("DISPLAY=:0\n"), (false, true));
        assert_eq!(parse_show_environment("DISPLAY=\n"), (false, false));
        // A key that merely starts with DISPLAY does not count.
        assert_eq!(parse_show_environment("DISPLAYX=1\n"), (false, false));
    }

    #[test]
    fn input_access_severity_depends_on_hotkey() {
        assert_eq!(input_access_check(true, 10, 3).severity, Severity::Ok);
        assert_eq!(input_access_check(true, 10, 0).severity, Severity::Error);
        assert_eq!(input_access_check(true, 0, 0).severity, Severity::Error);
        // Compositor keybindings without the input group: fine, and the
        // message points at submaps rather than the group.
        let off = input_access_check(false, 10, 0);
        assert_eq!(off.severity, Severity::Ok);
        assert!(off.message.contains("voxtype setup compositor"));
        assert!(!off.message.contains("usermod"));
        let on = input_access_check(true, 10, 0);
        let fix = on.fix.unwrap();
        assert!(fix.starts_with("Bind `voxtype record toggle`"), "{fix}");
        assert!(!fix.contains("usermod"), "{fix}");
        assert_eq!(input_access_check(false, 10, 1).severity, Severity::Ok);
    }

    #[test]
    fn notification_checks() {
        assert_eq!(notification_check(false, None).severity, Severity::Warn);
        assert_eq!(
            notification_check(true, Some(false)).severity,
            Severity::Warn
        );
        assert_eq!(notification_check(true, Some(true)).severity, Severity::Ok);
        assert_eq!(notification_check(true, None).severity, Severity::Ok);
    }

    #[test]
    fn busctl_parsing_finds_notification_service() {
        let text = "org.freedesktop.DBus 1 systemd user - - - -\n\
                    org.freedesktop.Notifications 2345 mako user :1.5 - - -\n";
        assert!(parse_busctl_has_notifications(text));
        assert!(!parse_busctl_has_notifications(
            "org.freedesktop.NotificationsX - - - (activatable) - -\n"
        ));
    }

    /// What the OSD resolver sees: installed voxtype binaries, programs on
    /// PATH, and whether a Wayland session is up.
    struct FakeOsd {
        bins: &'static [&'static str],
        on_path: &'static [&'static str],
        wayland: bool,
    }

    impl crate::osd::frontend::OsdProbe for FakeOsd {
        fn find_voxtype_binary(&self, name: &str) -> Option<PathBuf> {
            self.bins
                .contains(&name)
                .then(|| PathBuf::from(format!("/usr/bin/{name}")))
        }
        fn on_path(&self, name: &str) -> bool {
            self.on_path.contains(&name)
        }
        fn wayland_session(&self) -> bool {
            self.wayland
        }
        fn display_present(&self) -> bool {
            self.wayland
        }
        fn missing_libraries(&self, _binary: &Path) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn osd_check_resolves_like_the_daemon() {
        use crate::osd::config::OsdFrontend;
        const ALL: &[&str] = &[
            "voxtype-osd",
            "voxtype-osd-gtk4",
            "voxtype-osd-native",
            "voxtype-osd-quickshell",
        ];
        let everything = FakeOsd {
            bins: ALL,
            on_path: &["qs"],
            wayland: true,
        };
        let mut config = Config::default();
        config.osd.enabled = false;
        assert!(osd_check(&config, &everything).is_none(), "OSD disabled");

        config.osd.enabled = true;
        config.osd.frontend = OsdFrontend::Auto;
        assert_eq!(
            osd_check(&config, &everything).unwrap().severity,
            Severity::Ok
        );

        let nothing = FakeOsd {
            bins: &[],
            on_path: &[],
            wayland: true,
        };
        assert_eq!(
            osd_check(&config, &nothing).unwrap().severity,
            Severity::Warn
        );

        // auto falls back to native without a display: works, no warning.
        let native_only = FakeOsd {
            bins: &["voxtype-osd", "voxtype-osd-native"],
            on_path: &[],
            wayland: false,
        };
        assert_eq!(
            osd_check(&config, &native_only).unwrap().severity,
            Severity::Ok
        );

        // Explicit gtk4 missing, native present: works, with a warning.
        config.osd.frontend = OsdFrontend::Gtk4;
        assert_eq!(
            osd_check(&config, &native_only).unwrap().severity,
            Severity::Warn
        );

        // Explicit Quickshell without qs.
        config.osd.frontend = OsdFrontend::Quickshell;
        let qs_launcher_only = FakeOsd {
            bins: &["voxtype-osd", "voxtype-osd-quickshell"],
            on_path: &[],
            wayland: true,
        };
        let check = osd_check(&config, &qs_launcher_only).unwrap();
        assert_eq!(check.severity, Severity::Warn);
        assert!(check.message.contains("qs"));
    }

    #[test]
    fn skill_check_lists_stale_agents_as_a_warning() {
        use crate::setup::skill::{State, Target};
        assert!(skill_check(&[]).is_none());
        let check = skill_check(&[
            (Target::Claude, State::Outdated),
            (Target::Agents, State::NotInstalled),
        ])
        .unwrap();
        assert_eq!(check.severity, Severity::Warn);
        assert!(check.message.contains("Claude Code: has an outdated"));
        assert!(check.message.contains("not installed"));
        assert!(check.fix.unwrap().contains("voxtype setup skill"));
    }

    #[test]
    fn meeting_check_needs_pulse_tools_for_loopback() {
        let mut config = Config::default();
        assert!(meeting_check(&config, |_| false).is_none());
        config.meeting.enabled = true;
        config.meeting.audio.loopback_device = "auto".into();
        let check = meeting_check(&config, |t| t == "parec").unwrap();
        assert_eq!(check.severity, Severity::Warn);
        assert!(check.message.contains("pactl"));
        assert_eq!(
            meeting_check(&config, |_| true).unwrap().severity,
            Severity::Ok
        );
        config.meeting.audio.loopback_device = "disabled".into();
        assert_eq!(
            meeting_check(&config, |_| false).unwrap().severity,
            Severity::Ok
        );
    }

    #[test]
    fn audio_device_matching_follows_the_daemon() {
        let devices: Vec<String> = [
            "default",
            "pipewire",
            "alsa_input.usb-Blue_Yeti-00.analog-stereo",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            audio_device_check("default", Some(&devices)).severity,
            Severity::Ok
        );
        assert_eq!(
            audio_device_check("blue_yeti", Some(&devices)).severity,
            Severity::Ok
        );
        assert_eq!(
            audio_device_check("Rode", Some(&devices)).severity,
            Severity::Error
        );
        assert_eq!(
            audio_device_check("default", Some(&["default".to_string()])).severity,
            Severity::Error
        );
        assert_eq!(audio_device_check("default", None).severity, Severity::Warn);
    }

    #[test]
    fn gpu_check_reports_missing_provider_libs() {
        let check = gpu_check(
            Some("voxtype-onnx-migraphx"),
            TranscriptionEngine::Parakeet,
            Some(vec!["libmigraphx_c.so.3".into()]),
            false,
        )
        .unwrap();
        assert_eq!(check.severity, Severity::Error);
        assert!(check.fix.unwrap().contains("migraphx"));

        assert!(gpu_check(None, TranscriptionEngine::Whisper, None, false).is_none());
        assert_eq!(
            gpu_check(
                Some("voxtype-vulkan"),
                TranscriptionEngine::Whisper,
                None,
                false
            )
            .unwrap()
            .severity,
            Severity::Error
        );
        assert_eq!(
            gpu_check(
                Some("voxtype-vulkan"),
                TranscriptionEngine::Whisper,
                None,
                true
            )
            .unwrap()
            .severity,
            Severity::Ok
        );
        // CPU variants have nothing to check.
        assert!(gpu_check(
            Some("voxtype-avx2"),
            TranscriptionEngine::Whisper,
            None,
            true
        )
        .is_none());
    }

    #[test]
    fn systemd_unit_states() {
        let parse = |s: &str| UnitState::parse(s);
        let not_found = parse("LoadState=not-found\nActiveState=inactive\nUnitFileState=\n");
        assert_eq!(systemd_unit_check(&not_found, true).severity, Severity::Ok);
        assert_eq!(
            systemd_unit_check(&not_found, false).severity,
            Severity::Warn
        );

        let active = parse("LoadState=loaded\nActiveState=active\nUnitFileState=enabled\n");
        assert_eq!(systemd_unit_check(&active, true).severity, Severity::Ok);

        let failed = parse("LoadState=loaded\nActiveState=failed\nUnitFileState=enabled\n");
        assert_eq!(systemd_unit_check(&failed, false).severity, Severity::Error);

        let disabled = parse("LoadState=loaded\nActiveState=active\nUnitFileState=disabled\n");
        assert_eq!(systemd_unit_check(&disabled, true).severity, Severity::Warn);

        let inactive = parse("LoadState=loaded\nActiveState=inactive\nUnitFileState=enabled\n");
        assert_eq!(
            systemd_unit_check(&inactive, false).severity,
            Severity::Warn
        );
        assert_eq!(systemd_unit_check(&inactive, true).severity, Severity::Ok);
    }

    fn report(checks: Vec<Check>) -> Report {
        Report { checks }
    }

    #[test]
    fn startup_notification_groups_errors_only() {
        let r = report(vec![
            Check::ok("a.ok", "fine"),
            Check::warn("b.warn", "meh", "do x"),
        ]);
        assert!(startup_notification(&r).is_none());

        let r = report(vec![
            Check::error("osd.frontend", "OSD broken", "Install the OSD"),
            Check::warn("b.warn", "meh", "do x"),
            Check::error("output.method", "No output", "Install wtype"),
        ]);
        let (title, body) = startup_notification(&r).unwrap();
        assert_eq!(title, "Voxtype: 2 problems need attention");
        assert!(body.contains("OSD broken"));
        assert!(body.contains("Install the OSD"));
        assert!(body.contains("No output"));
        assert!(
            !body.contains("meh"),
            "warnings stay out of the notification"
        );
        assert!(body.ends_with("Run `voxtype doctor` for details."));
    }

    #[test]
    fn model_and_vad_errors_are_left_to_their_own_notifications() {
        let r = report(vec![
            Check::error("transcription.model", "model missing", "download it"),
            Check::error("vad.model", "VAD missing", "voxtype setup vad"),
        ]);
        assert!(startup_notification(&r).is_none());
        assert_eq!(
            marker_action(None, &notifiable_errors(&r)),
            MarkerAction::Clear
        );

        let r = report(vec![
            Check::error("transcription.model", "model missing", "download it"),
            Check::error("vad.model", "VAD missing", "voxtype setup vad"),
            Check::error("output.method", "No output", "Install wtype"),
        ]);
        let (title, body) = startup_notification(&r).unwrap();
        assert_eq!(title, "Voxtype: 1 problem needs attention");
        assert!(body.contains("No output"));
        assert!(!body.contains("model missing") && !body.contains("VAD missing"));
    }

    #[test]
    fn marker_notifies_once_per_distinct_failure_set() {
        let vad = Check::error("vad.model", "VAD missing", "fix");
        let out = Check::error("output.method", "No output", "fix");

        // First failure notifies and records the sorted ids.
        assert_eq!(
            marker_action(None, &[&vad, &out]),
            MarkerAction::Notify("output.method,vad.model".into())
        );
        // A restart with the same failures, in any order, stays quiet.
        assert_eq!(
            marker_action(Some("output.method,vad.model\n"), &[&out, &vad]),
            MarkerAction::Keep
        );
        // A changed set notifies again.
        assert_eq!(
            marker_action(Some("output.method,vad.model"), &[&vad]),
            MarkerAction::Notify("vad.model".into())
        );
        // A clean start clears the marker.
        assert_eq!(marker_action(Some("vad.model"), &[]), MarkerAction::Clear);
    }

    #[test]
    fn text_rendering_marks_and_fixes() {
        let r = report(vec![
            Check::ok("system.cpu", "CPU ok"),
            Check::warn("output.method", "clipboard only", "Install wtype"),
            Check::error("vad.model", "VAD missing", "Run: voxtype setup vad"),
        ]);
        let text = render_text(&r, false);
        assert!(text.contains("  ✓ CPU ok"));
        assert!(text.contains("  ! clipboard only\n      Fix: Install wtype"));
        assert!(text.contains("  ✗ VAD missing\n      Fix: Run: voxtype setup vad"));
        assert!(text.contains("✗ Some checks failed."));
        assert!(!text.contains('\x1b'));

        let ok = render_text(&report(vec![Check::ok("system.cpu", "CPU ok")]), false);
        assert!(ok.contains("✓ All checks passed!"));
    }

    #[test]
    fn json_rendering_is_machine_readable() {
        let r = report(vec![
            Check::ok("system.cpu", "CPU ok"),
            Check::error("vad.model", "VAD missing", "Run: voxtype setup vad"),
        ]);
        let v: serde_json::Value = serde_json::from_str(&render_json(&r)).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["errors"], 1);
        assert_eq!(v["checks"][0]["severity"], "ok");
        assert!(v["checks"][0].get("fix").is_none());
        assert_eq!(v["checks"][1]["id"], "vad.model");
        assert_eq!(v["checks"][1]["fix"], "Run: voxtype setup vad");
    }

    #[test]
    fn omarchy_checks_report_the_four_lines() {
        // Not Omarchy and not chosen: nothing to say.
        assert!(omarchy_checks(&OmarchyObservation::default()).is_empty());

        let healthy = OmarchyObservation {
            detected: true,
            used_by: vec!["[meeting.summary] backend"],
            model: Some(Ok("qwen at http://127.0.0.1:11435/v1".into())),
            key: Some(Ok(PathBuf::from("/home/u/.config/omarchy/agent-local/key"))),
            endpoint: Some(Ok(())),
        };
        let checks = omarchy_checks(&healthy);
        let ids: Vec<&str> = checks.iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            [
                "omarchy.detected",
                "omarchy.model",
                "omarchy.key",
                "omarchy.endpoint"
            ]
        );
        assert!(checks.iter().all(|c| c.severity == Severity::Ok));
        assert_eq!(heading("omarchy.model"), "Omarchy local AI");

        // Chosen but nothing running: a warning that says how to start one.
        let stopped = OmarchyObservation {
            model: Some(Err("No Omarchy text model is running".into())),
            endpoint: None,
            ..healthy.clone()
        };
        let model = omarchy_checks(&stopped)
            .into_iter()
            .find(|c| c.id == "omarchy.model")
            .unwrap();
        assert_eq!(model.severity, Severity::Warn);
        assert!(model.fix.unwrap().contains("omarchy models start"));

        // Not chosen: a stopped model is no problem.
        let unused = OmarchyObservation {
            used_by: Vec::new(),
            ..stopped
        };
        assert!(omarchy_checks(&unused)
            .iter()
            .all(|c| c.severity == Severity::Ok));

        // Chosen without Omarchy: an error.
        let missing = OmarchyObservation {
            detected: false,
            used_by: vec!["[output.post_process] backend"],
            ..Default::default()
        };
        let checks = omarchy_checks(&missing);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].severity, Severity::Error);
    }
}
