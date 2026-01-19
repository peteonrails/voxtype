//! GPU backend management for voxtype
//!
//! Supports two installation modes:
//! 1. Tiered mode (DEB/RPM pre-built): Multiple CPU binaries (avx2, avx512) + vulkan in /usr/lib/voxtype/
//! 2. Simple mode (AUR source build): Native CPU binary at /usr/bin/voxtype + vulkan in /usr/lib/voxtype/

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

const VOXTYPE_LIB_DIR: &str = "/usr/lib/voxtype";
const VOXTYPE_BIN: &str = "/usr/bin/voxtype";
const VOXTYPE_CPU_BACKUP: &str = "/usr/lib/voxtype/voxtype-cpu";

/// Available backend variants
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Backend {
    Cpu,    // Simple mode: native CPU binary
    Avx2,   // Tiered mode: AVX2 binary
    Avx512, // Tiered mode: AVX-512 binary
    Vulkan, // GPU acceleration
}

impl Backend {
    fn binary_name(&self) -> &'static str {
        match self {
            Backend::Cpu => "voxtype-cpu",
            Backend::Avx2 => "voxtype-avx2",
            Backend::Avx512 => "voxtype-avx512",
            Backend::Vulkan => "voxtype-vulkan",
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            Backend::Cpu => "CPU (native)",
            Backend::Avx2 => "CPU (AVX2)",
            Backend::Avx512 => "CPU (AVX-512)",
            Backend::Vulkan => "GPU (Vulkan)",
        }
    }
}

/// Detect if we're in tiered mode (pre-built packages) or simple mode (source build)
fn is_tiered_mode() -> bool {
    Path::new(VOXTYPE_LIB_DIR).join("voxtype-avx2").exists()
}

/// Detect which backend is currently active
pub fn detect_current_backend() -> Option<Backend> {
    // Check if /usr/bin/voxtype is a symlink
    if let Ok(link_target) = fs::read_link(VOXTYPE_BIN) {
        let target_name = link_target.file_name()?.to_str()?;
        return match target_name {
            "voxtype-cpu" => Some(Backend::Cpu),
            "voxtype-avx2" => Some(Backend::Avx2),
            "voxtype-avx512" => Some(Backend::Avx512),
            "voxtype-vulkan" => Some(Backend::Vulkan),
            _ => None,
        };
    }

    // Not a symlink - check if it's a regular file (simple mode with CPU active)
    if Path::new(VOXTYPE_BIN).is_file() {
        return Some(Backend::Cpu);
    }

    None
}

/// Detect available backends (installed binaries)
pub fn detect_available_backends() -> Vec<Backend> {
    let mut available = Vec::new();

    if is_tiered_mode() {
        // Tiered mode: check for avx2, avx512, vulkan
        for backend in [Backend::Avx2, Backend::Avx512, Backend::Vulkan] {
            let path = Path::new(VOXTYPE_LIB_DIR).join(backend.binary_name());
            if path.exists() {
                available.push(backend);
            }
        }
    } else {
        // Simple mode: CPU binary at /usr/bin/voxtype or backed up
        if Path::new(VOXTYPE_BIN).is_file()
            && !fs::symlink_metadata(VOXTYPE_BIN)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        {
            available.push(Backend::Cpu);
        } else if Path::new(VOXTYPE_CPU_BACKUP).exists() {
            available.push(Backend::Cpu);
        }

        // Check for vulkan
        if Path::new(VOXTYPE_LIB_DIR).join("voxtype-vulkan").exists() {
            available.push(Backend::Vulkan);
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

/// Switch to a different backend (tiered mode only)
fn switch_backend_tiered(backend: Backend) -> anyhow::Result<()> {
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
    let _ = Command::new("restorecon").arg(VOXTYPE_BIN).status();

    Ok(())
}

/// Enable GPU in simple mode (backup CPU binary, symlink to vulkan)
fn enable_simple_mode() -> anyhow::Result<()> {
    let vulkan_path = Path::new(VOXTYPE_LIB_DIR).join("voxtype-vulkan");

    if !vulkan_path.exists() {
        anyhow::bail!(
            "Vulkan backend not installed.\n\
             The voxtype-vulkan binary was not found in {}",
            VOXTYPE_LIB_DIR
        );
    }

    // Check if already using vulkan
    if fs::read_link(VOXTYPE_BIN).is_ok() {
        anyhow::bail!("GPU backend is already enabled.");
    }

    // Ensure lib dir exists
    fs::create_dir_all(VOXTYPE_LIB_DIR)
        .map_err(|e| anyhow::anyhow!("Failed to create {}: {}", VOXTYPE_LIB_DIR, e))?;

    // Backup the CPU binary
    if Path::new(VOXTYPE_BIN).exists() {
        fs::rename(VOXTYPE_BIN, VOXTYPE_CPU_BACKUP).map_err(|e| {
            anyhow::anyhow!(
                "Failed to backup CPU binary (need sudo?): {}\n\
                 Try: sudo voxtype setup gpu --enable",
                e
            )
        })?;
    }

    // Create symlink to vulkan
    symlink(&vulkan_path, VOXTYPE_BIN).map_err(|e| {
        // Try to restore backup on failure
        let _ = fs::rename(VOXTYPE_CPU_BACKUP, VOXTYPE_BIN);
        anyhow::anyhow!(
            "Failed to create symlink (need sudo?): {}\n\
             Try: sudo voxtype setup gpu --enable",
            e
        )
    })?;

    // Restore SELinux context if available
    let _ = Command::new("restorecon").arg(VOXTYPE_BIN).status();

    Ok(())
}

/// Disable GPU in simple mode (restore CPU binary)
fn disable_simple_mode() -> anyhow::Result<()> {
    // Check if CPU backup exists
    if !Path::new(VOXTYPE_CPU_BACKUP).exists() {
        anyhow::bail!(
            "CPU binary backup not found at {}\n\
             Cannot restore CPU backend.",
            VOXTYPE_CPU_BACKUP
        );
    }

    // Remove vulkan symlink
    if fs::symlink_metadata(VOXTYPE_BIN).is_ok() {
        fs::remove_file(VOXTYPE_BIN).map_err(|e| {
            anyhow::anyhow!(
                "Failed to remove symlink (need sudo?): {}\n\
                 Try: sudo voxtype setup gpu --disable",
                e
            )
        })?;
    }

    // Restore CPU binary
    fs::rename(VOXTYPE_CPU_BACKUP, VOXTYPE_BIN).map_err(|e| {
        anyhow::anyhow!(
            "Failed to restore CPU binary (need sudo?): {}\n\
             Try: sudo voxtype setup gpu --disable",
            e
        )
    })?;

    // Restore SELinux context if available
    let _ = Command::new("restorecon").arg(VOXTYPE_BIN).status();

    Ok(())
}

/// Show current GPU/backend status
pub fn show_status() {
    println!("=== Voxtype Backend Status ===\n");

    let tiered = is_tiered_mode();

    // Current backend
    match detect_current_backend() {
        Some(backend) => {
            println!("Active backend: {}", backend.display_name());
            if backend == Backend::Vulkan || (tiered && backend != Backend::Cpu) {
                println!(
                    "  Binary: {}",
                    Path::new(VOXTYPE_LIB_DIR)
                        .join(backend.binary_name())
                        .display()
                );
            } else {
                println!("  Binary: {}", VOXTYPE_BIN);
            }
        }
        None => {
            println!("Active backend: Unknown (symlink may be broken)");
        }
    }

    // Installation mode
    println!(
        "\nInstallation mode: {}",
        if tiered {
            "tiered (pre-built)"
        } else {
            "simple (source build)"
        }
    );

    // Available backends
    println!("\nAvailable backends:");
    let available = detect_available_backends();
    let current = detect_current_backend();

    if tiered {
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
    } else {
        for backend in [Backend::Cpu, Backend::Vulkan] {
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
    // Check Vulkan binary exists
    let vulkan_path = Path::new(VOXTYPE_LIB_DIR).join("voxtype-vulkan");
    if !vulkan_path.exists() {
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

    if is_tiered_mode() {
        switch_backend_tiered(Backend::Vulkan)?;
    } else {
        enable_simple_mode()?;
    }

    // Regenerate systemd service if it exists
    if super::systemd::regenerate_service_file()? {
        println!("Updated systemd service to use GPU backend.");
    }

    println!("Switched to GPU (Vulkan) backend.");
    println!();
    println!("Restart voxtype to use GPU acceleration:");
    println!("  systemctl --user restart voxtype");

    Ok(())
}

/// Disable GPU backend (switch back to best CPU backend)
pub fn disable() -> anyhow::Result<()> {
    if is_tiered_mode() {
        // Detect best CPU backend
        let best_cpu = detect_best_cpu_backend();
        switch_backend_tiered(best_cpu)?;
        println!("Switched to {} backend.", best_cpu.display_name());
    } else {
        disable_simple_mode()?;
        println!("Switched to CPU (native) backend.");
    }

    // Regenerate systemd service if it exists
    if super::systemd::regenerate_service_file()? {
        println!("Updated systemd service to use CPU backend.");
    }

    println!();
    println!("Restart voxtype to use CPU inference:");
    println!("  systemctl --user restart voxtype");

    Ok(())
}

/// Detect the best CPU backend for this system (tiered mode)
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
