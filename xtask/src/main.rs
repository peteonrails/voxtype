//! Development tasks for voxtype
//!
//! Usage:
//!   cargo xtask install [--vulkan]  Install release binary to /usr/local/bin (requires sudo)
//!   cargo xtask uninstall           Remove binary from /usr/local/bin (requires sudo)
//!   cargo xtask dist [--vulkan]     Build release binary for distribution
//!   cargo xtask dev [-- ARGS...]    Build and run the optimized Vulkan dev daemon
//!   cargo xtask man [DIR]           Render the man pages into DIR (default target/man)

use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

// The voxtype CLI definitions, attached the same way build.rs does it, so
// the man pages render from src/cli without compiling voxtype itself.
#[path = "../../src/cli/mod.rs"]
#[allow(dead_code, unused_imports)]
mod cli;
mod man;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        print_help();
        return ExitCode::SUCCESS;
    }

    let vulkan = args.iter().any(|a| a == "--vulkan" || a == "--gpu");

    let result = match args[0].as_str() {
        "dev" => dev(&args[1..]),
        "install" => install(vulkan),
        "uninstall" => uninstall(),
        "dist" => dist(vulkan),
        "man" => man_pages(args.get(1).map(String::as_str)),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        cmd => {
            eprintln!("Unknown command: {}", cmd);
            print_help();
            Err(anyhow::anyhow!("Unknown command"))
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    eprintln!(
        r#"
voxtype development tasks

Usage: cargo xtask <COMMAND> [OPTIONS]

Commands:
  dev        Build and run optimized dev binaries from target/release
  install    Build release binary and install to /usr/local/bin (requires sudo)
  uninstall  Remove voxtype from /usr/local/bin (requires sudo)
  dist       Build optimized release binary for distribution
  man [DIR]  Render man pages into DIR (default: target/man); used by
             scripts/package.sh, needs no voxtype build

Options:
  --vulkan   Build with Vulkan GPU acceleration (alias: --gpu)
  --cpu      For `dev`, build CPU-only instead of the default Vulkan build
  --no-stop-service
             For `dev`, do not stop systemd --user voxtype before launching
  -- ARGS    For `dev`, pass remaining args to target/release/voxtype

Examples:
  cargo xtask dev                # Build/run release+Vulkan dev daemon
  cargo xtask dev -- --verbose   # Pass args to voxtype
  cargo xtask dev --cpu          # CPU-only dev run for non-Vulkan systems
  cargo xtask install            # Build CPU-only and install
  cargo xtask install --vulkan   # Build with Vulkan GPU support and install
  cargo xtask dist --vulkan      # Build Vulkan binary for distribution
  cargo xtask uninstall          # Remove installed binary
  cargo xtask man /tmp/man1      # Render man pages into /tmp/man1
"#
    );
}

#[derive(Debug)]
struct DevOptions {
    vulkan: bool,
    stop_service: bool,
    voxtype_args: Vec<String>,
}

impl Default for DevOptions {
    fn default() -> Self {
        Self {
            vulkan: true,
            stop_service: true,
            voxtype_args: Vec::new(),
        }
    }
}

fn parse_dev_options(args: &[String]) -> anyhow::Result<DevOptions> {
    let mut opts = DevOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--" => {
                opts.voxtype_args.extend(args[i + 1..].iter().cloned());
                break;
            }
            "--vulkan" | "--gpu" => opts.vulkan = true,
            "--cpu" | "--no-vulkan" => opts.vulkan = false,
            "--no-stop-service" => opts.stop_service = false,
            other => anyhow::bail!("Unknown dev option: {other}"),
        }
        i += 1;
    }
    Ok(opts)
}

/// Render the man pages into `dir` (relative paths resolve against the
/// current directory; default `target/man` under the project root).
///
/// Compiled into xtask, the CLI's `author` and `version` attributes would
/// read xtask's own package metadata, so both come from voxtype's
/// Cargo.toml instead. That matches what build.rs renders.
fn man_pages(dir: Option<&str>) -> anyhow::Result<()> {
    use anyhow::Context;
    use clap::CommandFactory;

    let root = project_root();
    let out = match dir {
        Some(d) => env::current_dir()?.join(d),
        None => root.join("target/man"),
    };

    let manifest = std::fs::read_to_string(root.join("Cargo.toml"))?;
    let (version, authors) = package_metadata(&manifest)?;

    let cmd = cli::Cli::command()
        .version(version.leak() as &str)
        .author(authors.leak() as &str);
    let written = man::write_man_pages(&cmd, &out)
        .with_context(|| format!("writing man pages to {}", out.display()))?;
    println!("Wrote {written} man pages to {}", out.display());
    Ok(())
}

/// voxtype's version and authors, joined the way `clap::crate_authors!` does.
fn package_metadata(manifest: &str) -> anyhow::Result<(String, String)> {
    let doc: toml::Table = manifest.parse()?;
    let package = doc
        .get("package")
        .and_then(|p| p.as_table())
        .ok_or_else(|| anyhow::anyhow!("Cargo.toml has no [package] table"))?;
    let version = package
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Cargo.toml has no package.version"))?
        .to_string();
    let authors = package
        .get("authors")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    Ok((version, authors))
}

/// Get the project root directory
fn project_root() -> PathBuf {
    let dir = env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().unwrap());

    // xtask is in a subdirectory, go up one level
    dir.parent().unwrap_or(&dir).to_path_buf()
}

/// Build optimized dev binaries and run the daemon from target/release.
///
/// This intentionally does not use `cargo run`: the daemon spawns sidecars
/// such as `voxtype-osd`, and those sidecars need to come from the same build
/// directory as the daemon to avoid installed/dev binary mismatches.
fn dev(args: &[String]) -> anyhow::Result<()> {
    let opts = parse_dev_options(args)?;
    let root = project_root();
    let release_dir = root.join("target/release");

    if opts.vulkan {
        println!("==> Building optimized dev binaries with Vulkan GPU support...");
    } else {
        println!("==> Building optimized CPU-only dev binaries...");
    }

    let mut build_args = vec![
        "build",
        "--release",
        "--bin",
        "voxtype",
        "--bin",
        "voxtype-osd",
        "--bin",
        "voxtype-osd-quickshell",
        "--bin",
        "voxtype-audio-bridge",
    ];
    if opts.vulkan {
        build_args.push("--features");
        build_args.push("gpu-vulkan");
    }

    let status = Command::new("cargo")
        .args(&build_args)
        .current_dir(&root)
        .status()?;
    if !status.success() {
        anyhow::bail!("Build failed");
    }

    let voxtype = release_dir.join("voxtype");
    if !voxtype.is_file() {
        anyhow::bail!("Binary not found at {}", voxtype.display());
    }

    if opts.stop_service {
        println!("==> Stopping systemd user service to avoid the single-instance lock...");
        let status = Command::new("systemctl")
            .args(["--user", "stop", "voxtype"])
            .stdin(Stdio::null())
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => eprintln!("warning: systemctl --user stop voxtype exited with {s}"),
            Err(e) => eprintln!("warning: failed to stop systemd user service: {e}"),
        }
    }

    let path = prepend_path(&release_dir)?;
    let qml_path = root.join("quickshell");
    let bridge = release_dir.join("voxtype-audio-bridge");
    let style_file = default_style_file();

    println!("==> Running {}", voxtype.display());
    if opts.vulkan {
        println!("    feature: gpu-vulkan");
    }
    println!("    PATH starts with {}", release_dir.display());
    println!("    VOXTYPE_OSD_QML_PATH={}", qml_path.display());
    println!("    VOXTYPE_AUDIO_BRIDGE_BINARY={}", bridge.display());
    println!("    VOXTYPE_OSD_STYLE_FILE={}", style_file.display());
    println!();

    let status = Command::new(&voxtype)
        .args(&opts.voxtype_args)
        .current_dir(&root)
        .env("PATH", path)
        .env("VOXTYPE_OSD_QML_PATH", qml_path)
        .env("VOXTYPE_AUDIO_BRIDGE_BINARY", bridge)
        .env("VOXTYPE_OSD_STYLE_FILE", style_file)
        .status()?;

    if !status.success() {
        anyhow::bail!("Dev daemon exited with {status}");
    }
    Ok(())
}

fn prepend_path(dir: &PathBuf) -> anyhow::Result<String> {
    let old = env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![dir.clone()];
    paths.extend(env::split_paths(&old));
    Ok(env::join_paths(paths)?.to_string_lossy().into_owned())
}

fn default_style_file() -> PathBuf {
    if let Ok(path) = env::var("VOXTYPE_OSD_STYLE_FILE") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let runtime = env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    runtime.join("voxtype/quickshell-style.json")
}

/// Build release binary and install to /usr/local/bin
fn install(vulkan: bool) -> anyhow::Result<()> {
    let root = project_root();

    if vulkan {
        println!("==> Building release binary with Vulkan GPU support...");
    } else {
        println!("==> Building release binary...");
    }

    let mut args = vec!["build", "--release"];
    if vulkan {
        args.push("--features");
        args.push("gpu-vulkan");
    }

    let status = Command::new("cargo")
        .args(&args)
        .current_dir(&root)
        .status()?;

    if !status.success() {
        anyhow::bail!("Build failed");
    }

    let binary = root.join("target/release/voxtype");
    if !binary.exists() {
        anyhow::bail!("Binary not found at {:?}", binary);
    }

    println!("==> Installing to /usr/local/bin/voxtype...");

    let status = Command::new("sudo")
        .args([
            "install",
            "-Dm755",
            binary.to_str().unwrap(),
            "/usr/local/bin/voxtype",
        ])
        .status()?;

    if !status.success() {
        anyhow::bail!("Install failed (sudo required)");
    }

    println!("==> Installed successfully!");
    if vulkan {
        println!("    (with Vulkan GPU acceleration)");
    }
    println!();
    println!("Installed: /usr/local/bin/voxtype");

    // Show version
    let _ = Command::new("/usr/local/bin/voxtype")
        .arg("--version")
        .status();

    Ok(())
}

/// Remove voxtype from /usr/local/bin
fn uninstall() -> anyhow::Result<()> {
    println!("==> Removing /usr/local/bin/voxtype...");

    let status = Command::new("sudo")
        .args(["rm", "-f", "/usr/local/bin/voxtype"])
        .status()?;

    if !status.success() {
        anyhow::bail!("Uninstall failed (sudo required)");
    }

    println!("==> Uninstalled successfully!");
    Ok(())
}

/// Build optimized release binary for distribution
fn dist(vulkan: bool) -> anyhow::Result<()> {
    let root = project_root();

    if vulkan {
        println!("==> Building distribution binary with Vulkan GPU support...");
    } else {
        println!("==> Building distribution binary...");
    }

    let mut args = vec!["build", "--release"];
    if vulkan {
        args.push("--features");
        args.push("gpu-vulkan");
    }

    let status = Command::new("cargo")
        .args(&args)
        .current_dir(&root)
        .status()?;

    if !status.success() {
        anyhow::bail!("Build failed");
    }

    let binary = root.join("target/release/voxtype");
    println!("==> Built: {:?}", binary);
    if vulkan {
        println!("    (with Vulkan GPU acceleration)");
    }

    // Show binary info
    let _ = Command::new("ls")
        .args(["-lh", binary.to_str().unwrap()])
        .status();

    let _ = Command::new(binary.to_str().unwrap())
        .arg("--version")
        .status();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_metadata_reads_the_package_table() {
        let manifest = r#"
[workspace]
members = ["xtask"]

[package]
name = "voxtype"
version = "9.8.7"
authors = ["Ada", "Grace Hopper"]

[dependencies]
version = "not-this-one"
"#;
        let (version, authors) = package_metadata(manifest).unwrap();
        assert_eq!(version, "9.8.7");
        assert_eq!(authors, "Ada, Grace Hopper");
    }

    #[test]
    fn package_metadata_reads_the_real_manifest() {
        let manifest = std::fs::read_to_string(project_root().join("Cargo.toml")).unwrap();
        let (version, authors) = package_metadata(&manifest).unwrap();
        assert!(!version.is_empty());
        assert!(authors.starts_with("Peter Jackson"));
    }
}
