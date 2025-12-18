//! GPU backend management for voxtype
//!
//! Switches between CPU (AVX2/AVX-512) and GPU (Vulkan) binaries.
//! The binaries are installed to /usr/lib/voxtype/ and a symlink
//! at /usr/bin/voxtype points to the active one.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

const VOXTYPE_LIB_DIR: &str = "/usr/lib/voxtype";
const VOXTYPE_BIN: &str = "/usr/bin/voxtype";

/// Available backend variants
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Backend {
    Avx2,
    Avx512,
    Vulkan,
}

impl Backend {
    fn binary_name(&self) -> &'static str {
        match self {
            Backend::Avx2 => "voxtype-avx2",
            Backend::Avx512 => "voxtype-avx512",
            Backend::Vulkan => "voxtype-vulkan",
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            Backend::Avx2 => "CPU (AVX2)",
            Backend::Avx512 => "CPU (AVX-512)",
            Backend::Vulkan => "GPU (Vulkan)",
        }
    }
}

/// Detect which backend is currently active by reading the symlink
pub fn detect_current_backend() -> Option<Backend> {
    let link_target = fs::read_link(VOXTYPE_BIN).ok()?;
    let target_name = link_target.file_name()?.to_str()?;

    match target_name {
        "voxtype-avx2" => Some(Backend::Avx2),
        "voxtype-avx512" => Some(Backend::Avx512),
        "voxtype-vulkan" => Some(Backend::Vulkan),
        _ => None,
    }
}

/// Detect available backends (installed binaries)
pub fn detect_available_backends() -> Vec<Backend> {
    let mut available = Vec::new();

    for backend in [Backend::Avx2, Backend::Avx512, Backend::Vulkan] {
        let path = Path::new(VOXTYPE_LIB_DIR).join(backend.binary_name());
        if path.exists() {
            available.push(backend);
        }
    }

    available
}

/// Detect if GPU is available for Vulkan
pub fn detect_gpu() -> Option<String> {
    // Check for DRI render nodes (indicates GPU with working driver)
    if !Path::new("/dev/dri").exists() {
        return None;
    }

    // Check for render nodes
    let render_nodes: Vec<_> = fs::read_dir("/dev/dri")
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| s.starts_with("renderD"))
                .unwrap_or(false)
        })
        .collect();

    if render_nodes.is_empty() {
        return None;
    }

    // Try to get GPU info via lspci
    if let Ok(output) = Command::new("lspci").output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let lower = line.to_lowercase();
            if lower.contains("vga") || lower.contains("3d") || lower.contains("display") {
                // Extract the GPU name (after the colon)
                if let Some(idx) = line.find(": ") {
                    return Some(line[idx + 2..].to_string());
                }
            }
        }
    }

    // Fallback
    Some("GPU detected (install pciutils for details)".to_string())
}

/// Check if Vulkan runtime is available
pub fn check_vulkan_runtime() -> bool {
    // Check for vulkan ICD loader
    let vulkan_paths = [
        "/usr/lib/libvulkan.so.1",
        "/usr/lib64/libvulkan.so.1",
        "/usr/lib/x86_64-linux-gnu/libvulkan.so.1",
    ];

    vulkan_paths.iter().any(|p| Path::new(p).exists())
}

/// Switch to a different backend
pub fn switch_backend(backend: Backend) -> anyhow::Result<()> {
    let binary_path = Path::new(VOXTYPE_LIB_DIR).join(backend.binary_name());

    if !binary_path.exists() {
        anyhow::bail!(
            "Backend binary not found: {}\n\
             This package may not include the {} backend.",
            binary_path.display(),
            backend.display_name()
        );
    }

    // Remove existing symlink
    if Path::new(VOXTYPE_BIN).exists() || fs::symlink_metadata(VOXTYPE_BIN).is_ok() {
        fs::remove_file(VOXTYPE_BIN).map_err(|e| {
            anyhow::anyhow!(
                "Failed to remove existing symlink (need sudo?): {}\n\
                 Try: sudo voxtype setup gpu --enable",
                e
            )
        })?;
    }

    // Create new symlink
    symlink(&binary_path, VOXTYPE_BIN).map_err(|e| {
        anyhow::anyhow!(
            "Failed to create symlink (need sudo?): {}\n\
             Try: sudo voxtype setup gpu --enable",
            e
        )
    })?;

    // Restore SELinux context if available
    let _ = Command::new("restorecon")
        .arg(VOXTYPE_BIN)
        .status();

    Ok(())
}

/// Show current GPU/backend status
pub fn show_status() {
    println!("=== Voxtype Backend Status ===\n");

    // Current backend
    match detect_current_backend() {
        Some(backend) => {
            println!("Active backend: {}", backend.display_name());
            println!("  Binary: {}", Path::new(VOXTYPE_LIB_DIR).join(backend.binary_name()).display());
        }
        None => {
            println!("Active backend: Unknown (symlink may be broken)");
        }
    }

    // Available backends
    println!("\nAvailable backends:");
    let available = detect_available_backends();
    let current = detect_current_backend();

    for backend in [Backend::Avx2, Backend::Avx512, Backend::Vulkan] {
        let installed = available.contains(&backend);
        let active = current == Some(backend);

        let status = if active {
            "active"
        } else if installed {
            "installed"
        } else {
            "not installed"
        };

        println!("  {} - {}", backend.display_name(), status);
    }

    // GPU detection
    println!();
    if let Some(gpu) = detect_gpu() {
        println!("GPU detected: {}", gpu);

        if check_vulkan_runtime() {
            println!("Vulkan runtime: installed");
        } else {
            println!("Vulkan runtime: NOT FOUND");
            println!("  Install vulkan-icd-loader for GPU acceleration");
        }
    } else {
        println!("GPU: not detected");
    }

    // Usage hints
    println!();
    if current != Some(Backend::Vulkan) && available.contains(&Backend::Vulkan) {
        println!("To enable GPU acceleration:");
        println!("  sudo voxtype setup gpu --enable");
    } else if current == Some(Backend::Vulkan) {
        println!("To switch back to CPU:");
        println!("  sudo voxtype setup gpu --disable");
    }
}

/// Enable GPU (Vulkan) backend
pub fn enable() -> anyhow::Result<()> {
    let available = detect_available_backends();

    if !available.contains(&Backend::Vulkan) {
        anyhow::bail!(
            "Vulkan backend not installed.\n\
             The voxtype-vulkan binary was not found in {}",
            VOXTYPE_LIB_DIR
        );
    }

    // Check Vulkan runtime
    if !check_vulkan_runtime() {
        println!("Warning: Vulkan runtime (libvulkan.so.1) not found.");
        println!("You may need to install vulkan-icd-loader:");
        println!("  Fedora: sudo dnf install vulkan-loader");
        println!("  Arch:   sudo pacman -S vulkan-icd-loader");
        println!("  Ubuntu: sudo apt install libvulkan1");
        println!();
    }

    switch_backend(Backend::Vulkan)?;

    println!("Switched to GPU (Vulkan) backend.");
    println!();
    println!("Restart voxtype to use GPU acceleration:");
    println!("  systemctl --user restart voxtype");

    Ok(())
}

/// Disable GPU backend (switch back to best CPU backend)
pub fn disable() -> anyhow::Result<()> {
    // Detect best CPU backend
    let best_cpu = detect_best_cpu_backend();

    switch_backend(best_cpu)?;

    println!("Switched to {} backend.", best_cpu.display_name());
    println!();
    println!("Restart voxtype to use CPU inference:");
    println!("  systemctl --user restart voxtype");

    Ok(())
}

/// Detect the best CPU backend for this system
fn detect_best_cpu_backend() -> Backend {
    // Check for AVX-512 support
    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
        if cpuinfo.contains("avx512f") {
            let avx512_path = Path::new(VOXTYPE_LIB_DIR).join("voxtype-avx512");
            if avx512_path.exists() {
                return Backend::Avx512;
            }
        }
    }

    Backend::Avx2
}
