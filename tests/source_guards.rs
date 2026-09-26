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
fn detector_sees_planted_violations_and_ignores_tests() {
    // The guard above is only worth its runtime if the scan can fail, and only
    // worth its false-positive rate if it ignores test modules. Both directions
    // get pinned here against a tree built for the purpose.
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
