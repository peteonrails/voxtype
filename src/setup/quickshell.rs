//! Quickshell QML tree installer for voxtype.
//!
//! Copies the Quickshell QML tree (`shell.qml`, `OsdSurface.qml`,
//! `EnginePicker.qml`, `MeetingControls.qml`, and the `voxtype-shared/`
//! module) into the user's data directory so the `voxtype-osd-quickshell`
//! launcher can find it. Also prints Hyprland/Sway/River keybinding
//! examples for the Wave 2 widgets that toggle via flag files in
//! `$XDG_RUNTIME_DIR/voxtype/`.
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

use std::env;
use std::fs;
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
pub fn run(
    target: Option<PathBuf>,
    source: Option<PathBuf>,
    force: bool,
    print_bindings_only: bool,
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

    println!();
    print_bindings();

    println!();
    println!(
        "Quickshell config installed to {}.",
        resolved_target.display()
    );
    println!(
        "Run with: voxtype-osd-quickshell  (or set [osd] frontend = \"quickshell\" in config.toml)"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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

    #[test]
    fn default_target_dir_honors_xdg_data_home() {
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
