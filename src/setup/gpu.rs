//! GPU backend management for voxtype
//!
//! Supports two installation modes:
//! 1. Tiered mode (DEB/RPM pre-built): Multiple CPU binaries (avx2, avx512) + vulkan in /usr/lib/voxtype/
//! 2. Simple mode (AUR source build): Native CPU binary at /usr/bin/voxtype + vulkan in /usr/lib/voxtype/
//!
//! Engine-aware: In Parakeet mode, switches between parakeet-cuda and parakeet-avx*.
//! In Whisper mode, switches between vulkan and avx*.
//!
//! GPU Selection:
//! On systems with multiple GPUs (e.g., Intel integrated + NVIDIA discrete), the Vulkan
//! backend may select the wrong GPU by default. Use VOXTYPE_VULKAN_DEVICE environment
//! variable to select a specific GPU:
//!   - VOXTYPE_VULKAN_DEVICE=nvidia  (selects NVIDIA GPU)
//!   - VOXTYPE_VULKAN_DEVICE=amd     (selects AMD GPU)
//!   - VOXTYPE_VULKAN_DEVICE=intel   (selects Intel GPU)
//!
//! This sets VK_LOADER_DRIVERS_SELECT internally to filter Vulkan ICDs.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

const VOXTYPE_LIB_DIR: &str = "/usr/lib/voxtype";
const VOXTYPE_BIN: &str = "/usr/bin/voxtype";
const VOXTYPE_BIN_LOCAL: &str = "/usr/local/bin/voxtype";
const VOXTYPE_CPU_BACKUP: &str = "/usr/lib/voxtype/voxtype-cpu";
const VOXTYPE_NATIVE: &str = "/usr/lib/voxtype/voxtype-native";

/// Get the active voxtype binary path (prefers /usr/bin, falls back to /usr/local/bin)
fn get_active_binary_path() -> &'static str {
    // If /usr/bin/voxtype exists and points somewhere, use it
    if Path::new(VOXTYPE_BIN).exists() {
        return VOXTYPE_BIN;
    }
    // Fall back to /usr/local/bin/voxtype
    if Path::new(VOXTYPE_BIN_LOCAL).exists() {
        return VOXTYPE_BIN_LOCAL;
    }
    // Default to standard location
    VOXTYPE_BIN
}

/// Check if the current symlink points to a Parakeet binary
/// Follows symlink chains to find the final target
fn is_parakeet_binary_active() -> bool {
    let active_bin = get_active_binary_path();
    // Use canonicalize to resolve all symlinks and get the final target
    if let Ok(resolved) = fs::canonicalize(active_bin) {
        if let Some(target_name) = resolved.file_name() {
            if let Some(name) = target_name.to_str() {
                return name.contains("parakeet");
            }
        }
    }
    false
}

/// Get the name of the active Parakeet backend binary
fn detect_active_parakeet_backend() -> Option<String> {
    let active_bin = get_active_binary_path();
    // Use canonicalize to resolve all symlinks and get the final target
    if let Ok(resolved) = fs::canonicalize(active_bin) {
        if let Some(target_name) = resolved.file_name() {
            if let Some(name) = target_name.to_str() {
                if name.contains("parakeet") {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Available backend variants
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Backend {
    Cpu,    // Legacy: voxtype-cpu (deprecated, kept for compatibility)
    Native, // Simple mode: source-built native CPU binary (voxtype-native)
    Avx2,   // Tiered mode: AVX2 binary
    Avx512, // Tiered mode: AVX-512 binary
    Vulkan, // GPU acceleration
}

impl Backend {
    fn binary_name(&self) -> &'static str {
        match self {
            Backend::Cpu => "voxtype-cpu",
            Backend::Native => "voxtype-native",
            Backend::Avx2 => "voxtype-avx2",
            Backend::Avx512 => "voxtype-avx512",
            Backend::Vulkan => "voxtype-vulkan",
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            Backend::Cpu => "CPU (legacy)",
            Backend::Native => "CPU (native)",
            Backend::Avx2 => "CPU (AVX2)",
            Backend::Avx512 => "CPU (AVX-512)",
            Backend::Vulkan => "GPU (Vulkan)",
        }
    }
}

/// GPU vendor type for device selection
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

impl GpuVendor {
    /// Parse vendor from GPU name string
    fn from_name(name: &str) -> Self {
        let lower = name.to_lowercase();
        if lower.contains("nvidia")
            || lower.contains("geforce")
            || lower.contains("quadro")
            || lower.contains("rtx")
            || lower.contains("gtx")
        {
            GpuVendor::Nvidia
        } else if lower.contains("amd") || lower.contains("radeon") || lower.contains("rx ") {
            GpuVendor::Amd
        } else if lower.contains("intel") {
            GpuVendor::Intel
        } else {
            GpuVendor::Other
        }
    }

    /// Get the VK_LOADER_DRIVERS_SELECT glob pattern for this vendor
    fn vulkan_driver_glob(&self) -> &'static str {
        match self {
            GpuVendor::Nvidia => "nvidia*",
            GpuVendor::Amd => "*radeon*,*amd*",
            GpuVendor::Intel => "*intel*",
            GpuVendor::Other => "*",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            GpuVendor::Nvidia => "NVIDIA",
            GpuVendor::Amd => "AMD",
            GpuVendor::Intel => "Intel",
            GpuVendor::Other => "Other",
        }
    }
}

/// Information about a detected GPU
#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: GpuVendor,
    pub pci_slot: Option<String>,
}

/// Detect if we're in tiered mode (pre-built packages) or simple mode (source build)
fn is_tiered_mode() -> bool {
    Path::new(VOXTYPE_LIB_DIR).join("voxtype-avx2").exists()
}

/// Detect which backend is currently active
pub fn detect_current_backend() -> Option<Backend> {
    let active_bin = get_active_binary_path();
    // Check if the voxtype binary is a symlink
    if let Ok(link_target) = fs::read_link(active_bin) {
        let target_name = link_target.file_name()?.to_str()?;
        return match target_name {
            "voxtype-cpu" => Some(Backend::Cpu),
            "voxtype-native" => Some(Backend::Native),
            "voxtype-avx2" => Some(Backend::Avx2),
            "voxtype-avx512" => Some(Backend::Avx512),
            "voxtype-vulkan" => Some(Backend::Vulkan),
            _ => None,
        };
    }

    // Not a symlink - check if it's a regular file (simple mode with CPU active)
    if Path::new(active_bin).is_file() {
        return Some(Backend::Native);
    }

    None
}

/// Detect available backends (installed binaries)
pub fn detect_available_backends() -> Vec<Backend> {
    let mut available = Vec::new();
    let active_bin = get_active_binary_path();

    if is_tiered_mode() {
        // Tiered mode: check for avx2, avx512, vulkan
        for backend in [Backend::Avx2, Backend::Avx512, Backend::Vulkan] {
            let path = Path::new(VOXTYPE_LIB_DIR).join(backend.binary_name());
            if path.exists() {
                available.push(backend);
            }
        }
    } else {
        // Simple mode: check for native binary in lib dir or at active location
        if Path::new(VOXTYPE_NATIVE).exists() {
            available.push(Backend::Native);
        } else if Path::new(active_bin).is_file()
            && !fs::symlink_metadata(active_bin)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        {
            // Binary directly at active location (not a symlink)
            available.push(Backend::Native);
        } else if Path::new(VOXTYPE_CPU_BACKUP).exists() {
            // Legacy backup location
            available.push(Backend::Cpu);
        }

        // Check for vulkan
        if Path::new(VOXTYPE_LIB_DIR).join("voxtype-vulkan").exists() {
            available.push(Backend::Vulkan);
        }
    }

    available
}

/// Detect all available GPUs
pub fn detect_gpus() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();

    // Check for DRI render nodes (indicates GPU with working driver)
    if !Path::new("/dev/dri").exists() {
        return gpus;
    }

    // Check for render nodes
    let render_nodes: Vec<_> = fs::read_dir("/dev/dri")
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .map(|s| s.starts_with("renderD"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();

    if render_nodes.is_empty() {
        return gpus;
    }

    // Try to get GPU info via lspci
    if let Ok(output) = Command::new("lspci").output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let lower = line.to_lowercase();
            if lower.contains("vga") || lower.contains("3d") || lower.contains("display") {
                // Extract PCI slot (first field before space)
                let pci_slot = line.split_whitespace().next().map(String::from);

                // Extract the GPU name (after the colon)
                if let Some(idx) = line.find(": ") {
                    let name = line[idx + 2..].to_string();
                    let vendor = GpuVendor::from_name(&name);
                    gpus.push(GpuInfo {
                        name,
                        vendor,
                        pci_slot,
                    });
                }
            }
        }
    }

    // Fallback if lspci not available but render nodes exist
    if gpus.is_empty() && !render_nodes.is_empty() {
        gpus.push(GpuInfo {
            name: "GPU detected (install pciutils for details)".to_string(),
            vendor: GpuVendor::Other,
            pci_slot: None,
        });
    }

    gpus
}

/// Detect if GPU is available for Vulkan (returns first GPU for backward compatibility)
pub fn detect_gpu() -> Option<String> {
    detect_gpus().first().map(|g| g.name.clone())
}

/// Parse VOXTYPE_VULKAN_DEVICE environment variable and return the appropriate vendor
pub fn get_selected_gpu_vendor() -> Option<GpuVendor> {
    std::env::var("VOXTYPE_VULKAN_DEVICE")
        .ok()
        .and_then(|val| match val.to_lowercase().as_str() {
            "nvidia" | "nv" => Some(GpuVendor::Nvidia),
            "amd" | "radeon" => Some(GpuVendor::Amd),
            "intel" => Some(GpuVendor::Intel),
            _ => None,
        })
}

/// Apply GPU selection environment variables based on VOXTYPE_VULKAN_DEVICE
/// Call this before initializing Vulkan to ensure the correct GPU is selected.
/// Returns the vendor that was selected, if any.
pub fn apply_gpu_selection() -> Option<GpuVendor> {
    if let Some(vendor) = get_selected_gpu_vendor() {
        // Only set if not already set by user
        if std::env::var("VK_LOADER_DRIVERS_SELECT").is_err() {
            std::env::set_var("VK_LOADER_DRIVERS_SELECT", vendor.vulkan_driver_glob());
        }
        Some(vendor)
    } else {
        None
    }
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
    let active_bin = get_active_binary_path();

    if !binary_path.exists() {
        anyhow::bail!(
            "Backend binary not found: {}\n\
             This package may not include the {} backend.",
            binary_path.display(),
            backend.display_name()
        );
    }

    // Remove existing symlink
    if Path::new(active_bin).exists() || fs::symlink_metadata(active_bin).is_ok() {
        fs::remove_file(active_bin).map_err(|e| {
            anyhow::anyhow!(
                "Failed to remove existing symlink (need sudo?): {}\n\
                 Try: sudo voxtype setup gpu --enable",
                e
            )
        })?;
    }

    // Create new symlink
    symlink(&binary_path, active_bin).map_err(|e| {
        anyhow::anyhow!(
            "Failed to create symlink (need sudo?): {}\n\
             Try: sudo voxtype setup gpu --enable",
            e
        )
    })?;

    // Restore SELinux context if available
    let _ = Command::new("restorecon").arg(active_bin).status();

    Ok(())
}

/// Enable GPU in simple mode (switch symlink from native to vulkan)
fn enable_simple_mode() -> anyhow::Result<()> {
    let vulkan_path = Path::new(VOXTYPE_LIB_DIR).join("voxtype-vulkan");
    let native_path = Path::new(VOXTYPE_NATIVE);
    let active_bin = get_active_binary_path();

    if !vulkan_path.exists() {
        anyhow::bail!(
            "Vulkan backend not installed.\n\
             The voxtype-vulkan binary was not found in {}",
            VOXTYPE_LIB_DIR
        );
    }

    // Check if already using vulkan (symlink points to vulkan)
    if let Ok(target) = fs::read_link(active_bin) {
        if target.file_name().map(|n| n.to_str()) == Some(Some("voxtype-vulkan")) {
            anyhow::bail!("GPU backend is already enabled.");
        }
    }

    // Ensure lib dir exists
    fs::create_dir_all(VOXTYPE_LIB_DIR)
        .map_err(|e| anyhow::anyhow!("Failed to create {}: {}", VOXTYPE_LIB_DIR, e))?;

    // Handle different scenarios:
    // 1. New layout: symlink to voxtype-native -> just update symlink
    // 2. Old layout: actual binary at active_bin -> backup and symlink
    // 3. No native binary in lib dir -> backup current binary
    let is_symlink = fs::symlink_metadata(active_bin)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);

    if !is_symlink && Path::new(active_bin).exists() && !native_path.exists() {
        // Old layout: backup the CPU binary (only if native doesn't exist in lib dir)
        fs::rename(active_bin, VOXTYPE_CPU_BACKUP).map_err(|e| {
            anyhow::anyhow!(
                "Failed to backup CPU binary (need sudo?): {}\n\
                 Try: sudo voxtype setup gpu --enable",
                e
            )
        })?;
    } else if is_symlink || Path::new(active_bin).exists() {
        // New layout or existing symlink: just remove current symlink/file
        fs::remove_file(active_bin).map_err(|e| {
            anyhow::anyhow!(
                "Failed to remove existing binary/symlink (need sudo?): {}\n\
                 Try: sudo voxtype setup gpu --enable",
                e
            )
        })?;
    }

    // Create symlink to vulkan
    symlink(&vulkan_path, active_bin).map_err(|e| {
        // Try to restore on failure
        if native_path.exists() {
            let _ = symlink(native_path, active_bin);
        } else {
            let _ = fs::rename(VOXTYPE_CPU_BACKUP, active_bin);
        }
        anyhow::anyhow!(
            "Failed to create symlink (need sudo?): {}\n\
             Try: sudo voxtype setup gpu --enable",
            e
        )
    })?;

    // Restore SELinux context if available
    let _ = Command::new("restorecon").arg(active_bin).status();

    Ok(())
}

/// Disable GPU in simple mode (restore native CPU binary)
fn disable_simple_mode() -> anyhow::Result<()> {
    let active_bin = get_active_binary_path();
    let native_path = Path::new(VOXTYPE_NATIVE);

    // Check if native binary exists in lib dir (new layout) or backup exists (old layout)
    let use_native_layout = native_path.exists();
    let use_backup_layout = Path::new(VOXTYPE_CPU_BACKUP).exists();

    if !use_native_layout && !use_backup_layout {
        anyhow::bail!(
            "CPU binary not found.\n\
             Neither {} nor {} exists.\n\
             Cannot restore CPU backend.",
            VOXTYPE_NATIVE,
            VOXTYPE_CPU_BACKUP
        );
    }

    // Remove vulkan symlink
    if fs::symlink_metadata(active_bin).is_ok() {
        fs::remove_file(active_bin).map_err(|e| {
            anyhow::anyhow!(
                "Failed to remove symlink (need sudo?): {}\n\
                 Try: sudo voxtype setup gpu --disable",
                e
            )
        })?;
    }

    if use_native_layout {
        // New layout: create symlink to voxtype-native
        symlink(native_path, active_bin).map_err(|e| {
            anyhow::anyhow!(
                "Failed to create symlink (need sudo?): {}\n\
                 Try: sudo voxtype setup gpu --disable",
                e
            )
        })?;
    } else {
        // Old layout: restore from backup
        fs::rename(VOXTYPE_CPU_BACKUP, active_bin).map_err(|e| {
            anyhow::anyhow!(
                "Failed to restore CPU binary (need sudo?): {}\n\
                 Try: sudo voxtype setup gpu --disable",
                e
            )
        })?;
    }

    // Restore SELinux context if available
    let _ = Command::new("restorecon").arg(active_bin).status();

    Ok(())
}

/// Show current GPU/backend status
pub fn show_status() {
    println!("=== Voxtype Backend Status ===\n");

    let tiered = is_tiered_mode();
    let active_bin = get_active_binary_path();
    let is_parakeet = is_parakeet_binary_active();

    // Current backend
    if is_parakeet {
        // Detect active Parakeet backend from symlink
        if let Some(target) = detect_active_parakeet_backend() {
            let display_name = match target.as_str() {
                "voxtype-parakeet-avx2" => "Parakeet CPU (AVX2)",
                "voxtype-parakeet-avx512" => "Parakeet CPU (AVX-512)",
                "voxtype-parakeet-cuda" => "Parakeet GPU (CUDA)",
                "voxtype-parakeet-rocm" => "Parakeet GPU (ROCm)",
                _ => "Parakeet (unknown variant)",
            };
            println!("Active backend: {}", display_name);
            println!(
                "  Binary: {}",
                Path::new(VOXTYPE_LIB_DIR).join(&target).display()
            );
        } else {
            println!("Active backend: Parakeet (unknown variant)");
        }
    } else {
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
                    println!("  Binary: {}", active_bin);
                }
            }
            None => {
                println!("Active backend: Unknown (symlink may be broken)");
            }
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

    if is_parakeet {
        // Show Parakeet backends
        let parakeet_backends = [
            ("voxtype-parakeet-avx2", "Parakeet CPU (AVX2)"),
            ("voxtype-parakeet-avx512", "Parakeet CPU (AVX-512)"),
            ("voxtype-parakeet-cuda", "Parakeet GPU (CUDA)"),
            ("voxtype-parakeet-rocm", "Parakeet GPU (ROCm)"),
        ];

        // Get current symlink target
        let current_target = fs::read_link(active_bin)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));

        for (binary, display) in parakeet_backends {
            let path = Path::new(VOXTYPE_LIB_DIR).join(binary);
            let installed = path.exists();
            let active = current_target.as_deref() == Some(binary);

            let status = if active {
                "active"
            } else if installed {
                "installed"
            } else {
                "not installed"
            };

            println!("  {} - {}", display, status);
        }
    } else if tiered {
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
        for backend in [Backend::Native, Backend::Vulkan] {
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
    let gpus = detect_gpus();
    if gpus.is_empty() {
        println!("GPU: not detected");
    } else {
        println!("GPUs detected:");
        for (i, gpu) in gpus.iter().enumerate() {
            println!("  {}. [{}] {}", i + 1, gpu.vendor.display_name(), gpu.name);
        }

        // Show Vulkan runtime status
        println!();
        if check_vulkan_runtime() {
            println!("Vulkan runtime: installed");
        } else {
            println!("Vulkan runtime: NOT FOUND");
            println!("  Install vulkan-icd-loader for GPU acceleration");
        }

        // Show GPU selection status if multiple GPUs
        if gpus.len() > 1 {
            println!();
            if let Some(selected) = get_selected_gpu_vendor() {
                println!(
                    "GPU selection: {} (via VOXTYPE_VULKAN_DEVICE)",
                    selected.display_name()
                );
            } else {
                println!("GPU selection: auto (first available)");
                println!();
                println!("Multiple GPUs detected. To select a specific GPU, set:");
                println!("  VOXTYPE_VULKAN_DEVICE=nvidia   # Use NVIDIA GPU");
                println!("  VOXTYPE_VULKAN_DEVICE=amd      # Use AMD GPU");
                println!("  VOXTYPE_VULKAN_DEVICE=intel    # Use Intel GPU");
                println!();
                println!("For systemd, create ~/.config/systemd/user/voxtype.service.d/gpu.conf:");
                println!("  [Service]");
                println!("  Environment=\"VOXTYPE_VULKAN_DEVICE=nvidia\"");
            }
        }
    }

    // Usage hints
    println!();
    if is_parakeet {
        // Parakeet-specific hints
        let current_target = fs::read_link(active_bin)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));
        let is_gpu_active = current_target
            .as_ref()
            .map(|t| t.contains("cuda") || t.contains("rocm"))
            .unwrap_or(false);

        if !is_gpu_active && detect_best_parakeet_gpu_backend().is_some() {
            println!("To enable GPU acceleration:");
            println!("  sudo voxtype setup gpu --enable");
        } else if is_gpu_active {
            println!("To switch back to CPU:");
            println!("  sudo voxtype setup gpu --disable");
        }
    } else {
        if current != Some(Backend::Vulkan) && available.contains(&Backend::Vulkan) {
            println!("To enable GPU acceleration:");
            println!("  sudo voxtype setup gpu --enable");
        } else if current == Some(Backend::Vulkan) {
            println!("To switch back to CPU:");
            println!("  sudo voxtype setup gpu --disable");
        }
    }
}

/// Detect the best Parakeet GPU backend based on available hardware and installed binaries
fn detect_best_parakeet_gpu_backend() -> Option<(&'static str, &'static str)> {
    let gpus = detect_gpus();

    // Check for AMD GPU and ROCm binary
    let has_amd = gpus.iter().any(|g| g.vendor == GpuVendor::Amd);
    let rocm_path = Path::new(VOXTYPE_LIB_DIR).join("voxtype-parakeet-rocm");
    if has_amd && rocm_path.exists() {
        return Some(("voxtype-parakeet-rocm", "ROCm"));
    }

    // Check for NVIDIA GPU and CUDA binary
    let has_nvidia = gpus.iter().any(|g| g.vendor == GpuVendor::Nvidia);
    let cuda_path = Path::new(VOXTYPE_LIB_DIR).join("voxtype-parakeet-cuda");
    if has_nvidia && cuda_path.exists() {
        return Some(("voxtype-parakeet-cuda", "CUDA"));
    }

    // Fall back to whichever is installed (user may have external GPU)
    if rocm_path.exists() {
        return Some(("voxtype-parakeet-rocm", "ROCm"));
    }
    if cuda_path.exists() {
        return Some(("voxtype-parakeet-cuda", "CUDA"));
    }

    None
}

/// Enable GPU backend (engine-aware: Vulkan for Whisper, CUDA/ROCm for Parakeet)
pub fn enable() -> anyhow::Result<()> {
    // Check which engine is active by looking at the current symlink
    let is_parakeet = is_parakeet_binary_active();

    if is_parakeet {
        // Parakeet mode: switch to best available GPU backend (CUDA or ROCm)
        let (backend_binary, backend_name) = detect_best_parakeet_gpu_backend().ok_or_else(|| {
            let gpus = detect_gpus();
            let has_amd = gpus.iter().any(|g| g.vendor == GpuVendor::Amd);
            let has_nvidia = gpus.iter().any(|g| g.vendor == GpuVendor::Nvidia);

            let hint = if has_amd {
                "You have an AMD GPU. Install voxtype-parakeet-rocm for GPU acceleration."
            } else if has_nvidia {
                "You have an NVIDIA GPU. Install voxtype-parakeet-cuda for GPU acceleration."
            } else {
                "No supported GPU detected. Parakeet GPU acceleration requires NVIDIA (CUDA) or AMD (ROCm)."
            };

            anyhow::anyhow!(
                "No Parakeet GPU backend installed.\n\
                 Neither voxtype-parakeet-cuda nor voxtype-parakeet-rocm found in {}\n\n\
                 {}",
                VOXTYPE_LIB_DIR,
                hint
            )
        })?;

        switch_backend_tiered_parakeet(backend_binary)?;

        // Regenerate systemd service if it exists
        if super::systemd::regenerate_service_file()? {
            println!(
                "Updated systemd service to use Parakeet {} backend.",
                backend_name
            );
        }

        println!("Switched to Parakeet ({}) backend.", backend_name);
        println!();
        println!("Restart voxtype to use GPU acceleration:");
        println!("  systemctl --user restart voxtype");
    } else {
        // Whisper mode: switch to Vulkan backend
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
    }

    Ok(())
}

/// Disable GPU backend (engine-aware: switch to best CPU backend)
pub fn disable() -> anyhow::Result<()> {
    // Check which engine is active by looking at the current symlink
    let is_parakeet = is_parakeet_binary_active();

    if is_parakeet {
        // Parakeet mode: switch to best Parakeet CPU backend
        let best_backend = detect_best_parakeet_cpu_backend();
        if let Some(backend_name) = best_backend {
            switch_backend_tiered_parakeet(backend_name)?;
            println!(
                "Switched to Parakeet ({}) backend.",
                backend_name.trim_start_matches("voxtype-parakeet-")
            );
        } else {
            anyhow::bail!(
                "No Parakeet CPU backend found.\n\
                 Install voxtype-parakeet-avx2 or voxtype-parakeet-avx512."
            );
        }

        // Regenerate systemd service if it exists
        if super::systemd::regenerate_service_file()? {
            println!("Updated systemd service to use Parakeet CPU backend.");
        }

        println!();
        println!("Restart voxtype to use CPU inference:");
        println!("  systemctl --user restart voxtype");
    } else {
        // Whisper mode: existing logic
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
    }

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

/// Detect the best Parakeet CPU backend for this system
fn detect_best_parakeet_cpu_backend() -> Option<&'static str> {
    // Check for AVX-512 support
    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
        if cpuinfo.contains("avx512f") {
            let avx512_path = Path::new(VOXTYPE_LIB_DIR).join("voxtype-parakeet-avx512");
            if avx512_path.exists() {
                return Some("voxtype-parakeet-avx512");
            }
        }
    }

    // Fall back to AVX2
    let avx2_path = Path::new(VOXTYPE_LIB_DIR).join("voxtype-parakeet-avx2");
    if avx2_path.exists() {
        return Some("voxtype-parakeet-avx2");
    }

    None
}

/// Switch to a Parakeet backend binary (tiered mode)
fn switch_backend_tiered_parakeet(binary_name: &str) -> anyhow::Result<()> {
    let binary_path = Path::new(VOXTYPE_LIB_DIR).join(binary_name);
    let active_bin = get_active_binary_path();

    if !binary_path.exists() {
        anyhow::bail!(
            "Parakeet backend not found: {}\n\
             Install the appropriate voxtype-parakeet package.",
            binary_path.display()
        );
    }

    // Remove existing symlink
    if Path::new(active_bin).exists() || fs::symlink_metadata(active_bin).is_ok() {
        fs::remove_file(active_bin).map_err(|e| {
            anyhow::anyhow!(
                "Failed to remove existing symlink (need sudo?): {}\n\
                 Try: sudo voxtype setup gpu --enable",
                e
            )
        })?;
    }

    // Create new symlink
    symlink(&binary_path, active_bin).map_err(|e| {
        anyhow::anyhow!(
            "Failed to create symlink (need sudo?): {}\n\
             Try: sudo voxtype setup gpu --enable",
            e
        )
    })?;

    // Restore SELinux context if available
    let _ = Command::new("restorecon").arg(active_bin).status();

    Ok(())
}
