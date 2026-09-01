//! Build script for voxtype
//!
//! Generates man pages from CLI definitions using clap_mangen.

use clap::CommandFactory;
use clap_mangen::Man;
use std::env;
use std::fs::{self, File};
use std::io::Error;
use std::path::PathBuf;

// Include the CLI module tree. The CLI lives under src/cli/, with mod.rs as
// its entry; we attach it to the build-script's own crate via #[path] so
// `Cli::command()` below resolves through the normal module system.
//
// The build script only needs `Cli` for man-page generation, but the module
// re-exports several subcommand enums and impl methods that go with it. They
// are intentionally unused here; suppress dead-code warnings so the build
// script does not pollute `cargo clippy --all-targets -- -D warnings`.
#[path = "src/cli/mod.rs"]
#[allow(dead_code, unused_imports)]
mod cli;
use cli::Cli;

fn main() -> Result<(), Error> {
    // Before the man-page early return: the version must be stamped for
    // every build, not just release builds.
    expose_build_version();

    // Only generate man pages for release builds or when explicitly requested
    let profile = env::var("PROFILE").unwrap_or_default();
    let generate = env::var("VOXTYPE_GEN_MANPAGES").is_ok() || profile == "release";

    if !generate {
        return Ok(());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap_or_else(|_| "target".to_string()));
    let man_dir = out_dir.join("man");
    fs::create_dir_all(&man_dir)?;

    let cmd = Cli::command();

    // Generate main man page (voxtype.1)
    let man = Man::new(cmd.clone());
    let mut file = File::create(man_dir.join("voxtype.1"))?;
    man.render(&mut file)?;

    // Generate man pages for subcommands
    for subcommand in cmd.get_subcommands() {
        let name = subcommand.get_name();
        if name == "help" {
            continue;
        }

        let man = Man::new(subcommand.clone());
        let mut file = File::create(man_dir.join(format!("voxtype-{}.1", name)))?;
        man.render(&mut file)?;

        // Generate man pages for nested subcommands (e.g., voxtype-setup-gpu)
        for nested in subcommand.get_subcommands() {
            let nested_name = nested.get_name();
            if nested_name == "help" {
                continue;
            }

            let man = Man::new(nested.clone());
            let mut file =
                File::create(man_dir.join(format!("voxtype-{}-{}.1", name, nested_name)))?;
            man.render(&mut file)?;
        }
    }

    // Tell cargo to rerun if CLI definitions change
    println!("cargo:rerun-if-changed=src/cli");

    // Print location of generated man pages
    println!(
        "cargo:warning=Man pages generated in: {}",
        man_dir.display()
    );

    expose_cuda_build_major();

    Ok(())
}

/// Make untagged builds distinguishable in `--version` output.
///
/// Cargo.toml is bumped to the release version before the tag exists, so
/// every CI or local build between the bump and the tag self-reports as the
/// release. On 2026-08-31 a stale binary honestly claiming "1.0.1" was
/// missing a day of fixes, and nothing it printed could tell it apart from
/// the real release build. Stamp the short commit hash into the version
/// unless HEAD sits exactly on the matching release tag: `--version` prints
/// `1.1.0` only for a build of tag v1.1.0 and `1.1.0+g<sha>` for everything
/// else.
///
/// Builds without git (release tarballs — the AUR source package builds from
/// one, so there is no .git dir) get no env var; `cli::VERSION` then falls
/// back to the bare crate version. Git trouble must never fail the build and
/// never leak a placeholder like "+gunknown" into the version string.
fn expose_build_version() {
    // Re-run when HEAD moves (commit, checkout). Creating a tag without a
    // new commit does not retrigger by itself; release binaries are always
    // clean `--no-cache` Docker builds, so best-effort staleness is fine.
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        if std::path::Path::new(&head).exists() {
            println!("cargo:rerun-if-changed={head}");
        }
    }

    let Some(sha) = git(&["rev-parse", "--short", "HEAD"]) else {
        return;
    };

    let pkg = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let at_release_tag = git(&["tag", "--points-at", "HEAD"])
        .is_some_and(|tags| tags.lines().any(|t| t == format!("v{pkg}")));

    let version = if at_release_tag {
        pkg
    } else {
        format!("{pkg}+g{sha}")
    };
    println!("cargo:rustc-env=VOXTYPE_BUILD_VERSION={version}");
}

/// Run git, returning trimmed stdout only on success with non-empty output.
fn git(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Mirror ort-sys's build-time CUDA version selection so the binary's runtime
/// probe can reject mismatched hosts before ort attempts (and crashes on)
/// EP registration. ort 2.0.0-rc.12 picks cu12 vs cu13 prebuilt at compile time
/// based on the same env var; we read it here and emit a compile-time constant
/// the parakeet code path uses to short-circuit graceful fallback.
fn expose_cuda_build_major() {
    println!("cargo:rerun-if-env-changed=ORT_CUDA_VERSION");
    let major = match env::var("ORT_CUDA_VERSION").as_deref() {
        Ok("12") => "12",
        Ok("13") => "13",
        // ort-sys defaults to cu12 when unset (see resolve.rs in ort-sys 2.0.0-rc.12).
        // Match that default so a debug build without ORT_CUDA_VERSION set agrees
        // with the bundled prebuilt.
        _ => "12",
    };
    println!("cargo:rustc-env=VOXTYPE_BUILD_CUDA_MAJOR={major}");
}
