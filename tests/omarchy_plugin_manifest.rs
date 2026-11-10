//! The Omarchy Settings plugin's manifest version has to track the release.
//!
//! Packages ship the plugin's QML under `<version>/`, named by the manifest
//! (`scripts/stage-omarchy-plugin.sh`). Omarchy reloads a panel only when its
//! entry URL changes, and Quickshell keeps compiled QML cached by URL, so a
//! manifest left at an old version would ship new QML under an old path and
//! users would keep running the stale panel. 1.2.0 shipped with the manifest
//! still at `1.2.0-rc1`; this test makes the next forgotten bump fail
//! `cargo test` instead of reaching a user.
//!
//! The second test runs the stage script itself, so the packaged layout every
//! package depends on is checked in CI without building a package.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read_manifest(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

#[test]
fn plugin_manifest_version_matches_cargo_version() {
    let manifest = read_manifest(&repo().join("omarchy-plugin/manifest.json"));
    assert_eq!(
        manifest["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "omarchy-plugin/manifest.json \"version\" must equal Cargo.toml's version; \
         bump them together"
    );
    // The repo keeps the flat layout so `omarchy plugin add` from git works.
    assert_eq!(manifest["entryPoints"]["panel"].as_str(), Some("Panel.qml"));
}

fn relative_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    out.sort();
    out
}

#[test]
fn stage_script_produces_the_packaged_layout() {
    let version = env!("CARGO_PKG_VERSION");
    let dest: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("omarchy-plugin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);

    let status = Command::new("bash")
        .arg(repo().join("scripts/stage-omarchy-plugin.sh"))
        .arg(repo().join("omarchy-plugin"))
        .arg(&dest)
        .status()
        .expect("bash must be available");
    assert!(status.success(), "stage-omarchy-plugin.sh failed");

    let plugin = dest.join("io.voxtype.settings");
    let staged = read_manifest(&plugin.join("manifest.json"));
    let source = read_manifest(&repo().join("omarchy-plugin/manifest.json"));
    let entry = format!("{version}/Panel.qml");
    assert_eq!(
        staged["entryPoints"]["panel"].as_str(),
        Some(entry.as_str())
    );
    assert_eq!(staged["version"].as_str(), Some(version));
    // Only the entry point changes; every other field is carried over.
    let mut expected = source.clone();
    expected["entryPoints"]["panel"] = serde_json::Value::String(entry.clone());
    assert_eq!(staged, expected);

    let mut want = vec![
        "LICENSE".to_string(),
        "manifest.json".to_string(),
        entry.clone(),
    ];
    for component in std::fs::read_dir(repo().join("omarchy-plugin/components"))
        .unwrap()
        .flatten()
    {
        let name = component.file_name().to_string_lossy().into_owned();
        if name.ends_with(".qml") {
            want.push(format!("{version}/components/{name}"));
        }
    }
    want.sort();
    assert_eq!(relative_files(&plugin), want);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        for file in &want {
            assert_eq!(mode(&plugin.join(file)), 0o644, "{file}");
        }
        for dir in [
            plugin.clone(),
            plugin.join(version),
            plugin.join(version).join("components"),
        ] {
            assert_eq!(mode(&dir), 0o755, "{}", dir.display());
        }
    }

    // A second run into the same root must refuse rather than merge versions.
    let again = Command::new("bash")
        .arg(repo().join("scripts/stage-omarchy-plugin.sh"))
        .arg(repo().join("omarchy-plugin"))
        .arg(&dest)
        .output()
        .unwrap();
    assert!(
        !again.status.success(),
        "restaging over an existing tree must fail"
    );

    std::fs::remove_dir_all(&dest).unwrap();
}
