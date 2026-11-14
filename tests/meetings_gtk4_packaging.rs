//! The GTK4 meetings window (`voxtype-meetings-gtk4`) reaches every release
//! channel that ships the GTK4 OSD: built once per architecture beside the
//! OSD frontends, uploaded, validated, packaged (release deb/rpm, source RPM,
//! Debian source package) and offered by the flake.
//! `voxtype meeting view` falls back to it on desktops without Quickshell, so
//! a channel that drops it leaves those users with no meeting view.

use std::path::PathBuf;

fn read(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn every_release_channel_carries_the_meetings_window() {
    let checks: &[(&str, &[&str])] = &[
        // The image that builds the OSD frontends builds the window too.
        (
            "Dockerfile.onnx",
            &[
                "--features osd-gtk4,meetings-gtk4",
                "--bin voxtype-meetings-gtk4",
                "/output/voxtype-${VERSION}-linux-x86_64-meetings-gtk4",
                "linux-x86_64-meetings-gtk4.version-proof.json",
            ],
        ),
        (
            "Dockerfile.aarch64-onnx",
            &[
                "--bin voxtype-meetings-gtk4",
                "/output/voxtype-${VERSION}-linux-aarch64-meetings-gtk4",
            ],
        ),
        (
            ".github/workflows/build-linux.yml",
            &["-linux-${{ matrix.arch }}-meetings-gtk4*"],
        ),
        (
            "scripts/validate-release.sh",
            &["voxtype-${VERSION}-linux-x86_64-meetings-gtk4"],
        ),
        ("scripts/release-artifacts.py", &["\"meetings-gtk4\""]),
        (
            "scripts/package.sh",
            &["install_companion_binary meetings-gtk4 \"$STAGING/usr/lib/voxtype/voxtype-meetings-gtk4\""],
        ),
        // The source packages ship it in a subpackage of its own, like the
        // GTK4 OSD, and recommend it from the main package.
        (
            "packaging/rpm/voxtype.spec",
            &[
                "--features osd-gtk4,meetings-gtk4",
                "--bin voxtype-meetings-gtk4",
                "%{buildroot}%{vt_libdir}/voxtype-meetings-gtk4",
                "%package meetings-gtk4",
                "Recommends:     %{name}-meetings-gtk4",
                "%files meetings-gtk4\n%{vt_libdir}/voxtype-meetings-gtk4",
            ],
        ),
        (
            "packaging/debian/rules",
            &[
                "--features osd-gtk4,meetings-gtk4",
                "--bin voxtype-meetings-gtk4",
                "debian/voxtype-meetings-gtk4/usr/lib/voxtype/voxtype-meetings-gtk4",
            ],
        ),
        (
            "packaging/debian/control",
            &[
                "Package: voxtype-meetings-gtk4",
                "voxtype-meetings-gtk4 (= ${binary:Version})",
            ],
        ),
        (
            "flake.nix",
            &[
                "buildFeatures = [ \"meetings-gtk4\" ];",
                "\"--bin\" \"voxtype-meetings-gtk4\"",
                "meetings-gtk4 = meetingsGtk4;",
            ],
        ),
    ];
    for (file, needles) in checks {
        let text = read(file);
        for needle in *needles {
            assert!(text.contains(needle), "{file} does not mention {needle}");
        }
    }
}
