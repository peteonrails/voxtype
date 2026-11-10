//! Model upgrade offers: ONNX to GGUF format updates and Parakeet Ultra.
//!
//! Everything a frontend shows about an upgrade comes from [`UpgradeStatus`],
//! which is a pure function of the configured engine/model, the backends this
//! binary was compiled with, the models on disk and the state file. The tray,
//! the Omarchy Settings panel, Waybar and `voxtype setup migrate` all read that
//! one object (through `voxtype status --format json --extended` or
//! `voxtype setup migrate --json`) so no surface decides on its own.
//!
//! Only same-checkpoint format updates are offered as a "format" change. A
//! different checkpoint (Ultra) is its own offer and never a silent
//! substitution.
//!
//! Declines, the accepted switch and the first successful transcription with
//! it are recorded in the state file under voxtype's state directory, never in
//! `config.toml`.

pub mod migrate;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, TranscriptionEngine};
use crate::model_selection::{is_gguf, BackendCapabilities};

/// Version of the status object shape. Bump only on a breaking change.
pub const SCHEMA: u32 = 1;

/// Offer id for switching a Parakeet user to Parakeet Ultra.
pub const OFFER_ULTRA: &str = "parakeet-ultra";
/// Offer id for redownloading the same checkpoint as GGUF.
pub const OFFER_FORMAT: &str = "format-gguf";

/// The release that removes ONNX Runtime.
pub const ONNX_REMOVAL_VERSION: &str = "2.0";

const ULTRA_MODEL: &str = "parakeet-ultra-Q8_0.gguf";
const PARAKEET_V3_GGUF: &str = "parakeet-tdt-0.6b-v3-Q8_0.gguf";
const COHERE_Q4_GGUF: &str = "cohere-transcribe-03-2026-Q4_K_M.gguf";
const COHERE_Q8_GGUF: &str = "cohere-transcribe-03-2026-Q8_0.gguf";

/// The state file's name inside [`Config::state_dir`].
const STATE_FILE: &str = "model_upgrade.json";

/// What the status JSON reports under `model_upgrade`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeStatus {
    pub schema: u32,
    pub current: CurrentModel,
    pub offers: Vec<Offer>,
    pub removal_notice: Option<RemovalNotice>,
    pub cleanup: Option<Cleanup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentModel {
    pub engine: String,
    pub model: String,
    /// `onnx`, `gguf`, `ggml` (Whisper), `openvino` or `remote`.
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub id: String,
    /// `model` for a different checkpoint, `format` for the same one in GGUF.
    pub kind: String,
    pub engine: String,
    pub model: String,
    pub title: String,
    pub detail: String,
    /// Bytes still to download; 0 when the file is already on disk.
    pub download_bytes: u64,
    pub recommended: bool,
    pub declined: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovalNotice {
    pub version: String,
    pub message: String,
    pub pin_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cleanup {
    /// The model the user switched away from, or the first one to remove
    /// when that is already gone. Kept for clients that show one name.
    pub model: String,
    /// Total size of every model in `models`.
    pub bytes: u64,
    /// Every model `setup migrate --cleanup` removes: the installed ONNX
    /// models of the upgraded engine, which the model list no longer offers.
    #[serde(default)]
    pub models: Vec<String>,
}

impl UpgradeStatus {
    pub fn offer(&self, id: &str) -> Option<&Offer> {
        self.offers.iter().find(|offer| offer.id == id)
    }

    /// Whether the status JSON should carry this object at all. Nothing to
    /// show means `null`, so a frontend hides every upgrade surface.
    pub fn has_anything(&self) -> bool {
        !self.offers.is_empty() || self.removal_notice.is_some() || self.cleanup.is_some()
    }

    /// An offer the user has not declined. Drives the tooltip line.
    pub fn has_open_offer(&self) -> bool {
        self.offers.iter().any(|offer| !offer.declined)
    }
}

// ---------------------------------------------------------------- state file

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeState {
    #[serde(default)]
    pub declined: Vec<Decline>,
    #[serde(default)]
    pub accepted: Option<Accepted>,
}

/// A "keep current model" answer for one offer while on one model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decline {
    pub id: String,
    /// The model that was configured when the offer was declined.
    pub from_model: String,
    /// The voxtype version that recorded the decline.
    pub version: String,
}

/// The most recent accepted switch, kept so the old files can be removed
/// once the new model has proven itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accepted {
    pub id: String,
    pub engine: String,
    pub model: String,
    pub previous_engine: String,
    pub previous_model: String,
    pub version: String,
    /// Set by the daemon after its first successful transcription with
    /// `model`. Cleanup is only offered after that.
    #[serde(default)]
    pub first_success: bool,
    /// Set once `voxtype setup migrate --cleanup` removed the previous files.
    #[serde(default)]
    pub cleaned: bool,
}

impl UpgradeState {
    pub fn is_declined(&self, id: &str, from_model: &str) -> bool {
        self.declined
            .iter()
            .any(|decline| decline.id == id && decline.from_model == from_model)
    }

    /// Load the state file. A missing or unreadable file is an empty state:
    /// the worst outcome is showing an offer the user declined again, which
    /// is better than hiding status output over a damaged file.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                tracing::warn!(path = %path.display(), %error, "Ignoring unreadable model upgrade state");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Write atomically so a reader never sees a half-written file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("state path has no parent directory"))?;
        std::fs::create_dir_all(parent)?;
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text + "\n")?;
        std::fs::rename(&tmp, path)
    }
}

/// `~/.local/state/voxtype/model_upgrade.json`.
pub fn state_path() -> PathBuf {
    Config::state_dir().join(STATE_FILE)
}

// ------------------------------------------------------------ offer table

/// Everything the offer table depends on. Kept explicit so tests can cover
/// every row, and every feature combination, without touching the host.
pub struct Inputs<'a> {
    pub engine: &'a str,
    pub model: &'a str,
    /// `[parakeet] streaming`; streaming configs get no offers.
    pub streaming: bool,
    pub capabilities: BackendCapabilities,
    /// A GGUF worker executable is reachable. The GGUF client can be
    /// compiled in while the package left the worker out; a GGUF offer would
    /// then fail on accept, so none is made.
    pub gguf_worker: bool,
    pub models_dir: &'a Path,
    pub state: &'a UpgradeState,
}

impl<'a> Inputs<'a> {
    pub fn for_config(config: &'a Config, models_dir: &'a Path, state: &'a UpgradeState) -> Self {
        Self {
            engine: config.engine.name(),
            model: config.model_name(),
            streaming: config.engine == TranscriptionEngine::Parakeet
                && config.parakeet.as_ref().is_some_and(|p| p.streaming),
            capabilities: BackendCapabilities::compiled(),
            gguf_worker: gguf_worker_available(),
            models_dir,
            state,
        }
    }
}

/// Whether this binary can start a GGUF worker. Without any GGUF feature
/// there is nothing to start, and no GGUF offer is made anyway.
fn gguf_worker_available() -> bool {
    #[cfg(any(feature = "cohere-gguf", feature = "parakeet-gguf", feature = "gguf"))]
    {
        crate::transcribe::gguf_subprocess::worker_available()
    }
    #[cfg(not(any(feature = "cohere-gguf", feature = "parakeet-gguf", feature = "gguf")))]
    {
        false
    }
}

/// The capabilities an offer may rely on: the compiled GGUF backends count
/// only when a worker is there to run them.
fn reachable(capabilities: BackendCapabilities, gguf_worker: bool) -> BackendCapabilities {
    BackendCapabilities {
        parakeet_gguf: capabilities.parakeet_gguf && gguf_worker,
        cohere_gguf: capabilities.cohere_gguf && gguf_worker,
        gguf: capabilities.gguf && gguf_worker,
        ..capabilities
    }
}

fn basename(model: &str) -> &str {
    Path::new(model)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(model)
}

/// Engines whose models run on ONNX Runtime unless the model is a GGUF file.
pub fn model_format(engine: &str, model: &str) -> &'static str {
    match engine {
        "gguf" => "gguf",
        "parakeet" | "cohere" if is_gguf(model) => "gguf",
        "parakeet" | "cohere" | "moonshine" | "sensevoice" | "paraformer" | "dolphin"
        | "omnilingual" => "onnx",
        "whisper" => "ggml",
        "openvino" => "openvino",
        _ => "remote",
    }
}

/// `(offer id, target model)` pairs for one configuration, Ultra first.
fn offer_targets(
    engine: &str,
    model: &str,
    streaming: bool,
    capabilities: BackendCapabilities,
) -> Vec<(&'static str, &'static str)> {
    let name = basename(model);
    match engine {
        "parakeet" => {
            // Switching a streaming user to batch-only GGUF would take away
            // the live partials they chose. GGUF streaming lands later.
            if streaming || name == "parakeet-unified-en-0.6b" || !capabilities.parakeet_gguf {
                return Vec::new();
            }
            match name {
                "parakeet-tdt-0.6b-v3" | "parakeet-tdt-0.6b-v3-int8" => {
                    vec![(OFFER_ULTRA, ULTRA_MODEL), (OFFER_FORMAT, PARAKEET_V3_GGUF)]
                }
                // No v2 GGUF is in the catalog, so v2 only gets Ultra.
                "parakeet-tdt-0.6b-v2" | "parakeet-tdt-0.6b-v2-int8" => {
                    vec![(OFFER_ULTRA, ULTRA_MODEL)]
                }
                v3 if is_gguf(v3) && v3.starts_with("parakeet-tdt-0.6b-v3") => {
                    vec![(OFFER_ULTRA, ULTRA_MODEL)]
                }
                _ => Vec::new(),
            }
        }
        "cohere" if capabilities.cohere_gguf => match name {
            "cohere-transcribe-q4f16" | "cohere-transcribe-q4" | "cohere-transcribe-int8" => {
                vec![(OFFER_FORMAT, COHERE_Q4_GGUF)]
            }
            "cohere-transcribe-fp16" => vec![(OFFER_FORMAT, COHERE_Q8_GGUF)],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn offer_copy(id: &str, current: &str, downloaded: bool) -> (&'static str, String, String) {
    if id == OFFER_ULTRA {
        let detail = if basename(current).starts_with("parakeet-tdt-0.6b-v2") {
            "Parakeet Ultra covers English and 24 more European languages, with automatic language detection.".to_string()
        } else {
            "Parakeet Ultra covers the same 25 European languages as Parakeet v3.".to_string()
        };
        (
            "model",
            "Parakeet Ultra is now available".to_string(),
            detail,
        )
    } else {
        (
            "format",
            "Your model format has been updated".to_string(),
            if downloaded {
                format!(
                    "Switch {} to the new format for better performance.",
                    basename(current)
                )
            } else {
                format!(
                    "Redownload {} in the new format for better performance.",
                    basename(current)
                )
            },
        )
    }
}

/// Bytes still to download for a catalog GGUF, 0 when it is already present.
fn download_bytes(models_dir: &Path, engine: &str, model: &str) -> u64 {
    if crate::model_catalog::model_health_in(models_dir, engine, model)
        == crate::model_catalog::ModelHealth::Present
    {
        return 0;
    }
    crate::setup::model::find_gguf_model(model)
        .map(|entry| entry.size)
        .unwrap_or(0)
}

/// Where a model lives on disk: a file for GGUF, a directory for ONNX.
pub(crate) fn model_location(models_dir: &Path, engine: &str, model: &str) -> PathBuf {
    if is_gguf(model) {
        crate::model_selection::model_path_in(models_dir, model)
    } else {
        models_dir.join(crate::model_catalog::model_dir_name(engine, model))
    }
}

/// Resolve a model's files for deletion, refusing anything that is not a
/// plain entry directly inside the models directory. A symlink is refused
/// rather than followed, so cleanup can never reach outside the directory.
pub(crate) fn contained_model_path(models_dir: &Path, path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .filter(|name| !matches!(name.to_str(), Some("." | "..") | None))
        .ok_or_else(|| format!("{} does not name a model", path.display()))?;
    let meta =
        std::fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "{} is a symbolic link; voxtype only removes model files it downloaded",
            path.display()
        ));
    }
    let root = models_dir
        .canonicalize()
        .map_err(|error| format!("{}: {error}", models_dir.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if parent != root {
        return Err(format!(
            "{} is outside the models directory {}",
            path.display(),
            root.display()
        ));
    }
    Ok(root.join(name))
}

/// Total size of a file or directory tree, not following symlinks.
pub(crate) fn disk_usage(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| disk_usage(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

/// Engines whose model list holds both ONNX and GGUF checkpoints. Once a
/// user is on GGUF there, the ONNX rows are legacy: hidden from the model
/// list and removed together by cleanup.
pub fn has_gguf_successors(engine: &str) -> bool {
    matches!(engine, "parakeet" | "cohere")
}

/// Whether a model-list row is a legacy ONNX checkpoint that the list only
/// shows while it is the configured model.
pub fn is_legacy_model(engine: &str, model: &str) -> bool {
    has_gguf_successors(engine) && model_format(engine, model) == "onnx"
}

/// What `setup migrate --cleanup` may delete once the accepted model has
/// transcribed successfully: every installed ONNX model of the upgraded
/// engine, plus the previous model when it was ONNX under another engine.
fn cleanup_candidate(inputs: &Inputs) -> Option<(Cleanup, Vec<PathBuf>)> {
    let accepted = inputs.state.accepted.as_ref()?;
    if !accepted.first_success || accepted.cleaned || accepted.model != inputs.model {
        return None;
    }
    let mut names: Vec<(String, String)> = Vec::new();
    if model_format(&accepted.previous_engine, &accepted.previous_model) == "onnx" {
        names.push((
            accepted.previous_engine.clone(),
            accepted.previous_model.clone(),
        ));
    }
    if has_gguf_successors(&accepted.engine) {
        for name in crate::model_catalog::model_catalog(&accepted.engine) {
            if model_format(&accepted.engine, name) == "onnx" {
                names.push((accepted.engine.clone(), name.to_string()));
            }
        }
    }
    let mut models = Vec::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut bytes = 0;
    for (engine, name) in names {
        if name == inputs.model {
            continue;
        }
        let location = model_location(inputs.models_dir, &engine, &name);
        let Ok(path) = contained_model_path(inputs.models_dir, &location) else {
            continue;
        };
        if paths.contains(&path) {
            continue;
        }
        bytes += disk_usage(&path);
        models.push(name);
        paths.push(path);
    }
    let first = models.first()?.clone();
    let model = if models.contains(&accepted.previous_model) {
        accepted.previous_model.clone()
    } else {
        first
    };
    Some((
        Cleanup {
            model,
            bytes,
            models,
        },
        paths,
    ))
}

pub(crate) fn removal_message(model: &str, pin_command: &str) -> String {
    format!(
        "Support for {} (ONNX) will be removed in voxtype {ONNX_REMOVAL_VERSION}. To keep using it, pin voxtype to this version: {pin_command}",
        basename(model)
    )
}

/// Compute the status object. `pin_command` is only called when the current
/// model is ONNX, since detecting the install method runs a package manager.
pub fn compute(inputs: &Inputs, pin_command: impl FnOnce() -> String) -> UpgradeStatus {
    let format = model_format(inputs.engine, inputs.model);
    let offers = offer_targets(
        inputs.engine,
        inputs.model,
        inputs.streaming,
        reachable(inputs.capabilities, inputs.gguf_worker),
    )
    .into_iter()
    .filter(|(_, target)| crate::setup::model::find_gguf_model(target).is_some())
    .map(|(id, target)| {
        let bytes = download_bytes(inputs.models_dir, inputs.engine, target);
        let (kind, title, detail) = offer_copy(id, inputs.model, bytes == 0);
        Offer {
            id: id.to_string(),
            kind: kind.to_string(),
            engine: inputs.engine.to_string(),
            model: target.to_string(),
            title,
            detail,
            download_bytes: bytes,
            recommended: id == OFFER_ULTRA,
            declined: inputs.state.is_declined(id, inputs.model),
        }
    })
    .collect();

    let removal_notice = (format == "onnx").then(|| {
        let pin_command = pin_command();
        RemovalNotice {
            version: ONNX_REMOVAL_VERSION.to_string(),
            message: removal_message(inputs.model, &pin_command),
            pin_command,
        }
    });

    UpgradeStatus {
        schema: SCHEMA,
        current: CurrentModel {
            engine: inputs.engine.to_string(),
            model: inputs.model.to_string(),
            format: format.to_string(),
        },
        offers,
        removal_notice,
        cleanup: cleanup_candidate(inputs).map(|(cleanup, _)| cleanup),
    }
}

/// The status for this machine: the loaded config, this binary's backends,
/// the real models directory and state file, and the detected install method.
pub fn status_for_config(config: &Config) -> UpgradeStatus {
    let models_dir = Config::models_dir();
    let state = UpgradeState::load(&state_path());
    compute(&Inputs::for_config(config, &models_dir, &state), || {
        system_pin_command()
    })
}

// ------------------------------------------------------- install method

/// Runs a command and returns its stdout when it exits successfully.
/// Injected so tests never run a real package manager.
pub trait CommandRunner {
    fn output(&self, program: &str, args: &[&str]) -> Option<String>;
}

pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn output(&self, program: &str, args: &[&str]) -> Option<String> {
        let output = std::process::Command::new(program)
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
}

/// How voxtype was installed, as far as pinning it is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallMethod {
    Pacman(String),
    Dpkg(String),
    Rpm(String),
    Nix,
    Other,
}

impl InstallMethod {
    pub fn pin_command(&self) -> String {
        match self {
            InstallMethod::Pacman(package) => {
                format!("add IgnorePkg = {package} to /etc/pacman.conf")
            }
            InstallMethod::Dpkg(package) => format!("sudo apt-mark hold {package}"),
            InstallMethod::Rpm(package) => format!("sudo dnf versionlock add {package}"),
            InstallMethod::Nix => "pin the voxtype flake input".to_string(),
            InstallMethod::Other => format!(
                "hold voxtype at version {} in your package manager",
                env!("CARGO_PKG_VERSION")
            ),
        }
    }
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// Ask each package manager which package owns the running executable,
/// then the conventional `/usr/bin/voxtype`.
pub fn detect_install_method(exe: &Path, runner: &dyn CommandRunner) -> InstallMethod {
    if exe.starts_with("/nix/store") {
        return InstallMethod::Nix;
    }
    let mut candidates = vec![exe.to_path_buf()];
    let usr_bin = PathBuf::from("/usr/bin/voxtype");
    if exe != usr_bin {
        candidates.push(usr_bin);
    }
    for path in &candidates {
        let Some(path) = path.to_str() else { continue };
        if let Some(package) = runner
            .output("pacman", &["-Qqo", path])
            .as_deref()
            .and_then(first_line)
        {
            return InstallMethod::Pacman(package);
        }
        // `voxtype: /usr/bin/voxtype`, or `voxtype:amd64: ...` on multiarch.
        if let Some(package) = runner
            .output("dpkg", &["-S", path])
            .as_deref()
            .and_then(first_line)
            .and_then(|line| line.split(": ").next().map(str::to_string))
            .map(|owner| owner.split(':').next().unwrap_or(&owner).to_string())
            .filter(|package| !package.is_empty() && !package.contains(' '))
        {
            return InstallMethod::Dpkg(package);
        }
        if let Some(package) = runner
            .output("rpm", &["-qf", "--queryformat", "%{NAME}", path])
            .as_deref()
            .and_then(first_line)
            .filter(|package| !package.contains(' '))
        {
            return InstallMethod::Rpm(package);
        }
    }
    InstallMethod::Other
}

/// The pin command for this machine, detected once per process: `status
/// --follow` asks on every state change, and the answer cannot change while
/// the process is running.
pub fn system_pin_command() -> String {
    static PIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PIN.get_or_init(|| {
        let exe = std::env::current_exe()
            .and_then(|exe| exe.canonicalize())
            .unwrap_or_else(|_| PathBuf::from("/usr/bin/voxtype"));
        detect_install_method(&exe, &SystemRunner).pin_command()
    })
    .clone()
}

// ------------------------------------------------- daemon success record

/// The accepted model still waiting for its first successful transcription,
/// if `current_model` is it. Read once at daemon startup.
pub fn pending_first_success(state_file: &Path, current_model: &str) -> bool {
    UpgradeState::load(state_file)
        .accepted
        .is_some_and(|accepted| accepted.model == current_model && !accepted.first_success)
}

/// Record that the accepted model transcribed successfully. Returns whether
/// anything changed.
pub fn record_first_success(state_file: &Path, current_model: &str) -> std::io::Result<bool> {
    let mut state = UpgradeState::load(state_file);
    match state.accepted.as_mut() {
        Some(accepted) if accepted.model == current_model && !accepted.first_success => {
            accepted.first_success = true;
            state.save(state_file)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests;
