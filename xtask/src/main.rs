//! Development tasks for voxtype
//!
//! Usage:
//!   cargo xtask install [--vulkan]  Install release binary to /usr/local/bin (requires sudo)
//!   cargo xtask uninstall           Remove binary from /usr/local/bin (requires sudo)
//!   cargo xtask dist [--vulkan]     Build release binary for distribution

use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        print_help();
        return ExitCode::SUCCESS;
    }

    // Check for --vulkan flag
    let vulkan = args.iter().any(|a| a == "--vulkan" || a == "--gpu");

    let result = match args[0].as_str() {
        "install" => install(vulkan),
        "uninstall" => uninstall(),
        "dist" => dist(vulkan),
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
  install    Build release binary and install to /usr/local/bin (requires sudo)
  uninstall  Remove voxtype from /usr/local/bin (requires sudo)
  dist       Build optimized release binary for distribution

Options:
  --vulkan   Build with Vulkan GPU acceleration (alias: --gpu)

Examples:
  cargo xtask install            # Build CPU-only and install
  cargo xtask install --vulkan   # Build with Vulkan GPU support and install
  cargo xtask dist --vulkan      # Build Vulkan binary for distribution
  cargo xtask uninstall          # Remove installed binary
"#
    );
}

/// Get the project root directory
fn project_root() -> PathBuf {
    let dir = env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().unwrap());

    // xtask is in a subdirectory, go up one level
    dir.parent().unwrap_or(&dir).to_path_buf()
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
