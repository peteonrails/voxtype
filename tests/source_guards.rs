//! Source-level guards.
//!
//! Facts that must hold across the tree but cannot be expressed in the type
//! system: a name that must exist in exactly one place, a literal that must not
//! come back. Each guard names the desired end state, so it fails if the bad
//! pattern is reintroduced anywhere, including in a file that does not exist yet.
//!
//! Only production code is scanned. Test code legitimately spells out paths that
//! production code must derive (a test builds its own runtime dir and lockfile),
//! so everything from the first `#[cfg(test)]` onwards in a file is ignored.

use std::path::{Path, PathBuf};

/// Repo root, so the guards read the same tree the compiler does.
fn src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The production half of a source file: everything before its test module.
fn production(text: &str) -> &str {
    match text.find("#[cfg(test)]") {
        Some(idx) => &text[..idx],
        None => text,
    }
}

/// Relative paths of every `.rs` file under `root` whose production code
/// contains `needle`. The unit the guards reason in, small enough to self-test.
fn hits(root: &Path, needle: &str) -> Vec<String> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files);
    files.sort();
    files
        .into_iter()
        .filter(|path| {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            production(&text).contains(needle)
        })
        .map(|path| {
            path.strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string()
        })
        .collect()
}

/// Files that name a lockfile or hardcode the runtime directory, minus the
/// module that owns the derivation. Kept separate from the test so the
/// exemption rule can be pinned against a planted tree.
fn lock_path_offenders(root: &Path) -> Vec<String> {
    /// `daemon_status` derives every lock path; it is the one place allowed to.
    const OWNER: &str = "daemon_status.rs";
    let mut offenders: Vec<String> = Vec::new();
    for needle in ["voxtype.lock", "menubar.lock", r#""/tmp/voxtype/"#] {
        for rel in hits(root, needle) {
            if !rel.ends_with(OWNER) && !offenders.contains(&rel) {
                offenders.push(rel);
            }
        }
    }
    offenders.sort();
    offenders
}

#[test]
fn legacy_pid_file_is_gone() {
    // The daemon used to write `runtime_dir()/pid` at startup, keep the path in a
    // field, and delete it at shutdown. Nothing ever read it: liveness goes
    // through the lockfile (`voxtype.lock`), which is what `daemon_status` owns.
    let offenders = hits(&src_root(), r#"join("pid")"#);
    assert!(
        offenders.is_empty(),
        "the legacy pid file is back in {offenders:?}; liveness is the lockfile, \
         see daemon_status::pid_file_path"
    );
}

#[test]
fn lockfile_path_is_derived_in_one_place() {
    // `daemon_status::pid_file_path()` and `menubar_lock_path()` exist so a
    // rename reaches every consumer at once. The derivation had already drifted:
    // the daemon spelled out the same path the helper derives, and the macOS
    // launch path hardcoded /tmp/voxtype, which is not even the directory in use
    // when XDG_RUNTIME_DIR is set.
    let offenders = lock_path_offenders(&src_root());
    assert!(
        offenders.is_empty(),
        "these files name a lockfile or the runtime directory instead of \
         deriving it from daemon_status: {offenders:?}"
    );
}

#[test]
fn detector_sees_planted_violations_and_ignores_tests() {
    // The guards above are only worth their runtime if the scan can fail, and
    // only worth their false-positive rate if it ignores test modules. Both
    // directions get pinned here against a tree built for the purpose.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("planted_violations");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("nested")).expect("temp tree");
    std::fs::write(
        root.join("nested/production.rs"),
        "fn write_it() { let p = Config::runtime_dir().join(\"pid\"); }\n",
    )
    .expect("planted violation");
    // A production-looking use here must stay invisible: the file's test module
    // is allowed to spell out what production code has to derive.
    std::fs::write(
        root.join("tests_only.rs"),
        "fn helper() { let p = dir.join(\"state\"); }\n\n#[cfg(test)]\nmod tests {\n    \
         fn t() { let p = Config::runtime_dir().join(\"pid\"); }\n}\n",
    )
    .expect("test-only use");

    let offenders = hits(&root, r#"join("pid")"#);

    assert_eq!(
        offenders,
        vec!["nested/production.rs".to_string()],
        "scan must report the production hit and only the production hit"
    );
}

#[test]
fn lock_guard_flags_literals_and_exempts_its_owner() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("planted_lock_paths");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("app")).expect("temp tree");
    std::fs::write(
        root.join("daemon_status.rs"),
        "pub fn pid_file_path() -> PathBuf { Config::runtime_dir().join(\"voxtype.lock\") }\n",
    )
    .expect("owner module");
    std::fs::write(
        root.join("app/dispatch.rs"),
        "let _ = std::fs::remove_file(\"/tmp/voxtype/voxtype.lock\");\n\
         let p = Config::runtime_dir().join(\"menubar.lock\");\n",
    )
    .expect("planted literals");
    std::fs::write(
        root.join("app/clean.rs"),
        "let _ = std::fs::remove_file(crate::daemon_status::pid_file_path());\n",
    )
    .expect("clean file");

    let offenders = lock_path_offenders(&root);

    assert_eq!(
        offenders,
        vec!["app/dispatch.rs".to_string()],
        "the owner module must be exempt, derived call sites clean, and both \
         the hardcoded directory and the bare name must be flagged"
    );
}
