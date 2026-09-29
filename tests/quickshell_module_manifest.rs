//! The `voxtype-shared` QML module has to declare exactly the components it
//! ships, and every packaging path has to ship exactly what it declares.
//!
//! This has now broken twice in the same way. In 0.7.5 the AUR binary package
//! omitted `voxtype-audio-bridge` (#488). In 1.0.0 it shipped a `qmldir`
//! declaring five components while installing three, so Quickshell could not
//! resolve `VT.StyleLoader`, the OSD child exited 255, the supervisor gave up
//! after three attempts, and `osd.frontend = quickshell` left users with no
//! OSD and nothing but a daemon log to explain it (#697).
//!
//! Both had the same cause: a hand-written file list in a PKGBUILD drifting
//! from the tree. `scripts/package.sh` avoids it by tarring the whole
//! `quickshell/` directory, so deb and rpm were never affected.
//!
//! This test cannot see the AUR PKGBUILDs, which live in gitignored nested
//! repos. What it can do is pin the upstream half of the contract, so a
//! component added to `qmldir` without its file (or the reverse) fails here
//! rather than on a user's machine.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn shared_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("quickshell/voxtype-shared")
}

/// Component names declared in `qmldir`, from lines of the form
/// `[singleton] <Name> <version> <File>.qml`.
fn declared_components(qmldir: &str) -> BTreeSet<String> {
    qmldir
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("module "))
        .filter_map(|l| {
            let file = l.split_whitespace().last()?;
            file.strip_suffix(".qml").map(str::to_string)
        })
        .collect()
}

/// `.qml` files actually present in the module directory.
fn present_components(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .expect("quickshell/voxtype-shared must exist")
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            name.strip_suffix(".qml").map(str::to_string)
        })
        .collect()
}

/// Regression for #697. A `qmldir` promising a component the module does not
/// contain is a runtime failure in Quickshell, not a warning, and it takes the
/// whole OSD down rather than degrading.
#[test]
fn qmldir_declares_exactly_the_components_that_exist() {
    let dir = shared_dir();
    let qmldir = std::fs::read_to_string(dir.join("qmldir")).expect("qmldir must exist");

    let declared = declared_components(&qmldir);
    let present = present_components(&dir);

    let missing: Vec<_> = declared.difference(&present).collect();
    let undeclared: Vec<_> = present.difference(&declared).collect();

    assert!(
        missing.is_empty(),
        "qmldir declares {missing:?} but those .qml files are not in the module. \
         Quickshell fails to load and the OSD dies (#697)."
    );
    assert!(
        undeclared.is_empty(),
        "{undeclared:?} exist in the module but qmldir does not declare them, \
         so `import voxtype-shared 1.0` cannot resolve them."
    );
}

/// The module is only usable if every file it declares is also packaged.
/// `scripts/package.sh` copies the whole tree, so this pins the property that
/// makes that safe: nothing outside `quickshell/` is required for the module
/// to resolve.
#[test]
fn the_module_is_self_contained() {
    let dir = shared_dir();
    let qmldir = std::fs::read_to_string(dir.join("qmldir")).expect("qmldir must exist");

    for line in qmldir.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with("module ") {
            continue;
        }
        let Some(file) = line.split_whitespace().last() else {
            continue;
        };
        assert!(
            !file.contains('/'),
            "qmldir entry {line:?} points outside the module directory. \
             Packaging copies quickshell/ as a unit, so a path escaping it \
             would ship broken."
        );
    }
}
