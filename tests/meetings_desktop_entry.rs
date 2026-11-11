//! The Voxtype Meetings launcher entry: what it runs, and that every
//! package format installs it and the icon it names.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn key<'a>(entry: &'a str, name: &str) -> Option<&'a str> {
    entry
        .lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix('='))
}

#[test]
fn entry_opens_the_meeting_view_with_the_voxtype_icon() {
    let entry = read("packaging/voxtype-meetings.desktop");
    assert!(entry.starts_with("[Desktop Entry]\n"));
    assert_eq!(key(&entry, "Type"), Some("Application"));
    assert_eq!(key(&entry, "Name"), Some("Voxtype Meetings"));
    assert_eq!(key(&entry, "Exec"), Some("voxtype meeting view"));
    assert_eq!(key(&entry, "Icon"), Some("voxtype"));
    assert_eq!(key(&entry, "Terminal"), Some("false"));
    assert!(repo().join("assets/icon.svg").is_file());
    assert!(repo().join("assets/icon.png").is_file());
}

#[test]
fn every_package_format_installs_the_entry_and_the_icon() {
    let installs = [
        ("scripts/package.sh", read("scripts/package.sh")),
        ("packaging/debian/rules", read("packaging/debian/rules")),
        (
            "packaging/rpm/voxtype.spec",
            read("packaging/rpm/voxtype.spec"),
        ),
    ];
    for (file, text) in &installs {
        for needle in [
            "packaging/voxtype-meetings.desktop",
            "applications/voxtype-meetings.desktop",
            "assets/icon.svg",
            "hicolor/scalable/apps/voxtype.svg",
            "hicolor/128x128/apps/voxtype.png",
        ] {
            assert!(text.contains(needle), "{file} does not mention {needle}");
        }
    }
    // rpm fails the build on installed-but-unlisted files.
    let spec = read("packaging/rpm/voxtype.spec");
    let files = spec.split("%files").nth(1).expect("spec has %files");
    for needle in [
        "%{_datadir}/applications/voxtype-meetings.desktop",
        "%{_datadir}/icons/hicolor/scalable/apps/voxtype.svg",
        "%{_datadir}/icons/hicolor/128x128/apps/voxtype.png",
    ] {
        assert!(files.contains(needle), "%files lacks {needle}");
    }
}

#[test]
fn package_script_parses() {
    let status = std::process::Command::new("bash")
        .arg("-n")
        .arg(repo().join("scripts/package.sh"))
        .status()
        .expect("bash runs");
    assert!(status.success(), "bash -n scripts/package.sh failed");
}

#[test]
fn entry_validates_when_the_validator_is_installed() {
    let entry: &Path = &repo().join("packaging/voxtype-meetings.desktop");
    match std::process::Command::new("desktop-file-validate")
        .arg(entry)
        .output()
    {
        Ok(out) => assert!(
            out.status.success() && out.stdout.is_empty(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        ),
        Err(_) => eprintln!("skipping: desktop-file-validate is not installed"),
    }
}
