//! GPU backend management for voxtype
//!
//! Supports two installation modes:
//! 1. Tiered mode (DEB/RPM pre-built): Multiple CPU binaries (avx2, avx512) + vulkan in /usr/lib/voxtype/
//! 2. Simple mode (AUR source build): Native CPU binary at /usr/bin/voxtype + vulkan in /usr/lib/voxtype/
//!
//! Engine-aware: In ONNX mode, switches between onnx-cuda and onnx-avx*.
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

use super::binary::{install_active_binary, resolve_active_binary};
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

/// Resolve the real binary `/usr/bin/voxtype` dispatches to.
///
/// GPU and ONNX installs replace the symlink with a shell wrapper that
/// `exec`s the binary by canonical path, so ORT's provider lookup lands in
/// the right subdirectory. `fs::canonicalize` on that wrapper returns the
/// wrapper itself, whose filename is just "voxtype", which used to make
/// every check below conclude no ONNX backend was active. `setup onnx
/// --status` already resolved this correctly via `resolve_active_binary`;
/// share that instead of keeping a second, wrapper-blind implementation.
fn resolved_active_binary_name() -> Option<String> {
    let resolved = resolve_active_binary(get_active_binary_path())?;
    resolved
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// Check if the active binary is a Parakeet/ONNX binary
fn is_parakeet_binary_active() -> bool {
    resolved_active_binary_name()
        .map(|name| name.contains("onnx") || name.contains("parakeet"))
        .unwrap_or(false)
}

/// Get the name of the active Parakeet backend binary
fn detect_active_parakeet_backend() -> Option<String> {
    resolved_active_binary_name().filter(|name| name.contains("onnx") || name.contains("parakeet"))
}

/// Parse `ldd` output for dependencies the dynamic linker could not resolve.
///
/// Lines of interest look like `\tlibmigraphx_c.so.3 => not found`.
fn parse_ldd_missing(output: &str) -> Vec<String> {
    let mut missing: Vec<String> = output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (name, rest) = line.split_once("=>")?;
            if rest.trim() == "not found" {
                Some(name.trim().to_string())
            } else {
                None
            }
        })
        .collect();
    missing.sort();
    missing.dedup();
    missing
}

/// Shared libraries an ONNX GPU execution provider needs but cannot resolve.
///
/// The GPU binaries dlopen `libonnxruntime_providers_<ep>.so` at runtime,
/// relative to their own location. When the system libraries that provider
/// links against are absent, ORT fails to register the execution provider and
/// falls back to CPU without saying so. Reporting "Active backend: ONNX GPU
/// (MIGraphX)" in that state tells the user the opposite of the truth (#444:
/// the Arch package shipped voxtype-onnx-migraphx with no migraphx dependency).
///
/// Returns `None` when the check cannot be performed (no provider library
/// alongside the binary, or no `ldd`), and `Some(vec![])` when every
/// dependency resolves.
fn unresolved_provider_deps(binary_name: &str) -> Option<Vec<String>> {
    let resolved = fs::canonicalize(Path::new(VOXTYPE_LIB_DIR).join(binary_name)).ok()?;
    let dir = resolved.parent()?;

    // Providers sit next to the binary; ORT locates them via /proc/self/exe.
    let provider = ["migraphx", "cuda", "rocm"]
        .iter()
        .map(|ep| dir.join(format!("libonnxruntime_providers_{ep}.so")))
        .find(|p| p.exists())?;

    let output = Command::new("ldd").arg(&provider).output().ok()?;
    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }
    Some(parse_ldd_missing(&String::from_utf8_lossy(&output.stdout)))
}

/// Warn when the active GPU backend cannot actually load its execution
/// provider, so `--status` stops advertising acceleration that is not running.
fn warn_if_provider_unloadable(binary_name: &str) {
    let Some(missing) = unresolved_provider_deps(binary_name) else {
        return;
    };
    if missing.is_empty() {
        return;
    }

    println!();
    println!("  WARNING: this backend is selected but cannot load. ONNX Runtime will");
    println!("           fall back to CPU. Missing shared libraries:");
    for lib in &missing {
        println!("             {lib}");
    }
    if binary_name.contains("migraphx") || binary_name.contains("rocm") {
        println!("           Install the AMD runtime, e.g. on Arch:");
        println!("             sudo pacman -S migraphx rocm-hip-runtime");
    } else if binary_name.contains("cuda") {
        println!("           Install the matching NVIDIA CUDA runtime for this variant.");
    }
}

/// Human label for an ONNX variant's binary name.
fn describe_onnx_variant(name: &str) -> &'static str {
    match name {
        "voxtype-onnx-avx2" | "voxtype-parakeet-avx2" => "ONNX CPU (AVX2)",
        "voxtype-onnx-avx512" | "voxtype-parakeet-avx512" => "ONNX CPU (AVX-512)",
        "voxtype-onnx-cuda-12" => "ONNX GPU (CUDA 12)",
        "voxtype-onnx-cuda-13" => "ONNX GPU (CUDA 13)",
        "voxtype-onnx-cuda" | "voxtype-parakeet-cuda" => "ONNX GPU (CUDA, unversioned)",
        "voxtype-onnx-migraphx" => "ONNX GPU (MIGraphX)",
        "voxtype-onnx-rocm" | "voxtype-parakeet-rocm" => "ONNX GPU (MIGraphX, legacy name)",
        _ => "ONNX (unknown variant)",
    }
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
/// Map a binary's file name back to its `Backend`.
///
/// The forward direction lives in `Backend::binary_name`. This is the reverse,
/// used to name the variant a running process is executing.
fn backend_from_binary_name(name: &str) -> Option<Backend> {
    match name {
        "voxtype-cpu" => Some(Backend::Cpu),
        "voxtype-native" => Some(Backend::Native),
        "voxtype-avx2" => Some(Backend::Avx2),
        "voxtype-avx512" => Some(Backend::Avx512),
        "voxtype-vulkan" => Some(Backend::Vulkan),
        _ => None,
    }
}

/// The Whisper backend the live daemon is actually executing.
fn running_whisper_backend(pid: i32) -> Option<(Backend, std::path::PathBuf)> {
    let path = super::binary::running_binary_path(pid)?;
    let name = path.file_name()?.to_str()?;
    backend_from_binary_name(name).map(|b| (b, path))
}

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

/// Vulkan devices as ggml sees them: index and reported device name.
///
/// Shells out to `vulkaninfo --summary` because we have no Vulkan bindings of
/// our own and adding some to answer one question would be a large dependency
/// for a small job. Returns empty when vulkan-tools is not installed, which is
/// common enough that callers must treat it as "unknown", never as "no GPU".
pub fn enumerate_vulkan_devices() -> Vec<(i32, String)> {
    let Ok(out) = Command::new("vulkaninfo").arg("--summary").output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    parse_vulkan_summary(&String::from_utf8_lossy(&out.stdout))
}

/// Pull `(index, deviceName)` pairs out of `vulkaninfo --summary` output.
///
/// Split from the command invocation so the parsing can be tested against
/// captured output rather than whatever hardware the test happens to run on.
fn parse_vulkan_summary(text: &str) -> Vec<(i32, String)> {
    let mut devices = Vec::new();
    let mut current: Option<i32> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("GPU") {
            if let Ok(idx) = rest.trim_end_matches(':').parse::<i32>() {
                current = Some(idx);
            }
        } else if let Some(name) = trimmed.strip_prefix("deviceName") {
            if let Some(idx) = current.take() {
                let name = name.trim_start_matches([' ', '=']).trim().to_string();
                devices.push((idx, name));
            }
        }
    }
    devices
}

/// Lowest device index whose name matches `vendor`.
fn pick_vendor_index(devices: &[(i32, String)], vendor: GpuVendor) -> Option<i32> {
    let needles: &[&str] = match vendor {
        GpuVendor::Nvidia => &["nvidia", "geforce", "rtx", "quadro"],
        GpuVendor::Amd => &["amd", "radeon"],
        GpuVendor::Intel => &["intel"],
        GpuVendor::Other => return None,
    };
    devices
        .iter()
        .filter(|(_, name)| {
            let lower = name.to_lowercase();
            needles.iter().any(|n| lower.contains(n))
        })
        .map(|(idx, _)| *idx)
        .min()
}

/// The Vulkan device index matching a vendor, if one can be identified.
///
/// `VOXTYPE_VULKAN_DEVICE` sets `VK_LOADER_DRIVERS_SELECT`, which asks the
/// loader to expose only that vendor's driver. That does not work everywhere:
/// it needs a recent loader, and on a hybrid machine whisper.cpp has still
/// been observed initialising device 0 (the iGPU) regardless, while setting
/// `gpu_device` by hand works. Resolving the vendor to an index lets us set
/// the thing that actually takes effect (#577).
///
/// Returns the lowest matching index, so a machine with two devices from the
/// same vendor gets the discrete one that Vulkan enumerates first.
pub fn resolve_vulkan_device_index(vendor: GpuVendor) -> Option<i32> {
    pick_vendor_index(&enumerate_vulkan_devices(), vendor)
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

    install_active_binary(active_bin, &binary_path, "setup gpu --enable")
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
            let display_name = describe_onnx_variant(&target);
            // `target` comes from /usr/bin/voxtype, which describes the next
            // process to start. Report what the daemon is actually executing
            // when there is one, and say so when the two disagree; a variant
            // switch does not take effect until a restart.
            let daemon = crate::daemon_status::read_pid_if_alive();
            let running_path = daemon.and_then(super::binary::running_binary_path);

            match (&running_path, daemon) {
                (Some(path), Some(pid)) => {
                    let running_name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");
                    println!(
                        "Active backend: {} (daemon pid {})",
                        describe_onnx_variant(running_name),
                        pid
                    );
                    println!("  Binary: {}", path.display());
                    if running_name != target {
                        println!();
                        println!("  Next launch:  {}", display_name);
                        println!("    {}", Path::new(VOXTYPE_LIB_DIR).join(&target).display());
                        println!("  Restart voxtype to pick it up:");
                        println!("    systemctl --user restart voxtype");
                    }
                    warn_if_provider_unloadable(running_name);
                }
                _ => {
                    println!("Next launch: {} (no daemon running)", display_name);
                    println!(
                        "  Binary: {}",
                        Path::new(VOXTYPE_LIB_DIR).join(&target).display()
                    );
                    warn_if_provider_unloadable(&target);
                }
            }
        } else {
            println!("Active backend: Parakeet (unknown variant)");
        }
    } else {
        // Same split as the ONNX branch above: what the daemon is running
        // now, and separately what /usr/bin/voxtype would launch next. The
        // symlink alone was reported as "active", which is wrong whenever a
        // variant switch has not been followed by a restart.
        let next = detect_current_backend();
        let daemon = crate::daemon_status::read_pid_if_alive();

        match daemon.and_then(running_whisper_backend) {
            Some((running, path)) => {
                println!(
                    "Active backend: {} (daemon pid {})",
                    running.display_name(),
                    daemon.unwrap_or(0)
                );
                println!("  Binary: {}", path.display());

                if let Some(next) = next {
                    if next != running {
                        println!();
                        println!("  Next launch:  {}", next.display_name());
                        println!(
                            "    {}",
                            Path::new(VOXTYPE_LIB_DIR)
                                .join(next.binary_name())
                                .display()
                        );
                        println!("  Restart voxtype to pick it up:");
                        println!("    systemctl --user restart voxtype");
                    }
                }
            }
            None => match next {
                Some(backend) => {
                    println!(
                        "Next launch: {} (no daemon running)",
                        backend.display_name()
                    );
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
            },
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
        // Show ONNX backends (check both new and legacy names)
        let onnx_backends = [
            (
                "voxtype-onnx-avx2",
                "voxtype-parakeet-avx2",
                "ONNX CPU (AVX2)",
            ),
            (
                "voxtype-onnx-avx512",
                "voxtype-parakeet-avx512",
                "ONNX CPU (AVX-512)",
            ),
            (
                "voxtype-onnx-cuda-12",
                "voxtype-onnx-cuda",
                "ONNX GPU (CUDA 12)",
            ),
            (
                "voxtype-onnx-cuda-13",
                "voxtype-onnx-cuda-13",
                "ONNX GPU (CUDA 13)",
            ),
            (
                "voxtype-onnx-migraphx",
                "voxtype-onnx-rocm",
                "ONNX GPU (MIGraphX)",
            ),
        ];

        // Get current symlink target
        let current_target = fs::read_link(active_bin)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));

        for (binary, legacy_binary, display) in onnx_backends {
            let path = Path::new(VOXTYPE_LIB_DIR).join(binary);
            let legacy_path = Path::new(VOXTYPE_LIB_DIR).join(legacy_binary);
            let installed = path.exists() || legacy_path.exists();
            let active = current_target.as_deref() == Some(binary)
                || current_target.as_deref() == Some(legacy_binary);

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
            .map(|t| t.contains("cuda") || t.contains("migraphx") || t.contains("rocm"))
            .unwrap_or(false);

        if !is_gpu_active && detect_best_parakeet_gpu_backend().is_some() {
            println!("To enable GPU acceleration:");
            println!("  sudo voxtype setup gpu --enable");
        } else if is_gpu_active {
            println!("To switch back to CPU:");
            println!("  sudo voxtype setup gpu --disable");
        }
    } else if current != Some(Backend::Vulkan) && available.contains(&Backend::Vulkan) {
        println!("To enable GPU acceleration:");
        println!("  sudo voxtype setup gpu --enable");
    } else if current == Some(Backend::Vulkan) {
        println!("To switch back to CPU:");
        println!("  sudo voxtype setup gpu --disable");
    }
}

/// Detect the best ONNX GPU backend based on available hardware and installed binaries
fn detect_best_parakeet_gpu_backend() -> Option<(&'static str, &'static str)> {
    let gpus = detect_gpus();

    // The CUDA and MIGraphX binaries bundle ONNX Runtime which contains AVX-512
    // instructions. On CPUs without AVX-512 (e.g., Zen 3), these binaries will
    // crash with SIGILL. Only select GPU backends if the CPU supports AVX-512.
    let has_avx512 = fs::read_to_string("/proc/cpuinfo")
        .map(|info| info.contains("avx512f"))
        .unwrap_or(false);

    if !has_avx512 {
        return None;
    }

    // Helper to find installed binary, preferring new name over legacy
    let find_binary = |new_name: &'static str, legacy_name: &'static str| -> Option<&'static str> {
        if Path::new(VOXTYPE_LIB_DIR).join(new_name).exists() {
            Some(new_name)
        } else if Path::new(VOXTYPE_LIB_DIR).join(legacy_name).exists() {
            Some(legacy_name)
        } else {
            None
        }
    };

    // Check for AMD GPU and MIGraphX binary (legacy "rocm" name accepted via symlink)
    let has_amd = gpus.iter().any(|g| g.vendor == GpuVendor::Amd);
    if let Some(binary) = find_binary("voxtype-onnx-migraphx", "voxtype-onnx-rocm") {
        if has_amd {
            return Some((binary, "MIGraphX"));
        }
    }

    // Check for NVIDIA GPU and CUDA binary. v0.7.0 splits cuda into -12 and
    // -13 variants; pick the one matching the host's CUDA runtime so ort's
    // bundled libonnxruntime_providers_cuda.so (built against a fixed CUDA
    // ABI) doesn't fail to register at runtime. Mismatched pairings would
    // silently fall back to CPU.
    let has_nvidia = gpus.iter().any(|g| g.vendor == GpuVendor::Nvidia);
    if has_nvidia {
        let host_cuda_major = crate::setup::parakeet::detect_cuda_runtime_major();
        let cuda_pref: &[&str] = match host_cuda_major {
            Some(13) => &["voxtype-onnx-cuda-13", "voxtype-onnx-cuda"],
            Some(12) => &["voxtype-onnx-cuda-12", "voxtype-onnx-cuda"],
            // No detection — try cu13 first (rolling-distro default), then cu12
            _ => &[
                "voxtype-onnx-cuda-13",
                "voxtype-onnx-cuda-12",
                "voxtype-onnx-cuda",
            ],
        };
        for name in cuda_pref {
            if Path::new(VOXTYPE_LIB_DIR).join(name).exists() {
                let label = match host_cuda_major {
                    Some(13) => "CUDA 13",
                    Some(12) => "CUDA 12",
                    _ => "CUDA",
                };
                return Some((*name, label));
            }
        }
    }

    // Fall back to whichever is installed (user may have external GPU)
    if let Some(binary) = find_binary("voxtype-onnx-migraphx", "voxtype-onnx-rocm") {
        return Some((binary, "MIGraphX"));
    }
    for name in [
        "voxtype-onnx-cuda-13",
        "voxtype-onnx-cuda-12",
        "voxtype-onnx-cuda",
    ] {
        if Path::new(VOXTYPE_LIB_DIR).join(name).exists() {
            return Some((name, "CUDA"));
        }
    }

    None
}

/// Enable GPU backend (engine-aware: Vulkan for Whisper, CUDA/MIGraphX for Parakeet)
pub fn enable() -> anyhow::Result<()> {
    // Check which engine is active by looking at the current symlink
    let is_parakeet = is_parakeet_binary_active();

    if is_parakeet {
        // Parakeet mode: switch to best available GPU backend (CUDA or MIGraphX)
        let (backend_binary, backend_name) = detect_best_parakeet_gpu_backend().ok_or_else(|| {
            let gpus = detect_gpus();
            let has_amd = gpus.iter().any(|g| g.vendor == GpuVendor::Amd);
            let has_nvidia = gpus.iter().any(|g| g.vendor == GpuVendor::Nvidia);
            let has_avx512 = fs::read_to_string("/proc/cpuinfo")
                .map(|info| info.contains("avx512f"))
                .unwrap_or(false);

            let hint = if (has_amd || has_nvidia) && !has_avx512 {
                "You have a GPU, but the ONNX GPU binaries (CUDA/MIGraphX) require a CPU with \
                 AVX-512 support. Your CPU only supports AVX2.\n\n\
                 Use ONNX on CPU instead:\n  \
                 sudo ln -sf /usr/lib/voxtype/voxtype-onnx-avx2 /usr/bin/voxtype\n\n\
                 Or use the Whisper engine with Vulkan GPU acceleration:\n  \
                 voxtype setup onnx --disable && sudo voxtype setup gpu --enable"
            } else if has_amd {
                "You have an AMD GPU. Install voxtype-onnx-migraphx for GPU acceleration."
            } else if has_nvidia {
                "You have an NVIDIA GPU. Install voxtype-onnx-cuda-12 (for CUDA 12.x) or \
                 voxtype-onnx-cuda-13 (for CUDA 13.x) for GPU acceleration."
            } else {
                "No supported GPU detected. ONNX GPU acceleration requires NVIDIA (CUDA) or AMD (MIGraphX)."
            };

            anyhow::anyhow!(
                "No compatible ONNX GPU backend found.\n\n\
                 {}",
                hint
            )
        })?;

        switch_backend_tiered_parakeet(backend_binary)?;

        // Regenerate systemd service if it exists
        if super::systemd::regenerate_service_file()? {
            println!(
                "Updated systemd service to use ONNX {} backend.",
                backend_name
            );
        }

        println!("Switched to ONNX ({}) backend.", backend_name);
        // Selecting a backend whose execution provider cannot load is how
        // users end up believing they have GPU acceleration while running on
        // CPU (#444). Say so at the point of the switch, not just in --status.
        warn_if_provider_unloadable(backend_binary);
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
        // ONNX mode: switch to best ONNX CPU backend
        let best_backend = detect_best_parakeet_cpu_backend();
        if let Some(backend_name) = best_backend {
            switch_backend_tiered_parakeet(backend_name)?;
            println!(
                "Switched to ONNX ({}) backend.",
                backend_name
                    .trim_start_matches("voxtype-onnx-")
                    .trim_start_matches("voxtype-parakeet-")
            );
        } else {
            anyhow::bail!(
                "No ONNX CPU backend found.\n\
                 Install voxtype-onnx-avx2 or voxtype-onnx-avx512."
            );
        }

        // Regenerate systemd service if it exists
        if super::systemd::regenerate_service_file()? {
            println!("Updated systemd service to use ONNX CPU backend.");
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

/// Detect the best ONNX CPU backend for this system
fn detect_best_parakeet_cpu_backend() -> Option<&'static str> {
    // Helper to find installed binary, preferring new name over legacy
    let find_binary = |new_name: &'static str, legacy_name: &'static str| -> Option<&'static str> {
        if Path::new(VOXTYPE_LIB_DIR).join(new_name).exists() {
            Some(new_name)
        } else if Path::new(VOXTYPE_LIB_DIR).join(legacy_name).exists() {
            Some(legacy_name)
        } else {
            None
        }
    };

    // Check for AVX-512 support
    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
        if cpuinfo.contains("avx512f") {
            if let Some(binary) = find_binary("voxtype-onnx-avx512", "voxtype-parakeet-avx512") {
                return Some(binary);
            }
        }
    }

    // Fall back to AVX2
    find_binary("voxtype-onnx-avx2", "voxtype-parakeet-avx2")
}

/// Switch to an ONNX backend binary (tiered mode)
fn switch_backend_tiered_parakeet(binary_name: &str) -> anyhow::Result<()> {
    let binary_path = Path::new(VOXTYPE_LIB_DIR).join(binary_name);
    let active_bin = get_active_binary_path();

    if !binary_path.exists() {
        anyhow::bail!(
            "ONNX backend not found: {}\n\
             Install the appropriate voxtype-onnx package.",
            binary_path.display()
        );
    }

    install_active_binary(active_bin, &binary_path, "setup onnx --enable")
}

#[cfg(test)]
mod tests {

    /// #577: the vendor must resolve to the index whisper actually uses.
    /// Parsing is pinned against real `vulkaninfo --summary` output.
    #[test]
    fn vulkan_summary_parsing_pairs_index_with_name() {
        // Shape taken verbatim from vulkaninfo on a two-device machine.
        let sample = "\
Devices:
========
GPU0:
\tapiVersion         = 1.4.305
\tdriverVersion      = 25.0.3
\tdeviceName         = AMD Radeon RX 7800 XT (RADV NAVI32)
\tdriverName         = radv
GPU1:
\tapiVersion         = 1.4.305
\tdeviceName         = AMD Ryzen 9 9900X3D 12-Core Processor (RADV RAPHAEL_MENDOCINO)
\tdriverName         = radv
";
        let devices = parse_vulkan_summary(sample);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].0, 0);
        assert!(devices[0].1.contains("RX 7800 XT"));
        assert_eq!(devices[1].0, 1);
    }

    /// A hybrid machine: the vendor must map to its own device, not device 0.
    #[test]
    fn vendor_matches_the_right_device_on_a_hybrid_machine() {
        let sample = "\
GPU0:
\tdeviceName         = Intel(R) UHD Graphics 630
GPU1:
\tdeviceName         = NVIDIA GeForce RTX 3060 Laptop GPU
";
        let devices = parse_vulkan_summary(sample);
        assert_eq!(pick_vendor_index(&devices, GpuVendor::Nvidia), Some(1));
        assert_eq!(pick_vendor_index(&devices, GpuVendor::Intel), Some(0));
        assert_eq!(pick_vendor_index(&devices, GpuVendor::Amd), None);
    }

    /// Two devices from one vendor: take the first, which Vulkan enumerates
    /// as the discrete one.
    #[test]
    fn same_vendor_twice_picks_the_lowest_index() {
        let sample = "\
GPU0:
\tdeviceName         = AMD Radeon RX 7800 XT
GPU1:
\tdeviceName         = AMD Ryzen 9 Integrated Graphics
";
        let devices = parse_vulkan_summary(sample);
        assert_eq!(pick_vendor_index(&devices, GpuVendor::Amd), Some(0));
    }

    /// vulkan-tools missing must read as "unknown", never as "no GPU".
    #[test]
    fn empty_enumeration_yields_no_selection() {
        assert_eq!(pick_vendor_index(&[], GpuVendor::Nvidia), None);
    }
    use super::*;

    #[test]
    fn backend_names_round_trip() {
        // running_whisper_backend maps a /proc basename back to a Backend, so
        // every backend's own binary name has to resolve. A miss here is what
        // made setup gpu --status fall back to reading the symlink.
        for b in [
            Backend::Cpu,
            Backend::Native,
            Backend::Avx2,
            Backend::Avx512,
            Backend::Vulkan,
        ] {
            assert_eq!(
                backend_from_binary_name(b.binary_name()),
                Some(b),
                "{} did not round-trip",
                b.binary_name()
            );
        }
    }

    #[test]
    fn backend_from_binary_name_rejects_non_variants() {
        // A source build or an ONNX variant is not a Whisper backend; the
        // caller must get None rather than a wrong label.
        assert_eq!(backend_from_binary_name("voxtype"), None);
        assert_eq!(backend_from_binary_name("voxtype-onnx-migraphx"), None);
        assert_eq!(backend_from_binary_name(""), None);
    }

    #[test]
    fn parse_ldd_missing_finds_unresolved_deps() {
        // Shape of ldd output on a host without the AMD runtime installed,
        // which is the #444 failure: the provider is present, its deps are not.
        let output = "\tlinux-vdso.so.1 (0x00007ffd1b5f2000)\n\
                      \tlibmigraphx_c.so.3 => not found\n\
                      \tlibamdhip64.so.7 => not found\n\
                      \tlibstdc++.so.6 => /usr/lib/libstdc++.so.6 (0x00007f83de9ce000)\n\
                      \tlibc.so.6 => /usr/lib/libc.so.6 (0x00007f83de645000)\n";
        assert_eq!(
            parse_ldd_missing(output),
            vec!["libamdhip64.so.7", "libmigraphx_c.so.3"]
        );
    }

    #[test]
    fn parse_ldd_missing_empty_when_all_resolve() {
        let output =
            "\tlibmigraphx_c.so.3 => /opt/rocm/lib/libmigraphx_c.so.3 (0x00007f83e06e9000)\n\
                      \tlibc.so.6 => /usr/lib/libc.so.6 (0x00007f83de645000)\n";
        assert!(parse_ldd_missing(output).is_empty());
    }

    #[test]
    fn parse_ldd_missing_ignores_static_and_vdso_lines() {
        // Lines without "=>" must not be mistaken for dependencies.
        let output = "\tlinux-vdso.so.1 (0x00007ffd1b5f2000)\n\
                      \t/lib64/ld-linux-x86-64.so.2 (0x00007f83e0a1c000)\n\
                      \tstatically linked\n";
        assert!(parse_ldd_missing(output).is_empty());
    }

    #[test]
    fn parse_ldd_missing_dedupes() {
        let output = "\tlibfoo.so.1 => not found\n\tlibfoo.so.1 => not found\n";
        assert_eq!(parse_ldd_missing(output), vec!["libfoo.so.1"]);
    }
}
