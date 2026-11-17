//! Omarchy Settings panel linking: `voxtype setup omarchy` and the daemon's
//! first-run link.
//!
//! Packages ship the panel read-only at
//! `/usr/share/voxtype/omarchy-plugin/io.voxtype.settings`, with a manifest
//! whose entry point sits in a per-release directory (`<version>/Panel.qml`).
//! Each user gets a symlink at `~/.config/omarchy/plugins/io.voxtype.settings`
//! pointing there, the same pattern Superwhisper uses. A rescan after an
//! upgrade loads the new path, so no shell restart is needed.
//!
//! Rules that protect the user's own setup:
//! - A real directory (a copy from `omarchy plugin add`) or a symlink to
//!   somewhere else is never overwritten. `--replace` moves it to a hidden
//!   backup beside it, and only with consent. Nothing is ever deleted except
//!   our own symlink, on `--remove`.
//! - Once voxtype has linked the panel (or seen it linked), the daemon never
//!   adds it back after it disappears: the user removed it.
//!
//! Every shell call goes through [`CommandRunner`], so tests never run a real
//! `omarchy-shell`.

use crate::config::Config;
use crate::model_upgrade::CommandRunner;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The plugin id in the panel's `manifest.json`, and the name of the link.
pub const PLUGIN_ID: &str = "io.voxtype.settings";

/// Overrides the packaged panel directory (tests, custom prefixes).
pub const PLUGIN_DIR_ENV: &str = "VOXTYPE_OMARCHY_PLUGIN_DIR";

/// Where packages install the panel, in lookup order after the env override.
pub const SYSTEM_PLUGIN_DIRS: [&str; 2] = [
    "/usr/share/voxtype/omarchy-plugin/io.voxtype.settings",
    "/usr/local/share/voxtype/omarchy-plugin/io.voxtype.settings",
];

/// Omarchy's own scripts directory. The daemon runs under systemd, whose
/// PATH usually lacks it.
const OMARCHY_BIN_DIR: &str = "/usr/share/omarchy/bin";

/// Name of the marker under `Config::state_dir()`.
pub const MARKER_FILE: &str = "omarchy-plugin.json";

/// How builds without the packaged panel install it.
pub const GIT_INSTALL_COMMAND: &str =
    "omarchy plugin add https://github.com/peteonrails/omarchy-voxtype-settings.git --enable --yes";

/// How to open the panel once it is loaded (the manifest's own hint).
pub const OPEN_COMMAND: &str = "omarchy-shell shell toggle io.voxtype.settings '{}'";

// ------------------------------------------------------------------ state

/// The per-user link state (contract 2.2 of the 1.2.1 plan).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkState {
    /// No `~/.config/omarchy`, or no `omarchy-shell`.
    NotOmarchy,
    /// This install has no packaged panel.
    NoPackage,
    /// No panel with our id under `~/.config/omarchy/plugins`.
    Absent,
    /// Our link, and the shell has loaded the packaged version.
    Linked,
    /// Our link, but the shell has not rescanned since the version (or the
    /// packaged location) changed.
    LinkedStale,
    /// A real directory with our id, for example from `omarchy plugin add`.
    Copy,
    /// A symlink with our id that points somewhere else, such as a worktree.
    ForeignLink,
}

impl LinkState {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkState::NotOmarchy => "not-omarchy",
            LinkState::NoPackage => "no-package",
            LinkState::Absent => "absent",
            LinkState::Linked => "linked",
            LinkState::LinkedStale => "linked-stale",
            LinkState::Copy => "copy",
            LinkState::ForeignLink => "foreign-link",
        }
    }
}

/// `voxtype setup omarchy --status --json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub omarchy: bool,
    pub packaged_dir: Option<PathBuf>,
    pub packaged_version: Option<String>,
    pub state: LinkState,
    /// The panel entry under `~/.config/omarchy/plugins`, when there is one.
    pub path: Option<PathBuf>,
    /// The version the shell is running from that entry: for our link, the
    /// version it last rescanned; for a copy or foreign link, its manifest.
    pub installed_version: Option<String>,
    /// The user dismissed the Switch banner for their copy
    /// (`--dismiss-copy`). Only ever true while a copy is present.
    pub copy_dismissed: bool,
    /// Where a symlink entry points (not part of the JSON contract).
    #[serde(skip)]
    pub link_target: Option<PathBuf>,
}

/// What voxtype remembers between runs, at `state_dir/omarchy-plugin.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    /// Voxtype linked the panel, or saw its link, at least once. A later
    /// absence means the user removed it, and it is never re-added.
    #[serde(default)]
    pub auto_linked: bool,
    /// The packaged version the shell last rescanned for.
    #[serde(default)]
    pub version: Option<String>,
    /// The daemon already logged that the user has their own copy.
    #[serde(default)]
    pub copy_noted: bool,
    /// The first link could not enable the plugin; retry at the next start.
    #[serde(default)]
    pub enable_pending: bool,
    /// The user chose "Not now" on the panel's Switch banner for their copy.
    /// Cleared by `--replace` and once there is no longer a copy.
    #[serde(default)]
    pub copy_dismissed: bool,
}

impl Marker {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(&tmp, text + "\n")?;
        fs::rename(&tmp, path)
    }
}

// ---------------------------------------------------------------- context

/// Everything that depends on the machine, resolved once, so tests can
/// build one from fixtures instead of touching the environment.
#[derive(Debug, Clone)]
pub struct Context {
    /// `$XDG_CONFIG_HOME`, or `~/.config`.
    pub config_home: Option<PathBuf>,
    pub omarchy_shell: Option<PathBuf>,
    pub restart_shell: Option<PathBuf>,
    /// The packaged panel directory in use, if any is installed.
    pub packaged_dir: Option<PathBuf>,
    /// Every directory a link may point at and still count as ours.
    pub own_dirs: Vec<PathBuf>,
    pub marker_path: PathBuf,
}

impl Context {
    pub fn from_env() -> Self {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|h| !h.is_empty())
                    .map(|h| PathBuf::from(h).join(".config"))
            });
        let own_dirs = candidate_dirs(std::env::var_os(PLUGIN_DIR_ENV));
        Self {
            config_home,
            omarchy_shell: find_program("omarchy-shell"),
            restart_shell: find_program("omarchy-restart-shell"),
            packaged_dir: select_packaged(&own_dirs),
            own_dirs,
            marker_path: Config::state_dir().join(MARKER_FILE),
        }
    }

    pub fn omarchy_dir(&self) -> Option<PathBuf> {
        self.config_home.as_ref().map(|c| c.join("omarchy"))
    }

    pub fn plugins_dir(&self) -> Option<PathBuf> {
        self.omarchy_dir().map(|d| d.join("plugins"))
    }

    /// The link path voxtype manages.
    pub fn link_path(&self) -> Option<PathBuf> {
        self.plugins_dir().map(|d| d.join(PLUGIN_ID))
    }

    /// Omarchy is present when its config directory exists and its shell
    /// command can be found.
    pub fn is_omarchy(&self) -> bool {
        self.omarchy_shell.is_some() && self.omarchy_dir().is_some_and(|d| d.is_dir())
    }

    fn is_own_dir(&self, target: &Path) -> bool {
        self.own_dirs.iter().any(|d| same_path(d, target))
    }
}

/// The env override (when set), then the system locations.
fn candidate_dirs(env_dir: Option<std::ffi::OsString>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = env_dir
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .collect();
    dirs.extend(SYSTEM_PLUGIN_DIRS.iter().map(PathBuf::from));
    dirs
}

/// The first candidate whose manifest has our id.
fn select_packaged(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|d| is_packaged_panel(d)).cloned()
}

/// `name` on PATH, then in Omarchy's own bin directory.
fn find_program(name: &str) -> Option<PathBuf> {
    which::which(name).ok().or_else(|| {
        let path = Path::new(OMARCHY_BIN_DIR).join(name);
        path.is_file().then_some(path)
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ManifestInfo {
    id: Option<String>,
    version: Option<String>,
}

fn read_manifest(dir: &Path) -> Option<ManifestInfo> {
    let text = fs::read_to_string(dir.join("manifest.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let field = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
    Some(ManifestInfo {
        id: field("id"),
        version: field("version"),
    })
}

/// A directory counts as the packaged panel only when its manifest has our id.
fn is_packaged_panel(dir: &Path) -> bool {
    read_manifest(dir).is_some_and(|m| m.id.as_deref() == Some(PLUGIN_ID))
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

// ------------------------------------------------------------ detection

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    /// A symlink at the managed path into a packaged panel directory.
    OwnLink,
    ForeignLink,
    Copy,
}

#[derive(Debug, Clone)]
struct Entry {
    path: PathBuf,
    kind: EntryKind,
    /// Where a symlink points, resolved against the plugins directory.
    target: Option<PathBuf>,
    version: Option<String>,
}

/// Find the panel entry: the managed path first, then any other visible
/// top-level directory whose manifest has our id, the way Omarchy's scanner
/// would find it. Hidden names (our backups and temp links) are skipped,
/// as Omarchy skips them.
fn find_entry(ctx: &Context) -> Option<Entry> {
    let plugins = ctx.plugins_dir()?;
    let managed = plugins.join(PLUGIN_ID);
    if let Ok(meta) = fs::symlink_metadata(&managed) {
        let version = read_manifest(&managed).and_then(|m| m.version);
        if meta.file_type().is_symlink() {
            let target =
                fs::read_link(&managed)
                    .ok()
                    .map(|t| if t.is_absolute() { t } else { plugins.join(t) });
            let kind = match &target {
                Some(t) if ctx.is_own_dir(t) => EntryKind::OwnLink,
                _ => EntryKind::ForeignLink,
            };
            return Some(Entry {
                path: managed,
                kind,
                target,
                version,
            });
        }
        return Some(Entry {
            path: managed,
            kind: EntryKind::Copy,
            target: None,
            version,
        });
    }

    let mut names: Vec<_> = fs::read_dir(&plugins)
        .ok()?
        .flatten()
        .map(|e| e.file_name())
        .filter(|n| !n.to_string_lossy().starts_with('.'))
        .collect();
    names.sort();
    names.into_iter().find_map(|name| {
        let path = plugins.join(&name);
        let manifest = read_manifest(&path)?;
        if manifest.id.as_deref() != Some(PLUGIN_ID) {
            return None;
        }
        let is_link = fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        Some(Entry {
            target: is_link
                .then(|| fs::read_link(&path).ok())
                .flatten()
                .map(|t| if t.is_absolute() { t } else { plugins.join(t) }),
            kind: if is_link {
                EntryKind::ForeignLink
            } else {
                EntryKind::Copy
            },
            version: manifest.version,
            path,
        })
    })
}

/// Compute contract 2.2 for this machine.
pub fn status(ctx: &Context, marker: &Marker) -> Status {
    let omarchy = ctx.is_omarchy();
    let packaged_version = ctx
        .packaged_dir
        .as_deref()
        .and_then(read_manifest)
        .and_then(|m| m.version);
    let entry = find_entry(ctx);

    let state = if !omarchy {
        LinkState::NotOmarchy
    } else if ctx.packaged_dir.is_none() {
        LinkState::NoPackage
    } else {
        match entry.as_ref().map(|e| e.kind) {
            None => LinkState::Absent,
            Some(EntryKind::Copy) => LinkState::Copy,
            Some(EntryKind::ForeignLink) => LinkState::ForeignLink,
            Some(EntryKind::OwnLink) => {
                let entry = entry.as_ref().expect("matched Some");
                let points_at_package = match (&entry.target, &ctx.packaged_dir) {
                    (Some(t), Some(p)) => same_path(t, p),
                    _ => false,
                };
                if points_at_package
                    && marker.version.is_some()
                    && marker.version == packaged_version
                {
                    LinkState::Linked
                } else {
                    LinkState::LinkedStale
                }
            }
        }
    };

    let installed_version = entry.as_ref().and_then(|e| match e.kind {
        EntryKind::OwnLink => marker.version.clone().or_else(|| e.version.clone()),
        _ => e.version.clone(),
    });

    let has_copy = entry.as_ref().is_some_and(|e| e.kind == EntryKind::Copy);
    Status {
        omarchy,
        packaged_dir: ctx.packaged_dir.clone(),
        packaged_version,
        state,
        path: entry.as_ref().map(|e| e.path.clone()),
        installed_version,
        copy_dismissed: marker.copy_dismissed && has_copy,
        link_target: entry.and_then(|e| e.target),
    }
}

// ------------------------------------------------------------------ shell

/// Calls into Omarchy's shell.
pub struct Shell<'a> {
    runner: &'a dyn CommandRunner,
    program: String,
    restart_program: Option<String>,
    /// How many times to try `rescanPlugins` before giving up. The daemon
    /// waits for a shell that is still starting; the CLI asks once.
    rescan_attempts: u32,
    /// `enablePlugin` answers `unknown` until the registry's scan finishes.
    enable_attempts: u32,
    interval: Duration,
}

impl<'a> Shell<'a> {
    /// For the CLI: one rescan, and up to 5 s for the registry to finish.
    pub fn new(ctx: &Context, runner: &'a dyn CommandRunner) -> Option<Self> {
        Some(Self {
            runner,
            program: ctx.omarchy_shell.as_ref()?.to_string_lossy().into_owned(),
            restart_program: ctx
                .restart_shell
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            rescan_attempts: 1,
            enable_attempts: 20,
            interval: Duration::from_millis(250),
        })
    }

    /// For the daemon, which may start before the shell does: wait up to
    /// two minutes for the shell to answer, in a background thread.
    pub fn patient(ctx: &Context, runner: &'a dyn CommandRunner) -> Option<Self> {
        let mut shell = Self::new(ctx, runner)?;
        shell.rescan_attempts = 120;
        shell.enable_attempts = 20;
        shell.interval = Duration::from_secs(1);
        Some(shell)
    }

    #[cfg(test)]
    fn with_attempts(mut self, rescan: u32, enable: u32) -> Self {
        self.rescan_attempts = rescan;
        self.enable_attempts = enable;
        self.interval = Duration::ZERO;
        self
    }

    fn retry(&self, attempts: u32, mut call: impl FnMut() -> bool) -> bool {
        for attempt in 0..attempts.max(1) {
            if attempt > 0 && !self.interval.is_zero() {
                std::thread::sleep(self.interval);
            }
            if call() {
                return true;
            }
        }
        false
    }

    /// `omarchy-shell -q shell rescanPlugins`
    pub fn rescan(&self) -> bool {
        self.retry(self.rescan_attempts, || {
            self.runner
                .output(&self.program, &["-q", "shell", "rescanPlugins"])
                .is_some()
        })
    }

    /// `omarchy-shell shell enablePlugin io.voxtype.settings '{}'` until it
    /// answers `ok`.
    pub fn enable(&self) -> bool {
        self.retry(self.enable_attempts, || {
            self.runner
                .output(&self.program, &["shell", "enablePlugin", PLUGIN_ID, "{}"])
                .is_some_and(|out| out.trim() == "ok")
        })
    }

    /// `omarchy-restart-shell`
    pub fn restart(&self) -> bool {
        self.restart_program
            .as_deref()
            .is_some_and(|program| self.runner.output(program, &[]).is_some())
    }
}

/// A [`CommandRunner`] that gives up on a command after `timeout`, so a
/// wedged shell cannot hang `voxtype setup omarchy`. Stdout is read without
/// blocking once the command exits, because `omarchy-restart-shell` leaves
/// a new shell running that may still hold the pipe open.
pub struct TimeoutRunner {
    pub timeout: Duration,
}

impl Default for TimeoutRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
        }
    }
}

impl CommandRunner for TimeoutRunner {
    fn output(&self, program: &str, args: &[&str]) -> Option<String> {
        use std::io::Read;
        use std::os::fd::AsRawFd;
        use std::process::{Command, Stdio};

        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let deadline = std::time::Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        };
        let mut out = Vec::new();
        if let Some(mut stdout) = child.stdout.take() {
            // SAFETY: fcntl on a pipe fd we own; only flags are changed.
            unsafe {
                let fd = stdout.as_raw_fd();
                let flags = libc::fcntl(fd, libc::F_GETFL);
                if flags >= 0 {
                    libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                }
            }
            let mut buf = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n]);
            }
        }
        status
            .success()
            .then(|| String::from_utf8_lossy(&out).into_owned())
    }
}

// -------------------------------------------------------------- actions

/// What a link, replace or refresh did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkReport {
    pub path: PathBuf,
    pub target: PathBuf,
    pub version: Option<String>,
    /// The shell rescanned, so it loads the packaged version now.
    pub rescanned: bool,
    /// `None` when enabling was not attempted.
    pub enabled: Option<bool>,
    /// Where `--replace` moved the user's entry.
    pub backup: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum OmarchyError {
    #[error("Omarchy was not detected (needs ~/.config/omarchy and omarchy-shell)")]
    NotOmarchy,
    #[error(
        "This voxtype install does not include the packaged Settings panel.\n  \
         Install it from git instead: {GIT_INSTALL_COMMAND}"
    )]
    NoPackage,
    #[error(
        "{0} is not voxtype's link, so it was left alone.\n  \
         Replace it (it is backed up, not deleted): voxtype setup omarchy --replace"
    )]
    NotOurs(PathBuf),
    #[error(
        "{0} is your own copy of the panel, not voxtype's link, so it was left alone.\n  \
         Remove it with: omarchy plugin remove io.voxtype.settings"
    )]
    RemoveRefused(PathBuf),
    #[error("Replacing your copy needs confirmation. Re-run with --yes, or in a terminal.")]
    NeedsConsent,
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
}

fn io_err(context: impl Into<String>) -> impl FnOnce(io::Error) -> OmarchyError {
    let context = context.into();
    move |source| OmarchyError::Io { context, source }
}

/// Point the managed path at the packaged panel: write a hidden temp link
/// and rename it into place, so the scanner never sees a half-made entry.
/// Renaming a link over a real directory fails, which keeps a copy safe
/// even if a caller skipped the state check.
fn place_link(plugins: &Path, target: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(plugins)?;
    let link = plugins.join(PLUGIN_ID);
    let tmp = plugins.join(format!(".{PLUGIN_ID}.tmp-{}", std::process::id()));
    if fs::symlink_metadata(&tmp).is_ok() {
        fs::remove_file(&tmp)?;
    }
    std::os::unix::fs::symlink(target, &tmp)?;
    if let Err(err) = fs::rename(&tmp, &link) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(link)
}

/// Link (or re-point) the managed path, rescan, and optionally enable.
/// Refuses unless the managed path is absent or already our link.
pub fn link(
    ctx: &Context,
    shell: Option<&Shell>,
    marker: &mut Marker,
    enable: bool,
) -> Result<LinkReport, OmarchyError> {
    let packaged = ctx.packaged_dir.clone().ok_or(OmarchyError::NoPackage)?;
    let plugins = ctx.plugins_dir().ok_or(OmarchyError::NotOmarchy)?;
    match find_entry(ctx) {
        None => {}
        Some(e) if e.kind == EntryKind::OwnLink => {}
        Some(e) => return Err(OmarchyError::NotOurs(e.path)),
    }
    let version = read_manifest(&packaged).and_then(|m| m.version);

    let current = fs::read_link(plugins.join(PLUGIN_ID)).ok();
    let path = if current.as_deref().is_some_and(|t| same_path(t, &packaged)) {
        plugins.join(PLUGIN_ID)
    } else {
        place_link(&plugins, &packaged).map_err(io_err(format!(
            "Could not link {}",
            plugins.join(PLUGIN_ID).display()
        )))?
    };

    // Recorded before any shell call, which can take minutes in the daemon:
    // a daemon that exits meanwhile must not leave a link the next start
    // would treat as never made (and so re-add after the user removes it).
    marker.auto_linked = true;
    if enable {
        marker.enable_pending = true;
    }
    save_marker(ctx, marker);

    let rescanned = shell.is_some_and(|s| s.rescan());
    if rescanned {
        marker.version = version.clone();
    }
    let enabled = if enable {
        let ok = rescanned && shell.is_some_and(|s| s.enable());
        marker.enable_pending = !ok;
        Some(ok)
    } else {
        None
    };
    save_marker(ctx, marker);

    Ok(LinkReport {
        path,
        target: packaged,
        version,
        rescanned,
        enabled,
        backup: None,
    })
}

fn backup_path(plugins: &Path) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let base = format!(".{PLUGIN_ID}.backup-{now}");
    let mut candidate = plugins.join(&base);
    let mut n = 1;
    while fs::symlink_metadata(&candidate).is_ok() {
        candidate = plugins.join(format!("{base}-{n}"));
        n += 1;
    }
    candidate
}

/// Move the user's copy or foreign link to a hidden backup (never delete
/// it), then link. Callers must have the user's consent.
pub fn replace(
    ctx: &Context,
    shell: Option<&Shell>,
    marker: &mut Marker,
) -> Result<LinkReport, OmarchyError> {
    if ctx.packaged_dir.is_none() {
        return Err(OmarchyError::NoPackage);
    }
    let plugins = ctx.plugins_dir().ok_or(OmarchyError::NotOmarchy)?;
    let mut backup = None;
    if let Some(entry) = find_entry(ctx).filter(|e| e.kind != EntryKind::OwnLink) {
        let dest = backup_path(&plugins);
        fs::rename(&entry.path, &dest).map_err(io_err(format!(
            "Could not move {} to {}",
            entry.path.display(),
            dest.display()
        )))?;
        match &entry.target {
            // The CLI prints its own line for this; only the daemon logs at info.
            Some(target) => tracing::debug!(
                "Moved the Omarchy panel link {} (pointing at {}) to {}",
                entry.path.display(),
                target.display(),
                dest.display()
            ),
            None => tracing::debug!(
                "Moved the Omarchy panel copy {} to {}",
                entry.path.display(),
                dest.display()
            ),
        }
        backup = Some(dest);
    }
    marker.copy_noted = false;
    marker.copy_dismissed = false;
    let mut report = link(ctx, shell, marker, true)?;
    report.backup = backup;
    Ok(report)
}

/// Remove our link (never a copy). Returns the removed path, or `None`
/// when there was nothing to remove.
pub fn remove(
    ctx: &Context,
    shell: Option<&Shell>,
    marker: &mut Marker,
) -> Result<Option<PathBuf>, OmarchyError> {
    let Some(entry) = find_entry(ctx) else {
        return Ok(None);
    };
    if entry.kind != EntryKind::OwnLink {
        return Err(OmarchyError::RemoveRefused(entry.path));
    }
    fs::remove_file(&entry.path)
        .map_err(io_err(format!("Could not remove {}", entry.path.display())))?;
    // Remembered so the daemon never adds it back.
    marker.auto_linked = true;
    marker.enable_pending = false;
    save_marker(ctx, marker);
    if let Some(shell) = shell {
        shell.rescan();
    }
    Ok(Some(entry.path))
}

/// A dismissal belongs to a copy: once there is no copy (removed, replaced,
/// or swapped for a link), forget it so a later copy shows the banner again.
/// Returns true when the marker changed.
fn forget_stale_dismissal(ctx: &Context, marker: &mut Marker) -> bool {
    let has_copy = find_entry(ctx).is_some_and(|e| e.kind == EntryKind::Copy);
    if marker.copy_dismissed && !has_copy {
        marker.copy_dismissed = false;
        return true;
    }
    false
}

/// Remember "Not now" for the panel's Switch banner. Only a copy can be
/// dismissed; returns its path, or `None` when there is no copy.
pub fn dismiss_copy(ctx: &Context, marker: &mut Marker) -> Option<PathBuf> {
    let entry = find_entry(ctx).filter(|e| e.kind == EntryKind::Copy)?;
    marker.copy_dismissed = true;
    save_marker(ctx, marker);
    Some(entry.path)
}

fn save_marker(ctx: &Context, marker: &Marker) {
    if let Err(err) = marker.save(&ctx.marker_path) {
        tracing::warn!("Could not write {}: {}", ctx.marker_path.display(), err);
    }
}

// --------------------------------------------------------------- daemon

/// What the daemon decides to do at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoAction {
    Nothing,
    /// First run on Omarchy: link, rescan, enable.
    Link,
    /// Our link, but the shell has not loaded this version (or the link
    /// points at an old location): re-point if needed and rescan.
    Refresh,
    /// The first link could not enable the plugin; try again.
    RetryEnable,
    /// The user has their own copy: leave it, log once.
    NoteCopy,
}

/// The daemon's rules, as a pure function of the state.
pub fn decide(status: &Status, marker: &Marker, opted_in: bool) -> AutoAction {
    if !opted_in {
        return AutoAction::Nothing;
    }
    match status.state {
        LinkState::NotOmarchy | LinkState::NoPackage => AutoAction::Nothing,
        // Linked before, or the user had their own copy: either way the
        // user removed it, so it stays removed.
        LinkState::Absent if marker.auto_linked || marker.copy_noted => AutoAction::Nothing,
        LinkState::Absent => AutoAction::Link,
        LinkState::LinkedStale => AutoAction::Refresh,
        LinkState::Linked if marker.enable_pending => AutoAction::RetryEnable,
        LinkState::Linked => AutoAction::Nothing,
        LinkState::Copy | LinkState::ForeignLink if marker.copy_noted => AutoAction::Nothing,
        LinkState::Copy | LinkState::ForeignLink => AutoAction::NoteCopy,
    }
}

/// Desktop notification sent once when the daemon first finds an old copy:
/// copies made before 1.2.1 have no Switch banner of their own.
pub const COPY_NOTICE_TITLE: &str = "Voxtype Settings panel";
pub const COPY_NOTICE_BODY: &str =
    "Your Settings panel is a copy that does not update with voxtype. \
     Run voxtype setup omarchy --replace to switch to the packaged one.";

/// Apply the daemon's rules once. Never fails: problems are logged.
/// `notify(title, body)` sends a desktop notification.
pub fn auto_link(
    ctx: &Context,
    shell: Option<&Shell>,
    opted_in: bool,
    notify: &dyn Fn(&str, &str),
) -> AutoAction {
    let mut marker = Marker::load(&ctx.marker_path);
    if opted_in && forget_stale_dismissal(ctx, &mut marker) {
        save_marker(ctx, &marker);
    }
    let status = status(ctx, &marker);
    let action = decide(&status, &marker, opted_in);
    tracing::debug!(
        state = status.state.as_str(),
        ?action,
        "Omarchy Settings panel check"
    );
    match action {
        AutoAction::Nothing => {}
        AutoAction::Link => match link(ctx, shell, &mut marker, true) {
            Ok(report) => {
                tracing::info!(
                    "Linked the Voxtype Settings panel for Omarchy: {} -> {}",
                    report.path.display(),
                    report.target.display()
                );
                if report.enabled != Some(true) {
                    tracing::info!(
                        "Omarchy's shell did not answer; the panel will be enabled at the next \
                         daemon start. To do it now: voxtype setup omarchy"
                    );
                }
            }
            Err(err) => tracing::warn!("Could not link the Omarchy Settings panel: {err}"),
        },
        AutoAction::Refresh => {
            let enable = marker.enable_pending;
            match link(ctx, shell, &mut marker, enable) {
                Ok(report) if report.rescanned => tracing::info!(
                    "Omarchy reloaded the Voxtype Settings panel at version {}",
                    report.version.as_deref().unwrap_or("unknown")
                ),
                Ok(_) => tracing::info!(
                    "The Voxtype Settings panel changed version, but Omarchy's shell did not \
                     answer. It loads the new version when the shell starts, or now with: \
                     omarchy-shell shell rescanPlugins"
                ),
                Err(err) => tracing::warn!("Could not refresh the Omarchy Settings panel: {err}"),
            }
        }
        AutoAction::RetryEnable => {
            let ok = shell.is_some_and(|s| s.rescan() && s.enable());
            if ok {
                marker.enable_pending = false;
                save_marker(ctx, &marker);
                tracing::info!("Enabled the Voxtype Settings panel in Omarchy");
            } else {
                tracing::info!("Omarchy's shell did not answer; will enable the panel later");
            }
        }
        AutoAction::NoteCopy => {
            tracing::info!(
                "Found your own copy of the Voxtype Settings panel at {}; leaving it alone. \
                 To switch to the packaged panel, which updates with voxtype: \
                 voxtype setup omarchy --replace",
                status
                    .path
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
            marker.copy_noted = true;
            // Recorded before notifying, so the notice is sent at most once
            // per copy even if the daemon exits meanwhile. Never retried.
            save_marker(ctx, &marker);
            if status.state == LinkState::Copy && !marker.copy_dismissed {
                notify(COPY_NOTICE_TITLE, COPY_NOTICE_BODY);
            }
        }
    }
    action
}

/// Run [`auto_link`] on a background thread at daemon start. Never blocks
/// startup and never fails the daemon.
pub fn spawn_auto_link(config: &Config) {
    if !config.omarchy.link_settings_plugin {
        tracing::debug!("Omarchy Settings panel linking is off ([omarchy] link_settings_plugin)");
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("omarchy-link".into())
        .spawn(|| {
            let ctx = Context::from_env();
            let runner = TimeoutRunner::default();
            let shell = Shell::patient(&ctx, &runner);
            auto_link(&ctx, shell.as_ref(), true, &|title, body| {
                crate::notification::send_sync(title, body)
            });
        });
    if let Err(err) = spawned {
        tracing::warn!("Could not start the Omarchy Settings panel check: {err}");
    }
}

// ------------------------------------------------------------------ CLI

/// `voxtype setup omarchy` flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct CliArgs {
    pub status: bool,
    pub json: bool,
    pub replace: bool,
    pub remove: bool,
    pub restart_shell: bool,
    pub yes: bool,
    pub dismiss_copy: bool,
}

/// Entry point for `voxtype setup omarchy`.
pub fn run(args: CliArgs) -> anyhow::Result<()> {
    use std::io::IsTerminal;
    let ctx = Context::from_env();
    let runner = TimeoutRunner::default();
    let shell = Shell::new(&ctx, &runner);
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let mut stdout = std::io::stdout();
    execute(
        &ctx,
        shell.as_ref(),
        args,
        interactive,
        &mut confirm,
        &mut stdout,
    )
}

fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok()
        && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn display(path: Option<&Path>) -> String {
    path.map(|p| p.display().to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn print_status(status: &Status, out: &mut dyn Write) -> io::Result<()> {
    let version = |v: &Option<String>| v.as_deref().map(|v| format!(" ({v})")).unwrap_or_default();
    writeln!(
        out,
        "Omarchy:         {}",
        if status.omarchy {
            "detected"
        } else {
            "not detected"
        }
    )?;
    writeln!(
        out,
        "Packaged panel:  {}{}",
        display(status.packaged_dir.as_deref()),
        version(&status.packaged_version)
    )?;
    writeln!(out, "State:           {}", status.state.as_str())?;
    writeln!(
        out,
        "Panel entry:     {}{}",
        display(status.path.as_deref()),
        version(&status.installed_version)
    )?;
    if let Some(target) = &status.link_target {
        writeln!(out, "Points at:       {}", target.display())?;
    }
    writeln!(out)?;
    let hint = match status.state {
        LinkState::NotOmarchy => "This desktop is not Omarchy; there is nothing to link.".to_string(),
        LinkState::NoPackage => format!(
            "This install does not include the packaged panel. Install it from git:\n  {GIT_INSTALL_COMMAND}"
        ),
        LinkState::Absent => "Not linked. Link it with: voxtype setup omarchy".to_string(),
        LinkState::Linked => format!("Linked and loaded. Open it with: {OPEN_COMMAND}"),
        LinkState::LinkedStale => {
            "Linked, but Omarchy has not loaded this version yet. Load it with: voxtype setup omarchy"
                .to_string()
        }
        LinkState::Copy => "This is your own copy, which does not update with voxtype.\n\
             Switch to the packaged panel (your copy is backed up): voxtype setup omarchy --replace"
            .to_string(),
        LinkState::ForeignLink => "This link points somewhere other than the packaged panel.\n\
             Switch to the packaged panel (the link is backed up): voxtype setup omarchy --replace"
            .to_string(),
    };
    writeln!(out, "{hint}")
}

fn print_link_report(
    report: &LinkReport,
    restarted: Option<bool>,
    out: &mut dyn Write,
) -> io::Result<()> {
    if let Some(backup) = &report.backup {
        writeln!(out, "Moved your previous panel to {}", backup.display())?;
    }
    writeln!(
        out,
        "Linked {} -> {}{}",
        report.path.display(),
        report.target.display(),
        report
            .version
            .as_deref()
            .map(|v| format!(" ({v})"))
            .unwrap_or_default()
    )?;
    let loaded = report.rescanned || restarted == Some(true);
    match (loaded, report.enabled) {
        (true, Some(true)) | (true, None) => {
            writeln!(
                out,
                "Omarchy loaded the panel. Open it with: {OPEN_COMMAND}"
            )
        }
        (true, Some(false)) => writeln!(
            out,
            "Omarchy loaded the panel but did not confirm enabling it. Enable it with:\n  \
             omarchy plugin enable io.voxtype.settings"
        ),
        (false, _) => {
            if restarted == Some(false) {
                writeln!(out, "omarchy-restart-shell did not succeed.")?;
            }
            writeln!(
                out,
                "Omarchy's shell did not answer, so the panel loads the next time it starts.\n\
                 To load it now: voxtype setup omarchy --restart-shell"
            )
        }
    }
}

/// After `--restart-shell`: restart, then finish whatever the shell missed.
fn restart_and_finish(
    ctx: &Context,
    shell: Option<&Shell>,
    marker: &mut Marker,
    report: &mut LinkReport,
) -> bool {
    let Some(shell) = shell else { return false };
    if !shell.restart() {
        return false;
    }
    // The restarted shell scans at start and so loads the current link.
    marker.version = report.version.clone();
    report.rescanned = true;
    if report.enabled == Some(false) && shell.enable() {
        report.enabled = Some(true);
        marker.enable_pending = false;
    }
    save_marker(ctx, marker);
    true
}

/// The CLI logic, with the terminal, prompt and output injected.
pub fn execute(
    ctx: &Context,
    shell: Option<&Shell>,
    args: CliArgs,
    interactive: bool,
    confirm: &mut dyn FnMut(&str) -> bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let mut marker = Marker::load(&ctx.marker_path);
    let status = status(ctx, &marker);

    if !args.status && forget_stale_dismissal(ctx, &mut marker) {
        save_marker(ctx, &marker);
    }

    if args.status {
        if args.json {
            writeln!(out, "{}", serde_json::to_string_pretty(&status)?)?;
        } else {
            print_status(&status, out)?;
        }
        return Ok(());
    }

    if args.dismiss_copy {
        match dismiss_copy(ctx, &mut marker) {
            Some(path) => writeln!(
                out,
                "The Settings panel will stop offering to replace your copy at {}.\n\
                 Switch any time with: voxtype setup omarchy --replace",
                path.display()
            )?,
            None => writeln!(
                out,
                "There is no copy of the Settings panel to dismiss; nothing changed."
            )?,
        }
        return Ok(());
    }

    if args.remove {
        match remove(ctx, shell, &mut marker)? {
            Some(path) => {
                writeln!(out, "Removed {}", path.display())?;
                writeln!(
                    out,
                    "The daemon will not add it back. Link it again with: voxtype setup omarchy"
                )?;
                if args.restart_shell {
                    if let Some(shell) = shell {
                        shell.restart();
                    }
                }
            }
            None => writeln!(
                out,
                "The Voxtype Settings panel is not linked; nothing to remove."
            )?,
        }
        return Ok(());
    }

    match status.state {
        LinkState::NotOmarchy => return Err(OmarchyError::NotOmarchy.into()),
        LinkState::NoPackage => {
            // Informational, not a failure: older and source builds have no
            // packaged panel, and installers may call this unconditionally.
            writeln!(out, "{}", OmarchyError::NoPackage)?;
            return Ok(());
        }
        LinkState::Copy | LinkState::ForeignLink => {
            let path = display(status.path.as_deref());
            let question = format!(
                "Replace your copy of Voxtype Settings at {path} with the packaged one? \
                 Your copy is kept as a hidden backup beside it."
            );
            // Replacing takes --replace plus --yes, or a "yes" typed at a
            // prompt. --yes alone never replaces a copy.
            let consent = if args.replace && args.yes {
                true
            } else if interactive && !args.yes {
                confirm(&question)
            } else if args.replace {
                return Err(OmarchyError::NeedsConsent.into());
            } else {
                false
            };
            if !consent {
                writeln!(
                    out,
                    "Left your panel at {path} alone. It does not update with voxtype.\n\
                     To switch to the packaged panel: voxtype setup omarchy --replace"
                )?;
                return Ok(());
            }
            let mut report = replace(ctx, shell, &mut marker)?;
            let restarted = args
                .restart_shell
                .then(|| restart_and_finish(ctx, shell, &mut marker, &mut report));
            print_link_report(&report, restarted, out)?;
        }
        LinkState::Absent | LinkState::Linked | LinkState::LinkedStale => {
            let mut report = link(ctx, shell, &mut marker, true)?;
            let restarted = args
                .restart_shell
                .then(|| restart_and_finish(ctx, shell, &mut marker, &mut report));
            print_link_report(&report, restarted, out)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::os::unix::fs::symlink;

    const RESCAN: &str = "-q shell rescanPlugins";
    const ENABLE: &str = "shell enablePlugin io.voxtype.settings {}";
    const RESTART: &str = "<restart>";

    /// Records every call and answers from a per-command script. A command
    /// with no script fails, the way a missing shell does.
    #[derive(Default)]
    struct FakeRunner {
        calls: RefCell<Vec<String>>,
        /// Joined args -> queued answers; the last answer repeats.
        answers: RefCell<HashMap<String, Vec<Option<String>>>>,
    }

    impl FakeRunner {
        /// A running shell: every call succeeds and enable says ok.
        fn running() -> Self {
            let runner = Self::default();
            runner.answer(RESCAN, &[Some("")]);
            runner.answer(ENABLE, &[Some("ok\n")]);
            runner.answer(RESTART, &[Some("")]);
            runner
        }

        /// No shell process: every IPC call fails.
        fn stopped() -> Self {
            Self::default()
        }

        fn answer(&self, cmd: &str, answers: &[Option<&str>]) {
            self.answers.borrow_mut().insert(
                cmd.to_string(),
                answers.iter().map(|a| a.map(str::to_string)).collect(),
            );
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }

        fn count(&self, cmd: &str) -> usize {
            self.calls.borrow().iter().filter(|c| *c == cmd).count()
        }
    }

    impl CommandRunner for FakeRunner {
        fn output(&self, program: &str, args: &[&str]) -> Option<String> {
            let key = if program.ends_with("omarchy-restart-shell") {
                RESTART.to_string()
            } else {
                assert!(program.ends_with("omarchy-shell"), "unexpected {program}");
                args.join(" ")
            };
            self.calls.borrow_mut().push(key.clone());
            let mut answers = self.answers.borrow_mut();
            let queue = answers.get_mut(&key)?;
            if queue.len() > 1 {
                queue.remove(0)
            } else {
                queue.first().cloned().flatten()
            }
        }
    }

    fn no_notify(_: &str, _: &str) {}

    fn shell<'a>(ctx: &Context, runner: &'a FakeRunner) -> Shell<'a> {
        Shell::new(ctx, runner).unwrap().with_attempts(1, 3)
    }

    fn write_manifest(dir: &Path, id: &str, version: &str) {
        fs::create_dir_all(dir.join(version)).unwrap();
        fs::write(
            dir.join("manifest.json"),
            format!(
                r#"{{"schemaVersion":1,"id":"{id}","version":"{version}","entryPoints":{{"panel":"{version}/Panel.qml"}}}}"#
            ),
        )
        .unwrap();
        fs::write(dir.join(version).join("Panel.qml"), "Item {}\n").unwrap();
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        ctx: Context,
    }

    impl Fixture {
        /// An Omarchy machine with the packaged panel at `version` (contract
        /// 2.1 manifest) and an empty ~/.config/omarchy.
        fn new(version: &str) -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().to_path_buf();
            let packaged = root
                .join("usr/share/voxtype/omarchy-plugin")
                .join(PLUGIN_ID);
            write_manifest(&packaged, PLUGIN_ID, version);
            fs::create_dir_all(root.join("config/omarchy")).unwrap();
            let ctx = Context {
                config_home: Some(root.join("config")),
                omarchy_shell: Some(root.join("bin/omarchy-shell")),
                restart_shell: Some(root.join("bin/omarchy-restart-shell")),
                packaged_dir: Some(packaged.clone()),
                own_dirs: vec![packaged, root.join("usr/local/share/voxtype/old")],
                marker_path: root.join("state/voxtype").join(MARKER_FILE),
            };
            Self {
                _tmp: tmp,
                root,
                ctx,
            }
        }

        fn plugins(&self) -> PathBuf {
            self.ctx.plugins_dir().unwrap()
        }

        fn link_path(&self) -> PathBuf {
            self.ctx.link_path().unwrap()
        }

        fn packaged(&self) -> PathBuf {
            self.ctx.packaged_dir.clone().unwrap()
        }

        /// A package upgrade replaces the packaged tree in place.
        fn upgrade_package(&self, version: &str) {
            write_manifest(&self.packaged(), PLUGIN_ID, version);
        }

        fn marker(&self) -> Marker {
            Marker::load(&self.ctx.marker_path)
        }

        fn status(&self) -> Status {
            status(&self.ctx, &self.marker())
        }

        fn state(&self) -> LinkState {
            self.status().state
        }

        /// A git copy from `omarchy plugin add`, under `name`.
        fn add_copy(&self, name: &str, version: &str) -> PathBuf {
            let dir = self.plugins().join(name);
            write_manifest(&dir, PLUGIN_ID, version);
            fs::create_dir_all(dir.join(".git")).unwrap();
            fs::write(dir.join("user-file"), "keep me").unwrap();
            dir
        }

        /// A dev worktree linked in by hand at the managed path.
        fn add_foreign_link(&self, version: &str) -> PathBuf {
            let worktree = self.root.join("worktree/omarchy-plugin");
            write_manifest(&worktree, PLUGIN_ID, version);
            fs::create_dir_all(self.plugins()).unwrap();
            symlink(&worktree, self.link_path()).unwrap();
            worktree
        }

        fn link_target(&self) -> Option<PathBuf> {
            fs::read_link(self.link_path()).ok()
        }

        fn is_link(&self) -> bool {
            fs::symlink_metadata(self.link_path())
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        }

        fn hidden_entries(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.plugins())
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with('.'))
                .collect();
            names.sort();
            names
        }

        fn run_cli(
            &self,
            runner: &FakeRunner,
            args: CliArgs,
            interactive: bool,
            answer: bool,
        ) -> (anyhow::Result<()>, String, Vec<String>) {
            let shell = shell(&self.ctx, runner);
            let mut out = Vec::new();
            let mut asked = Vec::new();
            let mut confirm = |q: &str| {
                asked.push(q.to_string());
                answer
            };
            let result = execute(
                &self.ctx,
                Some(&shell),
                args,
                interactive,
                &mut confirm,
                &mut out,
            );
            (result, String::from_utf8(out).unwrap(), asked)
        }
    }

    // ------------------------------------------------------ packaged dir

    #[test]
    fn env_override_is_searched_before_system_dirs() {
        let dirs = candidate_dirs(Some("/opt/panel".into()));
        assert_eq!(dirs[0], PathBuf::from("/opt/panel"));
        assert_eq!(&dirs[1..], SYSTEM_PLUGIN_DIRS.map(PathBuf::from).as_slice());
        assert_eq!(
            candidate_dirs(Some("".into()))[0],
            PathBuf::from(SYSTEM_PLUGIN_DIRS[0])
        );
        assert_eq!(candidate_dirs(None).len(), SYSTEM_PLUGIN_DIRS.len());
    }

    #[test]
    fn packaged_dir_needs_our_manifest_id() {
        let tmp = tempfile::tempdir().unwrap();
        let wrong = tmp.path().join("wrong");
        let right = tmp.path().join("right");
        let missing = tmp.path().join("missing");
        write_manifest(&wrong, "com.example.other", "1.0.0");
        write_manifest(&right, PLUGIN_ID, "1.2.1");
        assert_eq!(
            select_packaged(&[missing.clone(), wrong.clone(), right.clone()]),
            Some(right)
        );
        assert_eq!(select_packaged(&[missing, wrong]), None);
    }

    // ------------------------------------------------------------ states

    #[test]
    fn not_omarchy_without_config_dir() {
        let f = Fixture::new("1.2.1");
        fs::remove_dir_all(f.root.join("config/omarchy")).unwrap();
        assert_eq!(f.state(), LinkState::NotOmarchy);
        assert!(!f.status().omarchy);
    }

    #[test]
    fn not_omarchy_without_shell() {
        let mut f = Fixture::new("1.2.1");
        f.ctx.omarchy_shell = None;
        assert_eq!(f.state(), LinkState::NotOmarchy);
    }

    #[test]
    fn no_package() {
        let mut f = Fixture::new("1.2.1");
        f.ctx.packaged_dir = None;
        let status = f.status();
        assert_eq!(status.state, LinkState::NoPackage);
        assert_eq!(status.packaged_version, None);
    }

    #[test]
    fn no_package_still_reports_a_copy() {
        let mut f = Fixture::new("1.2.1");
        f.ctx.packaged_dir = None;
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        let status = f.status();
        assert_eq!(status.state, LinkState::NoPackage);
        assert_eq!(status.path, Some(copy));
        assert_eq!(status.installed_version.as_deref(), Some("1.1.0"));
    }

    #[test]
    fn absent() {
        let f = Fixture::new("1.2.1");
        assert_eq!(f.state(), LinkState::Absent);
        fs::create_dir_all(f.plugins()).unwrap();
        assert_eq!(f.state(), LinkState::Absent);
    }

    #[test]
    fn another_plugin_does_not_count() {
        let f = Fixture::new("1.2.1");
        write_manifest(
            &f.plugins().join("superwhisper-panel"),
            "com.superwhisper",
            "2.0",
        );
        assert_eq!(f.state(), LinkState::Absent);
    }

    #[test]
    fn linked_after_link() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        link(
            &f.ctx,
            Some(&shell(&f.ctx, &runner)),
            &mut Marker::default(),
            true,
        )
        .unwrap();
        let status = f.status();
        assert_eq!(status.state, LinkState::Linked);
        assert_eq!(status.path, Some(f.link_path()));
        assert_eq!(status.installed_version.as_deref(), Some("1.2.1"));
        assert_eq!(status.packaged_version.as_deref(), Some("1.2.1"));
    }

    #[test]
    fn linked_stale_after_package_upgrade() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        link(
            &f.ctx,
            Some(&shell(&f.ctx, &runner)),
            &mut Marker::default(),
            true,
        )
        .unwrap();
        f.upgrade_package("1.2.2");
        let status = f.status();
        assert_eq!(status.state, LinkState::LinkedStale);
        // What the shell is still running, against what is packaged.
        assert_eq!(status.installed_version.as_deref(), Some("1.2.1"));
        assert_eq!(status.packaged_version.as_deref(), Some("1.2.2"));
    }

    #[test]
    fn own_link_without_marker_is_stale() {
        let f = Fixture::new("1.2.1");
        fs::create_dir_all(f.plugins()).unwrap();
        symlink(f.packaged(), f.link_path()).unwrap();
        assert_eq!(f.state(), LinkState::LinkedStale);
    }

    #[test]
    fn dangling_own_link_to_an_old_location_is_stale() {
        let f = Fixture::new("1.2.1");
        fs::create_dir_all(f.plugins()).unwrap();
        symlink(f.root.join("usr/local/share/voxtype/old"), f.link_path()).unwrap();
        f.ctx_marker_with_version("1.2.1");
        let status = f.status();
        assert_eq!(status.state, LinkState::LinkedStale);
        assert_eq!(status.path, Some(f.link_path()));
    }

    impl Fixture {
        fn ctx_marker_with_version(&self, version: &str) {
            Marker {
                auto_linked: true,
                version: Some(version.into()),
                ..Marker::default()
            }
            .save(&self.ctx.marker_path)
            .unwrap();
        }
    }

    #[test]
    fn copy_at_managed_path() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        let status = f.status();
        assert_eq!(status.state, LinkState::Copy);
        assert_eq!(status.path, Some(copy));
        assert_eq!(status.installed_version.as_deref(), Some("1.1.0"));
    }

    #[test]
    fn copy_under_another_name_is_found_by_id() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy("omarchy-voxtype-settings", "0.1.0");
        let status = f.status();
        assert_eq!(status.state, LinkState::Copy);
        assert_eq!(status.path, Some(copy));
    }

    #[test]
    fn hidden_backups_are_ignored() {
        let f = Fixture::new("1.2.1");
        f.add_copy(".io.voxtype.settings.backup-1700000000", "1.1.0");
        assert_eq!(f.state(), LinkState::Absent);
    }

    #[test]
    fn foreign_link() {
        let f = Fixture::new("1.2.1");
        let worktree = f.add_foreign_link("1.3.0-dev");
        let status = f.status();
        assert_eq!(status.state, LinkState::ForeignLink);
        assert_eq!(status.link_target, Some(worktree));
        assert_eq!(status.installed_version.as_deref(), Some("1.3.0-dev"));
    }

    #[test]
    fn dangling_foreign_link() {
        let f = Fixture::new("1.2.1");
        fs::create_dir_all(f.plugins()).unwrap();
        symlink(f.root.join("deleted-worktree"), f.link_path()).unwrap();
        let status = f.status();
        assert_eq!(status.state, LinkState::ForeignLink);
        assert_eq!(status.installed_version, None);
    }

    #[test]
    fn relative_link_to_the_package_is_ours() {
        let f = Fixture::new("1.2.1");
        fs::create_dir_all(f.plugins()).unwrap();
        symlink(
            "../../../usr/share/voxtype/omarchy-plugin/io.voxtype.settings",
            f.link_path(),
        )
        .unwrap();
        f.ctx_marker_with_version("1.2.1");
        assert_eq!(f.state(), LinkState::Linked);
    }

    #[test]
    fn status_json_matches_the_contract() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        link(
            &f.ctx,
            Some(&shell(&f.ctx, &runner)),
            &mut Marker::default(),
            true,
        )
        .unwrap();
        let json = serde_json::to_value(f.status()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "omarchy": true,
                "packaged_dir": f.packaged(),
                "packaged_version": "1.2.1",
                "state": "linked",
                "path": f.link_path(),
                "installed_version": "1.2.1",
                "copy_dismissed": false,
            })
        );
    }

    #[test]
    fn every_state_serializes_kebab_case() {
        for state in [
            LinkState::NotOmarchy,
            LinkState::NoPackage,
            LinkState::Absent,
            LinkState::Linked,
            LinkState::LinkedStale,
            LinkState::Copy,
            LinkState::ForeignLink,
        ] {
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::json!(state.as_str())
            );
        }
    }

    // ------------------------------------------------------------ link

    #[test]
    fn link_creates_symlink_rescans_enables_and_records() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        let mut marker = Marker::default();
        let report = link(&f.ctx, Some(&shell(&f.ctx, &runner)), &mut marker, true).unwrap();
        assert_eq!(f.link_target(), Some(f.packaged()));
        assert!(report.rescanned);
        assert_eq!(report.enabled, Some(true));
        assert_eq!(runner.calls(), vec![RESCAN, ENABLE]);
        assert_eq!(
            f.marker(),
            Marker {
                auto_linked: true,
                version: Some("1.2.1".into()),
                copy_noted: false,
                enable_pending: false,
                copy_dismissed: false,
            }
        );
        // No temp link left behind.
        assert!(f.hidden_entries().is_empty());
    }

    #[test]
    fn link_is_recorded_before_the_shell_is_asked() {
        // A daemon that exits mid-call (here: the shell call never returns
        // control to link) must already have recorded the link.
        struct Snoop<'a>(&'a Path);
        impl CommandRunner for Snoop<'_> {
            fn output(&self, _: &str, _: &[&str]) -> Option<String> {
                let marker = Marker::load(self.0);
                assert!(marker.auto_linked, "marker written before the rescan");
                assert!(
                    marker.enable_pending,
                    "enable assumed pending until confirmed"
                );
                None
            }
        }
        let f = Fixture::new("1.2.1");
        let runner = Snoop(&f.ctx.marker_path);
        let shell = Shell::new(&f.ctx, &runner).unwrap().with_attempts(1, 1);
        link(&f.ctx, Some(&shell), &mut Marker::default(), true).unwrap();
    }

    #[test]
    fn enable_retries_until_the_registry_answers_ok() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        runner.answer(ENABLE, &[Some("unknown"), Some("unknown"), Some("ok")]);
        let report = link(
            &f.ctx,
            Some(&shell(&f.ctx, &runner)),
            &mut Marker::default(),
            true,
        )
        .unwrap();
        assert_eq!(report.enabled, Some(true));
        assert_eq!(runner.count(ENABLE), 3);
    }

    #[test]
    fn enable_that_never_answers_ok_is_left_pending() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        runner.answer(ENABLE, &[Some("unknown")]);
        let report = link(
            &f.ctx,
            Some(&shell(&f.ctx, &runner)),
            &mut Marker::default(),
            true,
        )
        .unwrap();
        assert_eq!(report.enabled, Some(false));
        assert_eq!(runner.count(ENABLE), 3);
        assert!(f.marker().enable_pending);
    }

    #[test]
    fn shell_not_running_still_links_and_leaves_enable_pending() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::stopped();
        let report = link(
            &f.ctx,
            Some(&shell(&f.ctx, &runner)),
            &mut Marker::default(),
            true,
        )
        .unwrap();
        assert!(f.is_link());
        assert!(!report.rescanned);
        assert_eq!(report.enabled, Some(false));
        assert_eq!(runner.calls(), vec![RESCAN], "no enable without a shell");
        let marker = f.marker();
        assert!(marker.auto_linked && marker.enable_pending);
        assert_eq!(marker.version, None);
        assert_eq!(f.state(), LinkState::LinkedStale);
    }

    #[test]
    fn link_refuses_a_copy() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        let runner = FakeRunner::running();
        let err = link(
            &f.ctx,
            Some(&shell(&f.ctx, &runner)),
            &mut Marker::default(),
            true,
        )
        .unwrap_err();
        assert!(matches!(err, OmarchyError::NotOurs(p) if p == copy));
        assert!(copy.join("user-file").is_file());
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn link_refuses_a_foreign_link() {
        let f = Fixture::new("1.2.1");
        let worktree = f.add_foreign_link("1.3.0-dev");
        let err = link(&f.ctx, None, &mut Marker::default(), true).unwrap_err();
        assert!(matches!(err, OmarchyError::NotOurs(_)));
        assert_eq!(f.link_target(), Some(worktree));
    }

    #[test]
    fn placing_a_link_never_overwrites_a_directory() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        assert!(place_link(&f.plugins(), &f.packaged()).is_err());
        assert!(copy.join("user-file").is_file());
        assert!(f.hidden_entries().is_empty(), "temp link cleaned up");
    }

    #[test]
    fn link_repoints_our_dangling_link() {
        let f = Fixture::new("1.2.1");
        fs::create_dir_all(f.plugins()).unwrap();
        symlink(f.root.join("usr/local/share/voxtype/old"), f.link_path()).unwrap();
        let runner = FakeRunner::running();
        link(
            &f.ctx,
            Some(&shell(&f.ctx, &runner)),
            &mut Marker::default(),
            true,
        )
        .unwrap();
        assert_eq!(f.link_target(), Some(f.packaged()));
        assert_eq!(f.state(), LinkState::Linked);
    }

    // ---------------------------------------------------- replace/remove

    #[test]
    fn replace_moves_a_copy_to_a_hidden_backup() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        let runner = FakeRunner::running();
        let report = replace(
            &f.ctx,
            Some(&shell(&f.ctx, &runner)),
            &mut Marker::default(),
        )
        .unwrap();
        let backup = report.backup.unwrap();
        assert!(backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".io.voxtype.settings.backup-"));
        assert_eq!(
            fs::read_to_string(backup.join("user-file")).unwrap(),
            "keep me"
        );
        assert!(backup.join(".git").is_dir());
        assert_eq!(f.link_target(), Some(f.packaged()));
        assert_ne!(copy, backup);
        assert_eq!(f.state(), LinkState::Linked);
    }

    #[test]
    fn replace_moves_a_copy_under_another_name() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy("voxtype-settings", "0.1.0");
        let report = replace(&f.ctx, None, &mut Marker::default()).unwrap();
        assert!(!copy.exists());
        assert!(report.backup.unwrap().join("user-file").is_file());
        assert!(f.is_link());
    }

    #[test]
    fn replace_keeps_a_foreign_link_in_the_backup() {
        let f = Fixture::new("1.2.1");
        let worktree = f.add_foreign_link("1.3.0-dev");
        let report = replace(&f.ctx, None, &mut Marker::default()).unwrap();
        let backup = report.backup.unwrap();
        assert_eq!(fs::read_link(&backup).unwrap(), worktree);
        assert!(worktree.join("manifest.json").is_file());
        assert_eq!(f.link_target(), Some(f.packaged()));
    }

    #[test]
    fn backup_names_never_collide() {
        let f = Fixture::new("1.2.1");
        fs::create_dir_all(f.plugins()).unwrap();
        let first = backup_path(&f.plugins());
        fs::create_dir(&first).unwrap();
        let second = backup_path(&f.plugins());
        assert_ne!(first, second);
    }

    #[test]
    fn remove_deletes_only_our_link_and_remembers_it() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        let shell = shell(&f.ctx, &runner);
        let mut marker = Marker::default();
        link(&f.ctx, Some(&shell), &mut marker, true).unwrap();
        let removed = remove(&f.ctx, Some(&shell), &mut marker).unwrap();
        assert_eq!(removed, Some(f.link_path()));
        assert!(!f.is_link());
        assert!(
            f.packaged().join("manifest.json").is_file(),
            "package untouched"
        );
        assert!(f.marker().auto_linked);
        assert_eq!(runner.count(RESCAN), 2);
    }

    #[test]
    fn remove_refuses_a_copy_and_leaves_it() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        let err = remove(&f.ctx, None, &mut Marker::default()).unwrap_err();
        assert!(matches!(err, OmarchyError::RemoveRefused(_)));
        assert!(copy.join("user-file").is_file());
    }

    #[test]
    fn remove_refuses_a_foreign_link() {
        let f = Fixture::new("1.2.1");
        let worktree = f.add_foreign_link("1.3.0-dev");
        assert!(remove(&f.ctx, None, &mut Marker::default()).is_err());
        assert_eq!(f.link_target(), Some(worktree));
    }

    #[test]
    fn remove_with_nothing_linked_is_a_no_op() {
        let f = Fixture::new("1.2.1");
        assert_eq!(remove(&f.ctx, None, &mut Marker::default()).unwrap(), None);
    }

    // ------------------------------------------------------------ daemon

    fn status_in(state: LinkState) -> Status {
        Status {
            omarchy: true,
            packaged_dir: None,
            packaged_version: None,
            state,
            path: None,
            installed_version: None,
            copy_dismissed: false,
            link_target: None,
        }
    }

    #[test]
    fn daemon_rules() {
        let fresh = Marker::default();
        let linked_once = Marker {
            auto_linked: true,
            ..Marker::default()
        };
        let noted = Marker {
            copy_noted: true,
            ..Marker::default()
        };
        let pending = Marker {
            auto_linked: true,
            enable_pending: true,
            ..Marker::default()
        };
        use AutoAction::*;
        use LinkState::*;
        let cases = [
            (NotOmarchy, &fresh, Nothing),
            (NoPackage, &fresh, Nothing),
            (Absent, &fresh, Link),
            (Absent, &linked_once, Nothing),
            (Absent, &noted, Nothing),
            (Linked, &linked_once, Nothing),
            (Linked, &pending, RetryEnable),
            (LinkedStale, &linked_once, Refresh),
            (LinkedStale, &fresh, Refresh),
            (Copy, &fresh, NoteCopy),
            (Copy, &noted, Nothing),
            (ForeignLink, &fresh, NoteCopy),
            (ForeignLink, &noted, Nothing),
        ];
        for (state, marker, want) in cases {
            assert_eq!(
                decide(&status_in(state), marker, true),
                want,
                "{state:?} {marker:?}"
            );
            assert_eq!(
                decide(&status_in(state), marker, false),
                Nothing,
                "opt-out {state:?}"
            );
        }
    }

    #[test]
    fn daemon_first_run_links_and_enables() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        let action = auto_link(&f.ctx, Some(&shell(&f.ctx, &runner)), true, &no_notify);
        assert_eq!(action, AutoAction::Link);
        assert_eq!(f.link_target(), Some(f.packaged()));
        assert_eq!(runner.calls(), vec![RESCAN, ENABLE]);
        assert_eq!(f.state(), LinkState::Linked);
        // A second start does nothing at all.
        let again = FakeRunner::running();
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &again)), true, &no_notify),
            AutoAction::Nothing
        );
        assert!(again.calls().is_empty());
    }

    #[test]
    fn daemon_never_relinks_after_the_user_removed_it() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        auto_link(&f.ctx, Some(&shell(&f.ctx, &runner)), true, &no_notify);
        // `omarchy plugin remove` unlinks the symlink.
        fs::remove_file(f.link_path()).unwrap();
        let later = FakeRunner::running();
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &later)), true, &no_notify),
            AutoAction::Nothing
        );
        assert!(!f.is_link());
        assert!(later.calls().is_empty());
        // Even after an upgrade.
        f.upgrade_package("1.2.2");
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &later)), true, &no_notify),
            AutoAction::Nothing
        );
        assert!(!f.is_link());
    }

    #[test]
    fn daemon_never_relinks_after_cli_remove() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        let (result, _, _) = f.run_cli(&runner, CliArgs::default(), false, false);
        result.unwrap();
        let (result, _, _) = f.run_cli(
            &runner,
            CliArgs {
                remove: true,
                ..CliArgs::default()
            },
            false,
            false,
        );
        result.unwrap();
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &runner)), true, &no_notify),
            AutoAction::Nothing
        );
        assert!(!f.is_link());
    }

    #[test]
    fn daemon_rescans_when_the_packaged_version_changes() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        auto_link(&f.ctx, Some(&shell(&f.ctx, &runner)), true, &no_notify);
        f.upgrade_package("1.2.2");
        let after = FakeRunner::running();
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &after)), true, &no_notify),
            AutoAction::Refresh
        );
        assert_eq!(
            after.calls(),
            vec![RESCAN],
            "rescan only; enable is first-link only"
        );
        assert_eq!(f.marker().version.as_deref(), Some("1.2.2"));
        assert_eq!(f.state(), LinkState::Linked);
        assert_eq!(f.link_target(), Some(f.packaged()));
    }

    #[test]
    fn daemon_retries_the_rescan_at_next_start_when_the_shell_was_down() {
        let f = Fixture::new("1.2.1");
        auto_link(
            &f.ctx,
            Some(&shell(&f.ctx, &FakeRunner::running())),
            true,
            &no_notify,
        );
        f.upgrade_package("1.2.2");
        auto_link(
            &f.ctx,
            Some(&shell(&f.ctx, &FakeRunner::stopped())),
            true,
            &no_notify,
        );
        assert_eq!(f.state(), LinkState::LinkedStale);
        let up = FakeRunner::running();
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &up)), true, &no_notify),
            AutoAction::Refresh
        );
        assert_eq!(f.state(), LinkState::Linked);
    }

    #[test]
    fn daemon_leaves_a_copy_alone_and_notes_it_once() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        let runner = FakeRunner::running();
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &runner)), true, &no_notify),
            AutoAction::NoteCopy
        );
        assert!(f.marker().copy_noted);
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &runner)), true, &no_notify),
            AutoAction::Nothing
        );
        assert!(runner.calls().is_empty());
        assert!(copy.join("user-file").is_file());
        assert!(!f.is_link());
    }

    #[test]
    fn daemon_leaves_a_foreign_link_alone() {
        let f = Fixture::new("1.2.1");
        let worktree = f.add_foreign_link("1.3.0-dev");
        let runner = FakeRunner::running();
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &runner)), true, &no_notify),
            AutoAction::NoteCopy
        );
        assert_eq!(f.link_target(), Some(worktree));
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn daemon_does_not_link_after_the_user_removed_their_copy() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        auto_link(&f.ctx, None, true, &no_notify);
        fs::remove_dir_all(copy).unwrap();
        assert_eq!(
            auto_link(&f.ctx, None, true, &no_notify),
            AutoAction::Nothing
        );
        assert!(!f.is_link());
    }

    #[test]
    fn daemon_with_shell_down_links_then_enables_at_next_start() {
        let f = Fixture::new("1.2.1");
        let down = FakeRunner::stopped();
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &down)), true, &no_notify),
            AutoAction::Link
        );
        assert!(f.is_link());
        assert!(f.marker().enable_pending);

        // Next start: the stale rescan and the pending enable both happen.
        let up = FakeRunner::running();
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &up)), true, &no_notify),
            AutoAction::Refresh
        );
        assert_eq!(up.calls(), vec![RESCAN, ENABLE]);
        assert!(!f.marker().enable_pending);
        assert_eq!(f.state(), LinkState::Linked);
    }

    #[test]
    fn daemon_retries_a_pending_enable_on_a_current_link() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        runner.answer(ENABLE, &[Some("unknown")]);
        auto_link(&f.ctx, Some(&shell(&f.ctx, &runner)), true, &no_notify);
        assert!(f.marker().enable_pending);
        assert_eq!(f.state(), LinkState::Linked);

        let up = FakeRunner::running();
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &up)), true, &no_notify),
            AutoAction::RetryEnable
        );
        assert_eq!(up.calls(), vec![RESCAN, ENABLE]);
        assert!(!f.marker().enable_pending);
    }

    #[test]
    fn daemon_opt_out_touches_nothing() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        assert_eq!(
            auto_link(&f.ctx, Some(&shell(&f.ctx, &runner)), false, &no_notify),
            AutoAction::Nothing
        );
        assert!(runner.calls().is_empty());
        assert!(!f.is_link());
        assert!(!f.ctx.marker_path.exists());
    }

    #[test]
    fn spawn_auto_link_respects_the_config_opt_out() {
        // With the opt-out set nothing is spawned, so this returns at once
        // without reading the real environment.
        let mut config = Config::default();
        assert!(config.omarchy.link_settings_plugin, "on by default");
        config.omarchy.link_settings_plugin = false;
        spawn_auto_link(&config);
    }

    #[test]
    fn daemon_does_nothing_off_omarchy_or_without_a_package() {
        let mut f = Fixture::new("1.2.1");
        f.ctx.packaged_dir = None;
        assert_eq!(
            auto_link(&f.ctx, None, true, &no_notify),
            AutoAction::Nothing
        );
        f.ctx.omarchy_shell = None;
        assert_eq!(
            auto_link(&f.ctx, None, true, &no_notify),
            AutoAction::Nothing
        );
        assert!(!f.is_link());
        assert!(!f.ctx.marker_path.exists());
    }

    // ------------------------------------------- copy notice and dismissal

    /// Collects notifications instead of sending them.
    #[derive(Default)]
    struct Notices(RefCell<Vec<(String, String)>>);

    impl Notices {
        fn run(&self, f: &Fixture, shell: Option<&Shell>) -> AutoAction {
            auto_link(&f.ctx, shell, true, &|t: &str, b: &str| {
                self.0.borrow_mut().push((t.to_string(), b.to_string()))
            })
        }

        fn sent(&self) -> Vec<(String, String)> {
            self.0.borrow().clone()
        }
    }

    #[test]
    fn daemon_notifies_once_when_it_first_finds_a_copy() {
        let f = Fixture::new("1.2.1");
        f.add_copy(PLUGIN_ID, "1.1.0");
        let notices = Notices::default();
        let runner = FakeRunner::running();
        assert_eq!(
            notices.run(&f, Some(&shell(&f.ctx, &runner))),
            AutoAction::NoteCopy
        );
        assert_eq!(
            notices.sent(),
            vec![(
                "Voxtype Settings panel".to_string(),
                "Your Settings panel is a copy that does not update with voxtype. \
                 Run voxtype setup omarchy --replace to switch to the packaged one."
                    .to_string()
            )]
        );
        assert!(!COPY_NOTICE_BODY.contains('\u{2014}'), "no em dashes");
        // Later starts never repeat it.
        notices.run(&f, Some(&shell(&f.ctx, &runner)));
        notices.run(&f, None);
        assert_eq!(notices.sent().len(), 1);
        assert!(runner.calls().is_empty(), "the notice needs no shell call");
    }

    #[test]
    fn daemon_does_not_notify_for_a_foreign_link() {
        let f = Fixture::new("1.2.1");
        f.add_foreign_link("1.3.0-dev");
        let notices = Notices::default();
        assert_eq!(notices.run(&f, None), AutoAction::NoteCopy);
        assert!(notices.sent().is_empty());
    }

    #[test]
    fn daemon_does_not_notify_about_a_dismissed_copy() {
        let f = Fixture::new("1.2.1");
        f.add_copy(PLUGIN_ID, "1.1.0");
        dismiss_copy(&f.ctx, &mut Marker::default()).unwrap();
        let notices = Notices::default();
        assert_eq!(notices.run(&f, None), AutoAction::NoteCopy);
        assert!(notices.sent().is_empty());
        assert!(
            f.marker().copy_dismissed,
            "a present copy keeps its dismissal"
        );
    }

    #[test]
    fn daemon_opt_out_sends_no_notice() {
        let f = Fixture::new("1.2.1");
        f.add_copy(PLUGIN_ID, "1.1.0");
        let sent = RefCell::new(0);
        auto_link(&f.ctx, None, false, &|_: &str, _: &str| {
            *sent.borrow_mut() += 1
        });
        assert_eq!(*sent.borrow(), 0);
        assert!(!f.marker().copy_noted);
    }

    #[test]
    fn old_markers_read_as_not_dismissed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(MARKER_FILE);
        fs::write(
            &path,
            r#"{"auto_linked":false,"version":null,"copy_noted":true,"enable_pending":false}"#,
        )
        .unwrap();
        let marker = Marker::load(&path);
        assert!(marker.copy_noted);
        assert!(!marker.copy_dismissed);
    }

    #[test]
    fn dismiss_copy_is_remembered_and_reported() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        assert!(!f.status().copy_dismissed);
        let (result, out, _) = f.run_cli(
            &FakeRunner::running(),
            CliArgs {
                dismiss_copy: true,
                ..CliArgs::default()
            },
            false,
            false,
        );
        result.unwrap();
        assert!(out.contains("voxtype setup omarchy --replace"), "{out}");
        assert!(f.marker().copy_dismissed);
        let status = f.status();
        assert_eq!(status.state, LinkState::Copy);
        assert!(status.copy_dismissed);
        assert!(
            copy.join("user-file").is_file(),
            "dismissing changes no files"
        );
    }

    #[test]
    fn dismiss_without_a_copy_changes_nothing() {
        let f = Fixture::new("1.2.1");
        for setup in [None, Some("link")] {
            if setup.is_some() {
                f.add_foreign_link("1.3.0-dev");
            }
            let (result, out, _) = f.run_cli(
                &FakeRunner::running(),
                CliArgs {
                    dismiss_copy: true,
                    ..CliArgs::default()
                },
                false,
                false,
            );
            result.unwrap();
            assert!(out.contains("nothing changed"), "{out}");
            assert!(!f.marker().copy_dismissed);
        }
    }

    #[test]
    fn replace_clears_a_dismissal() {
        let f = Fixture::new("1.2.1");
        f.add_copy(PLUGIN_ID, "1.1.0");
        let mut marker = Marker::default();
        dismiss_copy(&f.ctx, &mut marker).unwrap();
        replace(&f.ctx, None, &mut marker).unwrap();
        assert!(!f.marker().copy_dismissed);
        assert!(!f.status().copy_dismissed);
    }

    #[test]
    fn a_dismissal_ends_when_the_copy_goes_away() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        dismiss_copy(&f.ctx, &mut Marker::default()).unwrap();
        fs::remove_dir_all(&copy).unwrap();
        // Reported false at once, even before anything rewrites the marker.
        assert!(!f.status().copy_dismissed);
        // The daemon forgets it, so a later copy offers the switch again.
        auto_link(&f.ctx, None, true, &no_notify);
        assert!(!f.marker().copy_dismissed);
        f.add_copy(PLUGIN_ID, "1.1.0");
        assert!(!f.status().copy_dismissed);
    }

    #[test]
    fn a_cli_action_forgets_a_stale_dismissal_but_status_writes_nothing() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        dismiss_copy(&f.ctx, &mut Marker::default()).unwrap();
        fs::remove_dir_all(&copy).unwrap();
        let status = CliArgs {
            status: true,
            json: true,
            ..CliArgs::default()
        };
        f.run_cli(&FakeRunner::running(), status, false, false)
            .0
            .unwrap();
        assert!(f.marker().copy_dismissed, "--status is read-only");
        f.run_cli(&FakeRunner::running(), CliArgs::default(), false, false)
            .0
            .unwrap();
        assert!(!f.marker().copy_dismissed);
    }

    #[test]
    fn status_json_ends_with_copy_dismissed() {
        let f = Fixture::new("1.2.1");
        f.add_copy(PLUGIN_ID, "1.1.0");
        dismiss_copy(&f.ctx, &mut Marker::default()).unwrap();
        let text = serde_json::to_string(&f.status()).unwrap();
        let keys: Vec<&str> = [
            "\"omarchy\"",
            "\"packaged_dir\"",
            "\"packaged_version\"",
            "\"state\"",
            "\"path\"",
            "\"installed_version\"",
            "\"copy_dismissed\"",
        ]
        .to_vec();
        let positions: Vec<usize> = keys.iter().map(|k| text.find(k).unwrap()).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{text}");
        assert!(text.ends_with("\"copy_dismissed\":true}"), "{text}");
    }

    // --------------------------------------------------------------- CLI

    #[test]
    fn cli_status_json_prints_the_contract_and_changes_nothing() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        let (result, out, _) = f.run_cli(
            &runner,
            CliArgs {
                status: true,
                json: true,
                ..CliArgs::default()
            },
            true,
            true,
        );
        result.unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["state"], "absent");
        assert_eq!(value["path"], serde_json::Value::Null);
        assert!(runner.calls().is_empty());
        assert!(!f.is_link());
    }

    #[test]
    fn cli_status_human_names_the_next_step() {
        let f = Fixture::new("1.2.1");
        f.add_copy(PLUGIN_ID, "1.1.0");
        let (result, out, _) = f.run_cli(
            &FakeRunner::running(),
            CliArgs {
                status: true,
                ..CliArgs::default()
            },
            false,
            false,
        );
        result.unwrap();
        assert!(out.contains("State:           copy"), "{out}");
        assert!(out.contains("voxtype setup omarchy --replace"), "{out}");
    }

    #[test]
    fn cli_links_when_absent() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        let (result, out, asked) = f.run_cli(&runner, CliArgs::default(), false, false);
        result.unwrap();
        assert!(asked.is_empty());
        assert!(out.contains("Omarchy loaded the panel"), "{out}");
        assert_eq!(f.state(), LinkState::Linked);
    }

    #[test]
    fn cli_suggests_a_restart_when_the_shell_does_not_answer() {
        let f = Fixture::new("1.2.1");
        let (result, out, _) = f.run_cli(&FakeRunner::stopped(), CliArgs::default(), false, false);
        result.unwrap();
        assert!(
            out.contains("voxtype setup omarchy --restart-shell"),
            "{out}"
        );
        assert!(f.is_link());
    }

    #[test]
    fn cli_restart_shell_restarts_and_finishes_enabling() {
        let f = Fixture::new("1.2.1");
        let runner = FakeRunner::running();
        runner.answer(RESCAN, &[None]);
        let (result, out, _) = f.run_cli(
            &runner,
            CliArgs {
                restart_shell: true,
                ..CliArgs::default()
            },
            false,
            false,
        );
        result.unwrap();
        assert_eq!(runner.calls(), vec![RESCAN, RESTART, ENABLE]);
        assert!(out.contains("Omarchy loaded the panel"), "{out}");
        let marker = f.marker();
        assert!(!marker.enable_pending);
        assert_eq!(marker.version.as_deref(), Some("1.2.1"));
    }

    #[test]
    fn cli_yes_alone_never_replaces_a_copy() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        let runner = FakeRunner::running();
        for interactive in [false, true] {
            let (result, out, asked) = f.run_cli(
                &runner,
                CliArgs {
                    yes: true,
                    ..CliArgs::default()
                },
                interactive,
                true,
            );
            result.unwrap();
            assert!(asked.is_empty());
            assert!(out.contains("Left your panel"), "{out}");
        }
        assert!(copy.join("user-file").is_file());
        assert!(f.hidden_entries().is_empty());
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn cli_replace_without_a_terminal_needs_yes() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        let (result, _, _) = f.run_cli(
            &FakeRunner::running(),
            CliArgs {
                replace: true,
                ..CliArgs::default()
            },
            false,
            true,
        );
        assert!(result.unwrap_err().to_string().contains("--yes"));
        assert!(copy.join("user-file").is_file());
        assert!(f.hidden_entries().is_empty());
    }

    #[test]
    fn cli_prompt_declined_keeps_the_copy() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        let (result, _, asked) = f.run_cli(&FakeRunner::running(), CliArgs::default(), true, false);
        result.unwrap();
        assert_eq!(asked.len(), 1);
        assert!(asked[0].starts_with("Replace your copy of Voxtype Settings"));
        assert!(copy.join("user-file").is_file());
        assert!(!f.is_link());
    }

    #[test]
    fn cli_prompt_accepted_replaces_with_a_backup() {
        let f = Fixture::new("1.2.1");
        f.add_copy(PLUGIN_ID, "1.1.0");
        let (result, out, _) = f.run_cli(&FakeRunner::running(), CliArgs::default(), true, true);
        result.unwrap();
        assert!(out.contains("Moved your previous panel to"), "{out}");
        assert_eq!(f.hidden_entries().len(), 1);
        assert_eq!(f.state(), LinkState::Linked);
    }

    #[test]
    fn cli_replace_yes_restart_shell_as_the_panel_banner_runs_it() {
        let f = Fixture::new("1.2.1");
        f.add_copy(PLUGIN_ID, "1.1.0");
        let runner = FakeRunner::running();
        let (result, _, asked) = f.run_cli(
            &runner,
            CliArgs {
                replace: true,
                yes: true,
                restart_shell: true,
                ..CliArgs::default()
            },
            false,
            false,
        );
        result.unwrap();
        assert!(asked.is_empty());
        assert_eq!(runner.calls(), vec![RESCAN, ENABLE, RESTART]);
        assert_eq!(f.state(), LinkState::Linked);
        assert_eq!(f.hidden_entries().len(), 1);
    }

    #[test]
    fn cli_remove_refuses_a_copy() {
        let f = Fixture::new("1.2.1");
        let copy = f.add_copy(PLUGIN_ID, "1.1.0");
        let (result, _, _) = f.run_cli(
            &FakeRunner::running(),
            CliArgs {
                remove: true,
                ..CliArgs::default()
            },
            false,
            false,
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("omarchy plugin remove"));
        assert!(copy.join("user-file").is_file());
    }

    #[test]
    fn cli_not_omarchy_is_an_error() {
        let f = Fixture::new("1.2.1");
        fs::remove_dir_all(f.root.join("config/omarchy")).unwrap();
        let (result, _, _) = f.run_cli(&FakeRunner::running(), CliArgs::default(), false, false);
        assert!(result.is_err());
    }

    #[test]
    fn cli_without_a_package_prints_the_git_install() {
        let mut f = Fixture::new("1.2.1");
        f.ctx.packaged_dir = None;
        let runner = FakeRunner::running();
        let (result, out, _) = f.run_cli(&runner, CliArgs::default(), false, false);
        result.unwrap();
        assert!(out.contains(GIT_INSTALL_COMMAND), "{out}");
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn marker_tolerates_missing_and_corrupt_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("m.json");
        assert_eq!(Marker::load(&path), Marker::default());
        fs::write(&path, "not json").unwrap();
        assert_eq!(Marker::load(&path), Marker::default());
        fs::write(&path, r#"{"auto_linked":true}"#).unwrap();
        assert!(Marker::load(&path).auto_linked);
    }

    #[test]
    fn timeout_runner_returns_stdout_and_gives_up_on_hangs() {
        let quick = TimeoutRunner {
            timeout: Duration::from_secs(5),
        };
        assert_eq!(
            quick.output("sh", &["-c", "echo ok"]).as_deref(),
            Some("ok\n")
        );
        assert_eq!(quick.output("sh", &["-c", "exit 3"]), None);
        assert_eq!(quick.output("/nonexistent/omarchy-shell", &[]), None);
        // A grandchild holding stdout open must not block the read.
        assert_eq!(
            quick
                .output("sh", &["-c", "sleep 3 & echo started"])
                .as_deref(),
            Some("started\n")
        );
        let slow = TimeoutRunner {
            timeout: Duration::from_millis(200),
        };
        assert_eq!(slow.output("sleep", &["5"]), None);
    }
}
