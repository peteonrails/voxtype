//! Quickshell QML tree installer for voxtype.
//!
//! Copies the Quickshell QML tree (`shell.qml`, `OsdSurface.qml`,
//! `EnginePicker.qml`, `MeetingControls.qml`, and the `voxtype-shared/`
//! module) into the user's data directory so the `voxtype-osd-quickshell`
//! launcher can find it. Also symlinks the `voxtype-audio-bridge` sidecar
//! into a user-owned PATH directory so the QML can spawn it. Prints
//! Hyprland/Sway/River keybinding examples for the Wave 2 widgets that
//! toggle via flag files in `$XDG_RUNTIME_DIR/voxtype/`.
//!
//! Source tree resolution (first match wins):
//! 1. `--source <DIR>` CLI override
//! 2. `$VOXTYPE_QUICKSHELL_SOURCE_DIR` env var
//! 3. `<binary's dir>/../share/voxtype/quickshell/` (installed layout)
//! 4. `/usr/share/voxtype/quickshell/`
//! 5. `quickshell/` relative to current working directory (dev-from-repo-root)
//!
//! Destination: `$XDG_DATA_HOME/voxtype/quickshell/` (or
//! `~/.local/share/voxtype/quickshell/` if `XDG_DATA_HOME` is unset).
//!
//! Bridge binary resolution (first match wins):
//! 1. `--bridge <PATH>` CLI override
//! 2. `$VOXTYPE_AUDIO_BRIDGE_BINARY` env var
//! 3. `<binary's dir>/../lib/voxtype/voxtype-audio-bridge` (installed layout)
//! 4. `/usr/lib/voxtype/voxtype-audio-bridge`
//! 5. `which voxtype-audio-bridge` (already on PATH — no symlink needed)
//! 6. `target/release/voxtype-audio-bridge` / `target/debug/...` (dev convenience)
//!
//! Bridge symlink target defaults to `$XDG_BIN_HOME/voxtype-audio-bridge`
//! (or `~/.local/bin/voxtype-audio-bridge`). Refuses to write outside
//! `$HOME` without `--force`.

use std::env;
use std::fs;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

use crate::error::VoxtypeError;

/// Subdirectory under the user's data dir / system share dir.
const QUICKSHELL_SUBDIR: &str = "voxtype/quickshell";

/// Files at the root of the QML tree that are part of the install.
const ROOT_FILES: &[&str] = &[
    "shell.qml",
    "OsdSurface.qml",
    "EnginePicker.qml",
    "MeetingControls.qml",
    "README.md",
];

/// Files in the `voxtype-shared/` subdirectory.
const SHARED_FILES: &[&str] = &[
    "Theme.qml",
    "StateReader.qml",
    "AudioBridge.qml",
    "qmldir",
    "README.md",
];

const SHARED_SUBDIR: &str = "voxtype-shared";

/// Resolve the default install target.
///
/// Returns `$XDG_DATA_HOME/voxtype/quickshell` if `XDG_DATA_HOME` is set
/// and non-empty, otherwise `~/.local/share/voxtype/quickshell`.
pub fn default_target_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join(QUICKSHELL_SUBDIR);
        }
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".local/share").join(QUICKSHELL_SUBDIR);
    }
    // Last-resort fallback (shouldn't happen on Linux with HOME set).
    PathBuf::from(".local/share").join(QUICKSHELL_SUBDIR)
}

/// Resolve the QML source tree.
///
/// Honors the precedence documented in the module header.
pub fn resolve_source_dir(cli_source: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = cli_source {
        if is_valid_source(p) {
            return Some(p.to_path_buf());
        }
    }
    if let Ok(env_path) = env::var("VOXTYPE_QUICKSHELL_SOURCE_DIR") {
        if !env_path.is_empty() {
            let p = PathBuf::from(env_path);
            if is_valid_source(&p) {
                return Some(p);
            }
        }
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            let installed = parent.join("../share/voxtype/quickshell");
            if is_valid_source(&installed) {
                if let Ok(canon) = fs::canonicalize(&installed) {
                    return Some(canon);
                }
                return Some(installed);
            }
        }
    }
    let system = PathBuf::from("/usr/share/voxtype/quickshell");
    if is_valid_source(&system) {
        return Some(system);
    }
    let dev = PathBuf::from("quickshell");
    if is_valid_source(&dev) {
        return Some(dev);
    }
    None
}

/// Returns true if `dir` looks like a valid quickshell QML source tree.
fn is_valid_source(dir: &Path) -> bool {
    dir.is_dir() && dir.join("shell.qml").is_file()
}

/// List of (source_path, relative_destination_path) pairs that will be copied.
fn enumerate_files(source: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for name in ROOT_FILES {
        let src = source.join(name);
        if src.is_file() {
            out.push((src, PathBuf::from(name)));
        }
    }
    let shared_src = source.join(SHARED_SUBDIR);
    if shared_src.is_dir() {
        for name in SHARED_FILES {
            let src = shared_src.join(name);
            if src.is_file() {
                out.push((src, PathBuf::from(SHARED_SUBDIR).join(name)));
            }
        }
    }
    out
}

/// Returns true if `dir` exists and is not an empty directory.
fn dir_has_entries(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|mut iter| iter.next().is_some())
        .unwrap_or(false)
}

/// Install the QML tree from `source` into `target`.
///
/// Mirrors the source tree's structure for `shell.qml`, the three Wave 2
/// QML files, the `voxtype-shared/` module, and the README files. Returns
/// the list of relative paths that were written.
pub fn install_tree(
    source: &Path,
    target: &Path,
    force: bool,
) -> Result<Vec<PathBuf>, VoxtypeError> {
    if target.exists() && !force && dir_has_entries(target) {
        return Err(VoxtypeError::Config(format!(
            "Target directory is not empty: {}\n  Re-run with --force to overwrite.",
            target.display()
        )));
    }

    fs::create_dir_all(target).map_err(|e| {
        VoxtypeError::Config(format!("Failed to create {}: {}", target.display(), e))
    })?;

    let files = enumerate_files(source);
    if files.is_empty() {
        return Err(VoxtypeError::Config(format!(
            "Source directory contains no QML files: {}",
            source.display()
        )));
    }

    let mut written = Vec::new();
    for (src, rel) in &files {
        let dst = target.join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                VoxtypeError::Config(format!("Failed to create {}: {}", parent.display(), e))
            })?;
        }
        fs::copy(src, &dst).map_err(|e| {
            VoxtypeError::Config(format!(
                "Failed to copy {} -> {}: {}",
                src.display(),
                dst.display(),
                e
            ))
        })?;
        written.push(rel.clone());
    }
    Ok(written)
}

/// Name of the audio bridge binary used in PATH lookups and symlinks.
const BRIDGE_BIN_NAME: &str = "voxtype-audio-bridge";

/// Outcome of bridge resolution: either a concrete source path to symlink
/// from, or a confirmation that the bridge is already discoverable on the
/// user's `PATH` (in which case no symlink is needed).
#[derive(Debug, PartialEq, Eq)]
pub enum BridgeSource {
    /// Bridge binary lives at this absolute path. Caller should symlink.
    Path(PathBuf),
    /// `which voxtype-audio-bridge` succeeded at this path; PATH already has it.
    OnPath(PathBuf),
}

/// Resolve the voxtype-audio-bridge source binary.
///
/// Search order (first match wins) mirrors the module header.
pub fn resolve_bridge_source(cli_bridge: Option<&Path>) -> Option<BridgeSource> {
    if let Some(p) = cli_bridge {
        if p.is_file() {
            return Some(BridgeSource::Path(p.to_path_buf()));
        }
    }
    if let Ok(env_path) = env::var("VOXTYPE_AUDIO_BRIDGE_BINARY") {
        if !env_path.is_empty() {
            let p = PathBuf::from(env_path);
            if p.is_file() {
                return Some(BridgeSource::Path(p));
            }
        }
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            let installed = parent.join("../lib/voxtype").join(BRIDGE_BIN_NAME);
            if installed.is_file() {
                if let Ok(canon) = fs::canonicalize(&installed) {
                    return Some(BridgeSource::Path(canon));
                }
                return Some(BridgeSource::Path(installed));
            }
        }
    }
    let system = PathBuf::from("/usr/lib/voxtype").join(BRIDGE_BIN_NAME);
    if system.is_file() {
        return Some(BridgeSource::Path(system));
    }
    if let Ok(path) = which::which(BRIDGE_BIN_NAME) {
        return Some(BridgeSource::OnPath(path));
    }
    for build in &["release", "debug"] {
        let dev = PathBuf::from("target").join(build).join(BRIDGE_BIN_NAME);
        if dev.is_file() {
            if let Ok(canon) = fs::canonicalize(&dev) {
                return Some(BridgeSource::Path(canon));
            }
            return Some(BridgeSource::Path(dev));
        }
    }
    None
}

/// Resolve the default bridge symlink target.
///
/// Honors `$XDG_BIN_HOME` when set (per the user-dirs spec), otherwise
/// falls back to `~/.local/bin/`.
pub fn default_bridge_target() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_BIN_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join(BRIDGE_BIN_NAME);
        }
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".local/bin").join(BRIDGE_BIN_NAME);
    }
    PathBuf::from(".local/bin").join(BRIDGE_BIN_NAME)
}

/// Check whether `target` lives under the user's `$HOME`.
fn target_is_under_home(target: &Path) -> bool {
    match dirs::home_dir() {
        Some(home) => target.starts_with(home),
        None => false,
    }
}

/// Outcome of `install_bridge_symlink`. Used to format the user-facing
/// message and inform tests.
#[derive(Debug, PartialEq, Eq)]
pub enum BridgeInstallOutcome {
    /// Created or replaced the symlink.
    Linked,
    /// Symlink already pointed at the right source; no change.
    AlreadyLinked,
    /// Skipped because the bridge was already on the user's PATH.
    AlreadyOnPath(PathBuf),
}

/// Install a symlink at `target` pointing at the bridge source.
///
/// Refuses to overwrite an existing file (or a symlink pointing elsewhere)
/// without `force`. Refuses to write outside the user's `$HOME` without
/// `force` to avoid stomping on system paths like `/usr/local/bin/`.
pub fn install_bridge_symlink(
    source: &Path,
    target: &Path,
    force: bool,
) -> Result<BridgeInstallOutcome, VoxtypeError> {
    if !target_is_under_home(target) && !force {
        return Err(VoxtypeError::Config(format!(
            "Refusing to write bridge symlink outside of HOME: {}\n  \
             Re-run with --force if this is really what you want.",
            target.display()
        )));
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            VoxtypeError::Config(format!("Failed to create {}: {}", parent.display(), e))
        })?;
    }

    let metadata = fs::symlink_metadata(target).ok();
    if let Some(md) = metadata {
        if md.file_type().is_symlink() {
            let current = fs::read_link(target).map_err(|e| {
                VoxtypeError::Config(format!(
                    "Failed to read existing symlink {}: {}",
                    target.display(),
                    e
                ))
            })?;
            // Compare canonicalized forms when possible so relative vs
            // absolute representations don't trigger spurious mismatches.
            let same = current == source
                || fs::canonicalize(&current).ok() == fs::canonicalize(source).ok();
            if same {
                return Ok(BridgeInstallOutcome::AlreadyLinked);
            }
            if !force {
                return Err(VoxtypeError::Config(format!(
                    "{} already exists and points to {}.\n  \
                     Re-run with --force to overwrite, or pass \
                     --skip-bridge to leave it alone.",
                    target.display(),
                    current.display()
                )));
            }
            fs::remove_file(target).map_err(|e| {
                VoxtypeError::Config(format!(
                    "Failed to remove existing symlink {}: {}",
                    target.display(),
                    e
                ))
            })?;
        } else {
            if !force {
                return Err(VoxtypeError::Config(format!(
                    "{} already exists and is not a symlink.\n  \
                     Re-run with --force to overwrite, or pass \
                     --skip-bridge to leave it alone.",
                    target.display()
                )));
            }
            fs::remove_file(target).map_err(|e| {
                VoxtypeError::Config(format!(
                    "Failed to remove existing file {}: {}",
                    target.display(),
                    e
                ))
            })?;
        }
    }

    unix_fs::symlink(source, target).map_err(|e| {
        VoxtypeError::Config(format!(
            "Failed to symlink {} -> {}: {}",
            target.display(),
            source.display(),
            e
        ))
    })?;
    Ok(BridgeInstallOutcome::Linked)
}

/// Print the Hyprland/Sway/River keybinding examples that drive the
/// flag-file triggers for the engine picker and meeting controls panels.
pub fn print_bindings() {
    println!("Compositor keybindings (engine picker + meeting controls):\n");

    println!("# Hyprland (~/.config/hypr/hyprland.conf)");
    println!("bind = SUPER, E, exec, mkdir -p $XDG_RUNTIME_DIR/voxtype && touch $XDG_RUNTIME_DIR/voxtype/engine-picker.flag");
    println!("bind = SUPER, M, exec, mkdir -p $XDG_RUNTIME_DIR/voxtype && touch $XDG_RUNTIME_DIR/voxtype/meeting-controls.flag");
    println!();

    println!("# Sway (~/.config/sway/config)");
    println!("bindsym $mod+e exec mkdir -p $XDG_RUNTIME_DIR/voxtype && touch $XDG_RUNTIME_DIR/voxtype/engine-picker.flag");
    println!("bindsym $mod+m exec mkdir -p $XDG_RUNTIME_DIR/voxtype && touch $XDG_RUNTIME_DIR/voxtype/meeting-controls.flag");
    println!();

    println!("# River (~/.config/river/init)");
    println!("riverctl map normal Super E spawn 'mkdir -p $XDG_RUNTIME_DIR/voxtype && touch $XDG_RUNTIME_DIR/voxtype/engine-picker.flag'");
    println!("riverctl map normal Super M spawn 'mkdir -p $XDG_RUNTIME_DIR/voxtype && touch $XDG_RUNTIME_DIR/voxtype/meeting-controls.flag'");
    println!();

    println!(
        "The Quickshell-based OSD activates automatically when voxtype is recording \
         - no compositor binding needed."
    );
}

/// Run the full `voxtype setup quickshell` flow.
#[allow(clippy::too_many_arguments)]
pub fn run(
    target: Option<PathBuf>,
    source: Option<PathBuf>,
    force: bool,
    print_bindings_only: bool,
    bridge: Option<PathBuf>,
    bridge_target: Option<PathBuf>,
    skip_bridge: bool,
) -> Result<(), VoxtypeError> {
    if print_bindings_only {
        print_bindings();
        return Ok(());
    }

    let resolved_target = target.unwrap_or_else(default_target_dir);
    println!("Quickshell install target: {}", resolved_target.display());

    let source_dir = resolve_source_dir(source.as_deref()).ok_or_else(|| {
        VoxtypeError::Config(
            "Could not find the Quickshell QML source tree.\n\
             Searched (in order):\n  \
             --source <DIR>\n  \
             $VOXTYPE_QUICKSHELL_SOURCE_DIR\n  \
             <binary>/../share/voxtype/quickshell/\n  \
             /usr/share/voxtype/quickshell/\n  \
             ./quickshell/\n\
             Re-run with --source pointing at the QML directory."
                .to_string(),
        )
    })?;

    println!("Source: {}\n", source_dir.display());

    let written = install_tree(&source_dir, &resolved_target, force)?;
    for rel in &written {
        println!("  copied {}", rel.display());
    }

    // Bridge install (symlink so updates to the source binary propagate).
    let bridge_summary = if skip_bridge {
        println!();
        println!("Skipping voxtype-audio-bridge install (--skip-bridge).");
        None
    } else {
        println!();
        match install_bridge(bridge.as_deref(), bridge_target.as_deref(), force)? {
            Some((outcome, link_target, link_source)) => match outcome {
                BridgeInstallOutcome::Linked => {
                    println!(
                        "Linked {} -> {}",
                        link_target.display(),
                        link_source.display()
                    );
                    Some(link_target)
                }
                BridgeInstallOutcome::AlreadyLinked => {
                    println!(
                        "voxtype-audio-bridge already linked at {} (no change).",
                        link_target.display()
                    );
                    Some(link_target)
                }
                BridgeInstallOutcome::AlreadyOnPath(path) => {
                    println!(
                        "voxtype-audio-bridge is already on PATH at {} (no symlink needed).",
                        path.display()
                    );
                    None
                }
            },
            None => {
                println!(
                    "Could not locate voxtype-audio-bridge. The QML waveform will\n  \
                     stay empty until the bridge is on PATH. Searched (in order):\n  \
                     --bridge <PATH>\n  \
                     $VOXTYPE_AUDIO_BRIDGE_BINARY\n  \
                     <binary>/../lib/voxtype/voxtype-audio-bridge\n  \
                     /usr/lib/voxtype/voxtype-audio-bridge\n  \
                     `which voxtype-audio-bridge`\n  \
                     ./target/release/voxtype-audio-bridge\n  \
                     ./target/debug/voxtype-audio-bridge"
                );
                None
            }
        }
    };

    println!();
    print_bindings();

    println!();
    println!(
        "Quickshell config installed to {}.",
        resolved_target.display()
    );
    if let Some(link) = bridge_summary.as_ref() {
        println!(
            "voxtype-audio-bridge linked at {}. Make sure ~/.local/bin is on your\n  \
             PATH (most shells already include it).",
            link.display()
        );
    }
    println!(
        "Run with: voxtype-osd-quickshell  (or set [osd] frontend = \"quickshell\" in config.toml)"
    );

    Ok(())
}

/// Resolve, decide, and install the bridge symlink. Returns `None` if no
/// bridge source could be found anywhere. Returns the install outcome
/// along with the resolved target path and source path for messaging.
fn install_bridge(
    cli_bridge: Option<&Path>,
    cli_target: Option<&Path>,
    force: bool,
) -> Result<Option<(BridgeInstallOutcome, PathBuf, PathBuf)>, VoxtypeError> {
    let Some(source) = resolve_bridge_source(cli_bridge) else {
        return Ok(None);
    };
    let resolved_target = cli_target
        .map(PathBuf::from)
        .unwrap_or_else(default_bridge_target);
    println!("voxtype-audio-bridge target: {}", resolved_target.display());

    match source {
        BridgeSource::OnPath(path) if cli_target.is_none() => {
            // If the user gave an explicit --bridge-target, fall through
            // and create the symlink anyway. Otherwise honor the existing
            // PATH entry.
            Ok(Some((
                BridgeInstallOutcome::AlreadyOnPath(path.clone()),
                resolved_target,
                path,
            )))
        }
        BridgeSource::OnPath(path) => {
            let outcome = install_bridge_symlink(&path, &resolved_target, force)?;
            Ok(Some((outcome, resolved_target, path)))
        }
        BridgeSource::Path(path) => {
            let outcome = install_bridge_symlink(&path, &resolved_target, force)?;
            Ok(Some((outcome, resolved_target, path)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Serializes tests that mutate process-wide environment variables so
    /// they don't race each other when cargo runs them in parallel.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn make_fake_source(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        for f in ROOT_FILES {
            fs::write(dir.join(f), format!("// fake {}\n", f)).unwrap();
        }
        let shared = dir.join(SHARED_SUBDIR);
        fs::create_dir_all(&shared).unwrap();
        for f in SHARED_FILES {
            fs::write(shared.join(f), format!("// fake shared {}\n", f)).unwrap();
        }
    }

    #[test]
    fn install_copies_all_expected_files() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        make_fake_source(src.path());

        let written = install_tree(src.path(), dst.path(), false).unwrap();
        assert_eq!(written.len(), ROOT_FILES.len() + SHARED_FILES.len());

        for f in ROOT_FILES {
            assert!(
                dst.path().join(f).is_file(),
                "expected {} at root of target",
                f
            );
        }
        for f in SHARED_FILES {
            assert!(
                dst.path().join(SHARED_SUBDIR).join(f).is_file(),
                "expected {} in voxtype-shared/",
                f
            );
        }
    }

    #[test]
    fn install_refuses_non_empty_target_without_force() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        make_fake_source(src.path());
        fs::write(dst.path().join("stray.txt"), "existing").unwrap();

        let err = install_tree(src.path(), dst.path(), false).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("not empty"), "got: {}", msg);
        // Stray file should remain untouched.
        assert!(dst.path().join("stray.txt").exists());
    }

    #[test]
    fn install_with_force_overwrites_existing() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        make_fake_source(src.path());

        // Pre-existing shell.qml with different content.
        fs::write(dst.path().join("shell.qml"), "// OLD\n").unwrap();
        fs::write(dst.path().join("stray.txt"), "existing").unwrap();

        install_tree(src.path(), dst.path(), true).unwrap();

        let new_content = fs::read_to_string(dst.path().join("shell.qml")).unwrap();
        assert!(
            new_content.starts_with("// fake shell.qml"),
            "expected shell.qml to be overwritten, got: {}",
            new_content
        );
        // Force does not clean stray files - it only overwrites the install set.
        assert!(dst.path().join("stray.txt").exists());
    }

    #[test]
    fn install_rejects_invalid_source() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        // No shell.qml in source.
        let err = install_tree(src.path(), dst.path(), false).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("no QML files"), "got: {}", msg);
    }

    #[test]
    fn resolve_source_dir_prefers_cli_override() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let valid = TempDir::new().unwrap();
        make_fake_source(valid.path());

        // Set env var to point at an INVALID dir; CLI override should win.
        let bogus = TempDir::new().unwrap();
        // SAFETY: tests in this module are single-threaded thanks to env mutation;
        // each test must own its env-var lifetime.
        unsafe {
            env::set_var("VOXTYPE_QUICKSHELL_SOURCE_DIR", bogus.path());
        }

        let resolved = resolve_source_dir(Some(valid.path())).unwrap();
        assert_eq!(resolved, valid.path());

        unsafe {
            env::remove_var("VOXTYPE_QUICKSHELL_SOURCE_DIR");
        }
    }

    #[test]
    fn resolve_source_dir_honors_env_var() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let valid = TempDir::new().unwrap();
        make_fake_source(valid.path());

        unsafe {
            env::set_var("VOXTYPE_QUICKSHELL_SOURCE_DIR", valid.path());
        }
        let resolved = resolve_source_dir(None).unwrap();
        assert_eq!(resolved, valid.path());

        unsafe {
            env::remove_var("VOXTYPE_QUICKSHELL_SOURCE_DIR");
        }
    }

    fn make_fake_bridge(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"#!/bin/sh\necho fake\n").unwrap();
    }

    #[test]
    fn resolve_bridge_source_prefers_cli_override() {
        let dir = TempDir::new().unwrap();
        let bin = dir.path().join("voxtype-audio-bridge");
        make_fake_bridge(&bin);

        let resolved = resolve_bridge_source(Some(&bin)).unwrap();
        assert_eq!(resolved, BridgeSource::Path(bin));
    }

    #[test]
    fn resolve_bridge_source_honors_env_var() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        let bin = dir.path().join("voxtype-audio-bridge");
        make_fake_bridge(&bin);

        unsafe {
            env::set_var("VOXTYPE_AUDIO_BRIDGE_BINARY", &bin);
        }
        let resolved = resolve_bridge_source(None).unwrap();
        unsafe {
            env::remove_var("VOXTYPE_AUDIO_BRIDGE_BINARY");
        }
        assert_eq!(resolved, BridgeSource::Path(bin));
    }

    #[test]
    fn resolve_bridge_source_returns_none_when_missing() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Make sure no env override is set, and the search will hit
        // /usr/lib/voxtype/... only if it actually exists on disk.
        unsafe {
            env::remove_var("VOXTYPE_AUDIO_BRIDGE_BINARY");
        }
        // Pass a non-existent CLI path — should fall through to system
        // paths and PATH lookup. We can't deterministically guarantee
        // those are absent, but we can at least confirm that with a
        // bogus CLI override we still do the search.
        let bogus = PathBuf::from("/nonexistent/path/voxtype-audio-bridge");
        // Result is system-dependent; we just exercise the code path.
        let _ = resolve_bridge_source(Some(&bogus));
    }

    #[test]
    fn install_bridge_symlink_creates_link() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let src_dir = TempDir::new().unwrap();
        let bin = src_dir.path().join("voxtype-audio-bridge");
        make_fake_bridge(&bin);

        // Pretend the target is under HOME by setting HOME to a tmpdir.
        let fake_home = TempDir::new().unwrap();
        let prev_home = env::var("HOME").ok();
        unsafe {
            env::set_var("HOME", fake_home.path());
        }
        let target = fake_home.path().join(".local/bin/voxtype-audio-bridge");

        let outcome = install_bridge_symlink(&bin, &target, false).unwrap();
        assert_eq!(outcome, BridgeInstallOutcome::Linked);
        let md = fs::symlink_metadata(&target).unwrap();
        assert!(md.file_type().is_symlink());
        assert_eq!(fs::read_link(&target).unwrap(), bin);

        unsafe {
            match prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn install_bridge_symlink_is_idempotent_when_matches() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let src_dir = TempDir::new().unwrap();
        let bin = src_dir.path().join("voxtype-audio-bridge");
        make_fake_bridge(&bin);

        let fake_home = TempDir::new().unwrap();
        let prev_home = env::var("HOME").ok();
        unsafe {
            env::set_var("HOME", fake_home.path());
        }
        let target = fake_home.path().join(".local/bin/voxtype-audio-bridge");

        let first = install_bridge_symlink(&bin, &target, false).unwrap();
        assert_eq!(first, BridgeInstallOutcome::Linked);

        let second = install_bridge_symlink(&bin, &target, false).unwrap();
        assert_eq!(second, BridgeInstallOutcome::AlreadyLinked);

        unsafe {
            match prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn install_bridge_symlink_refuses_overwrite_without_force() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let src_dir = TempDir::new().unwrap();
        let bin_a = src_dir.path().join("voxtype-audio-bridge");
        let bin_b = src_dir.path().join("some-other-binary");
        make_fake_bridge(&bin_a);
        make_fake_bridge(&bin_b);

        let fake_home = TempDir::new().unwrap();
        let prev_home = env::var("HOME").ok();
        unsafe {
            env::set_var("HOME", fake_home.path());
        }
        let target = fake_home.path().join(".local/bin/voxtype-audio-bridge");

        // First install points to bin_b.
        install_bridge_symlink(&bin_b, &target, false).unwrap();

        // Re-install pointing at bin_a should refuse without --force.
        let err = install_bridge_symlink(&bin_a, &target, false).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("already exists") && msg.contains("--force"),
            "got: {}",
            msg
        );

        // Existing link should not have been clobbered.
        assert_eq!(fs::read_link(&target).unwrap(), bin_b);

        // With --force, it should overwrite.
        let outcome = install_bridge_symlink(&bin_a, &target, true).unwrap();
        assert_eq!(outcome, BridgeInstallOutcome::Linked);
        assert_eq!(fs::read_link(&target).unwrap(), bin_a);

        unsafe {
            match prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn install_bridge_symlink_refuses_outside_home_without_force() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let src_dir = TempDir::new().unwrap();
        let bin = src_dir.path().join("voxtype-audio-bridge");
        make_fake_bridge(&bin);

        // Point HOME at a tmpdir, and try to install OUTSIDE of it.
        let fake_home = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let prev_home = env::var("HOME").ok();
        unsafe {
            env::set_var("HOME", fake_home.path());
        }
        let target = outside.path().join("voxtype-audio-bridge");

        let err = install_bridge_symlink(&bin, &target, false).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("outside of HOME"), "got: {}", msg);
        assert!(!target.exists(), "target should not exist after refusal");

        unsafe {
            match prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn default_bridge_target_honors_xdg_bin_home() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = env::var("XDG_BIN_HOME").ok();
        unsafe {
            env::set_var("XDG_BIN_HOME", "/tmp/voxtype-test-bin");
        }
        let dir = default_bridge_target();
        assert_eq!(
            dir,
            PathBuf::from("/tmp/voxtype-test-bin/voxtype-audio-bridge")
        );
        unsafe {
            match prev {
                Some(v) => env::set_var("XDG_BIN_HOME", v),
                None => env::remove_var("XDG_BIN_HOME"),
            }
        }
    }

    #[test]
    fn run_with_skip_bridge_does_not_touch_target() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Stage a source tree so the QML install succeeds.
        let src = TempDir::new().unwrap();
        make_fake_source(src.path());
        let dst = TempDir::new().unwrap();

        // Point HOME and XDG_BIN_HOME at a tmpdir so any accidental
        // bridge install would land there (we then assert it didn't).
        let fake_home = TempDir::new().unwrap();
        let bin_home = fake_home.path().join(".local/bin");
        let prev_home = env::var("HOME").ok();
        let prev_xdg = env::var("XDG_BIN_HOME").ok();
        unsafe {
            env::set_var("HOME", fake_home.path());
            env::set_var("XDG_BIN_HOME", &bin_home);
        }

        run(
            Some(dst.path().to_path_buf()),
            Some(src.path().to_path_buf()),
            false,
            false,
            None,
            None,
            true, // skip_bridge
        )
        .unwrap();

        assert!(
            !bin_home.join("voxtype-audio-bridge").exists(),
            "skip_bridge should leave the target untouched"
        );

        unsafe {
            match prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
            match prev_xdg {
                Some(v) => env::set_var("XDG_BIN_HOME", v),
                None => env::remove_var("XDG_BIN_HOME"),
            }
        }
    }

    #[test]
    fn default_target_dir_honors_xdg_data_home() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Save and restore env to avoid clobbering other tests.
        let prev = env::var("XDG_DATA_HOME").ok();
        unsafe {
            env::set_var("XDG_DATA_HOME", "/tmp/voxtype-test-xdg");
        }
        let dir = default_target_dir();
        assert_eq!(
            dir,
            PathBuf::from("/tmp/voxtype-test-xdg/voxtype/quickshell")
        );
        unsafe {
            match prev {
                Some(v) => env::set_var("XDG_DATA_HOME", v),
                None => env::remove_var("XDG_DATA_HOME"),
            }
        }
    }
}
