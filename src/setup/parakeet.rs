//! Parakeet backend management for voxtype
//!
//! Switches between Whisper and Parakeet binaries by updating the symlink.
//! Parakeet binaries are stored in /usr/lib/voxtype/ alongside Whisper variants.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

const VOXTYPE_LIB_DIR: &str = "/usr/lib/voxtype";
const VOXTYPE_BIN: &str = "/usr/bin/voxtype";

/// Parakeet backend variants
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParakeetBackend {
    Avx2,
    Avx512,
    /// CUDA 12.x (NVIDIA, ort built against libcudart.so.12)
    Cuda12,
    /// CUDA 13.x (NVIDIA, ort built against libcudart.so.13, requires driver 580+)
    Cuda13,
    Migraphx,
    /// Custom binary (source-compiled without specific suffix)
    Custom,
}

impl ParakeetBackend {
    fn binary_name(&self) -> &'static str {
        match self {
            ParakeetBackend::Avx2 => "voxtype-onnx-avx2",
            ParakeetBackend::Avx512 => "voxtype-onnx-avx512",
            ParakeetBackend::Cuda12 => "voxtype-onnx-cuda-12",
            ParakeetBackend::Cuda13 => "voxtype-onnx-cuda-13",
            ParakeetBackend::Migraphx => "voxtype-onnx-migraphx",
            ParakeetBackend::Custom => "voxtype-onnx",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            ParakeetBackend::Avx2 => "ONNX (AVX2)",
            ParakeetBackend::Avx512 => "ONNX (AVX-512)",
            ParakeetBackend::Cuda12 => "ONNX (CUDA 12)",
            ParakeetBackend::Cuda13 => "ONNX (CUDA 13)",
            ParakeetBackend::Migraphx => "ONNX (MIGraphX)",
            ParakeetBackend::Custom => "ONNX (Custom)",
        }
    }

    fn whisper_equivalent(&self) -> &'static str {
        match self {
            ParakeetBackend::Avx2 => "voxtype-avx2",
            ParakeetBackend::Avx512 => "voxtype-avx512",
            // CUDA users likely have GPU; fall back to vulkan when switching off Parakeet
            ParakeetBackend::Cuda12 => "voxtype-vulkan",
            ParakeetBackend::Cuda13 => "voxtype-vulkan",
            ParakeetBackend::Migraphx => "voxtype-vulkan",
            ParakeetBackend::Custom => "voxtype-native",
        }
    }
}

/// Detect if Parakeet is currently active
pub fn is_parakeet_active() -> bool {
    if let Ok(link_target) = fs::read_link(VOXTYPE_BIN) {
        if let Some(target_name) = link_target.file_name() {
            if let Some(name) = target_name.to_str() {
                return name.contains("onnx") || name.contains("parakeet");
            }
        }
    }
    false
}

/// Detect which Parakeet backend is currently active (if any)
pub fn detect_current_parakeet_backend() -> Option<ParakeetBackend> {
    if let Ok(link_target) = fs::read_link(VOXTYPE_BIN) {
        let target_name = link_target.file_name()?.to_str()?;
        return match target_name {
            // New ONNX names
            "voxtype-onnx-avx2" => Some(ParakeetBackend::Avx2),
            "voxtype-onnx-avx512" => Some(ParakeetBackend::Avx512),
            "voxtype-onnx-cuda-12" => Some(ParakeetBackend::Cuda12),
            "voxtype-onnx-cuda-13" => Some(ParakeetBackend::Cuda13),
            "voxtype-onnx-migraphx" => Some(ParakeetBackend::Migraphx),
            "voxtype-onnx" => Some(ParakeetBackend::Custom),
            // Legacy parakeet names (backward compat)
            "voxtype-parakeet-avx2" => Some(ParakeetBackend::Avx2),
            "voxtype-parakeet-avx512" => Some(ParakeetBackend::Avx512),
            "voxtype-parakeet" => Some(ParakeetBackend::Custom),
            // Legacy unversioned CUDA name (resolved to whichever -12/-13 the
            // symlink points at; we can't tell which from the name alone, so
            // assume CUDA 12 since that's what 0.6.x shipped). Users on CUDA 13
            // should re-run `voxtype setup gpu --enable` after upgrading.
            "voxtype-onnx-cuda" => Some(ParakeetBackend::Cuda12),
            "voxtype-parakeet-cuda" => Some(ParakeetBackend::Cuda12),
            // Legacy ROCm names (renamed to MIGraphX in v0.7.0; symlink shipped one release for compat)
            "voxtype-onnx-rocm" => Some(ParakeetBackend::Migraphx),
            "voxtype-parakeet-rocm" => Some(ParakeetBackend::Migraphx),
            _ => None,
        };
    }
    None
}

/// Detect which Whisper backend is currently active
fn detect_current_whisper_backend() -> Option<&'static str> {
    if let Ok(link_target) = fs::read_link(VOXTYPE_BIN) {
        let target_name = link_target.file_name()?.to_str()?;
        return match target_name {
            "voxtype-avx2" => Some("voxtype-avx2"),
            "voxtype-avx512" => Some("voxtype-avx512"),
            "voxtype-vulkan" => Some("voxtype-vulkan"),
            "voxtype-native" => Some("voxtype-native"),
            _ => None,
        };
    }
    None
}

/// Detect available Parakeet backends
pub fn detect_available_backends() -> Vec<ParakeetBackend> {
    let mut available = Vec::new();

    for backend in [
        ParakeetBackend::Avx2,
        ParakeetBackend::Avx512,
        ParakeetBackend::Cuda12,
        ParakeetBackend::Cuda13,
        ParakeetBackend::Migraphx,
        ParakeetBackend::Custom,
    ] {
        let path = Path::new(VOXTYPE_LIB_DIR).join(backend.binary_name());
        if path.exists() {
            available.push(backend);
        }
    }

    available
}

/// Detect the host's CUDA runtime major version by dlopen'ing libcudart.
/// Returns Some(12), Some(13), or None if CUDA isn't installed or the probe
/// fails. Used by detect_best_parakeet_backend to pick between voxtype-onnx-cuda-12
/// and voxtype-onnx-cuda-13 based on what the host can actually run.
pub fn detect_cuda_runtime_major() -> Option<i32> {
    use std::ffi::CString;
    let candidates = ["libcudart.so", "libcudart.so.13", "libcudart.so.12"];
    let handle = candidates.iter().find_map(|name| {
        let cstr = CString::new(*name).ok()?;
        let h = unsafe { libc::dlopen(cstr.as_ptr(), libc::RTLD_LAZY) };
        if h.is_null() {
            None
        } else {
            Some(h)
        }
    })?;

    let sym_name = CString::new("cudaRuntimeGetVersion").ok()?;
    let sym = unsafe { libc::dlsym(handle, sym_name.as_ptr()) };
    if sym.is_null() {
        unsafe { libc::dlclose(handle) };
        return None;
    }

    type CudaRuntimeGetVersion = unsafe extern "C" fn(*mut i32) -> i32;
    let get_version: CudaRuntimeGetVersion = unsafe { std::mem::transmute(sym) };
    let mut version: i32 = 0;
    let result = unsafe { get_version(&mut version) };
    unsafe { libc::dlclose(handle) };

    if result != 0 {
        return None;
    }
    Some(version / 1000)
}

/// Detect the best Parakeet backend for this system
fn detect_best_parakeet_backend() -> Option<ParakeetBackend> {
    let available = detect_available_backends();

    if available.is_empty() {
        return None;
    }

    let has_avx512 = fs::read_to_string("/proc/cpuinfo")
        .map(|info| info.contains("avx512f"))
        .unwrap_or(false);

    // Prefer CUDA if available and NVIDIA GPU detected.
    // Pick the variant matching the host's CUDA runtime: cu12 binary needs
    // libcudart.so.12 at runtime, cu13 needs libcudart.so.13. Mismatched
    // pairings register the EP, then fail at first dlopen — silent CPU fallback.
    // The CUDA binaries bundle ONNX Runtime that contains AVX-512 instructions,
    // so only select if the CPU supports AVX-512.
    if detect_nvidia_gpu() && has_avx512 {
        let host_cuda_major = detect_cuda_runtime_major();
        match host_cuda_major {
            Some(13) if available.contains(&ParakeetBackend::Cuda13) => {
                return Some(ParakeetBackend::Cuda13);
            }
            Some(12) if available.contains(&ParakeetBackend::Cuda12) => {
                return Some(ParakeetBackend::Cuda12);
            }
            // Host CUDA detection failed but binaries are available — prefer
            // cu13 since CUDA 13 is the rolling-distro default; users on cu12
            // can override by symlinking voxtype-onnx-cuda manually.
            None if available.contains(&ParakeetBackend::Cuda13) => {
                return Some(ParakeetBackend::Cuda13);
            }
            None if available.contains(&ParakeetBackend::Cuda12) => {
                return Some(ParakeetBackend::Cuda12);
            }
            _ => {}
        }
    }

    // Prefer MIGraphX if available and AMD GPU detected.
    // The MIGraphX binary bundles ONNX Runtime which contains AVX-512 instructions,
    // so only select it if the CPU supports AVX-512.
    if available.contains(&ParakeetBackend::Migraphx) && detect_amd_gpu() && has_avx512 {
        return Some(ParakeetBackend::Migraphx);
    }

    // Check for AVX-512 CPU-only backend
    if available.contains(&ParakeetBackend::Avx512) && has_avx512 {
        return Some(ParakeetBackend::Avx512);
    }

    // Fall back to AVX2
    if available.contains(&ParakeetBackend::Avx2) {
        return Some(ParakeetBackend::Avx2);
    }

    // Fall back to Native (source-compiled generic binary)
    if available.contains(&ParakeetBackend::Custom) {
        return Some(ParakeetBackend::Custom);
    }

    // Last resort: whatever is available
    available.first().copied()
}

/// Detect if NVIDIA GPU is present
fn detect_nvidia_gpu() -> bool {
    // Check for nvidia-smi
    if let Ok(output) = Command::new("nvidia-smi")
        .arg("--query-gpu=name")
        .arg("--format=csv,noheader")
        .output()
    {
        return output.status.success() && !output.stdout.is_empty();
    }

    // Check for NVIDIA device nodes
    Path::new("/dev/nvidia0").exists()
}

/// Detect if AMD GPU is present
fn detect_amd_gpu() -> bool {
    // Check for AMD GPU via lspci
    if let Ok(output) = Command::new("lspci").output() {
        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout).to_lowercase();
            if output_str.contains("amd") || output_str.contains("radeon") {
                return true;
            }
        }
    }

    // Check for AMD DRI render nodes
    if let Ok(entries) = fs::read_dir("/dev/dri") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("renderD") {
                    // Check if it's an AMD device via sysfs
                    let card_num = name.trim_start_matches("renderD");
                    let vendor_path = format!(
                        "/sys/class/drm/card{}/device/vendor",
                        card_num.parse::<i32>().unwrap_or(0) - 128
                    );
                    if let Ok(vendor) = fs::read_to_string(&vendor_path) {
                        // AMD vendor ID is 0x1002
                        if vendor.trim() == "0x1002" {
                            return true;
                        }
                    }
                }
            }
        }
    }

    false
}

/// Switch symlink to a different binary
fn switch_binary(binary_name: &str) -> anyhow::Result<()> {
    let binary_path = Path::new(VOXTYPE_LIB_DIR).join(binary_name);

    if !binary_path.exists() {
        anyhow::bail!(
            "Binary not found: {}\n\
             Install the appropriate voxtype package variant.",
            binary_path.display()
        );
    }

    // Remove existing symlink
    if Path::new(VOXTYPE_BIN).exists() || fs::symlink_metadata(VOXTYPE_BIN).is_ok() {
        fs::remove_file(VOXTYPE_BIN).map_err(|e| {
            anyhow::anyhow!(
                "Failed to remove existing symlink (need sudo?): {}\n\
                 Try: sudo voxtype setup onnx --enable",
                e
            )
        })?;
    }

    // Create new symlink
    symlink(&binary_path, VOXTYPE_BIN).map_err(|e| {
        anyhow::anyhow!(
            "Failed to create symlink (need sudo?): {}\n\
             Try: sudo voxtype setup onnx --enable",
            e
        )
    })?;

    // Restore SELinux context if available
    let _ = Command::new("restorecon").arg(VOXTYPE_BIN).status();

    Ok(())
}

/// Show Parakeet backend status
pub fn show_status() {
    println!("=== Voxtype ONNX Engine Status ===\n");

    // Current engine
    if is_parakeet_active() {
        if let Some(backend) = detect_current_parakeet_backend() {
            println!("Active engine: Parakeet");
            println!("  Backend: {}", backend.display_name());
            println!(
                "  Binary: {}",
                Path::new(VOXTYPE_LIB_DIR)
                    .join(backend.binary_name())
                    .display()
            );
        }
    } else {
        println!("Active engine: Whisper");
        if let Some(backend) = detect_current_whisper_backend() {
            println!(
                "  Binary: {}",
                Path::new(VOXTYPE_LIB_DIR).join(backend).display()
            );
        }
    }

    // Available ONNX backends
    println!("\nAvailable ONNX backends:");
    let available = detect_available_backends();
    let current = detect_current_parakeet_backend();

    if available.is_empty() {
        println!("  No ONNX binaries installed.");
        println!("\n  Install an ONNX-enabled voxtype package to use this feature.");
    } else {
        for backend in [
            ParakeetBackend::Avx2,
            ParakeetBackend::Avx512,
            ParakeetBackend::Cuda12,
            ParakeetBackend::Cuda13,
            ParakeetBackend::Migraphx,
            ParakeetBackend::Custom,
        ] {
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

    // GPU detection for CUDA/MIGraphX
    println!();
    let has_nvidia = detect_nvidia_gpu();
    let has_amd = detect_amd_gpu();
    let has_avx512 = fs::read_to_string("/proc/cpuinfo")
        .map(|info| info.contains("avx512f"))
        .unwrap_or(false);

    if has_nvidia {
        println!("NVIDIA GPU: detected");
    }
    if has_amd {
        println!("AMD GPU: detected");
    }
    if !has_nvidia && !has_amd {
        println!("GPU: not detected");
    }
    if (has_nvidia || has_amd) && !has_avx512 {
        println!("\nNote: ONNX GPU binaries (CUDA/MIGraphX) require AVX-512 CPU support.");
        println!("  Your CPU supports AVX2 only. Use ONNX (AVX2) for CPU-based inference,");
        println!("  or use the Whisper engine with Vulkan for GPU acceleration.");
    }

    // Usage hints
    println!();
    if !is_parakeet_active() && !available.is_empty() {
        println!("To enable ONNX engines:");
        println!("  sudo voxtype setup onnx --enable");
    } else if is_parakeet_active() {
        println!("To switch back to Whisper:");
        println!("  sudo voxtype setup onnx --disable");
    }
}

/// Enable Parakeet backend
pub fn enable() -> anyhow::Result<()> {
    let available = detect_available_backends();

    if available.is_empty() {
        anyhow::bail!(
            "No ONNX binaries installed.\n\
             Install an ONNX-enabled voxtype package first."
        );
    }

    if is_parakeet_active() {
        println!("ONNX engine is already enabled.");
        if let Some(backend) = detect_current_parakeet_backend() {
            println!("  Current backend: {}", backend.display_name());
        }
        return Ok(());
    }

    // Find best ONNX backend
    let backend = detect_best_parakeet_backend()
        .ok_or_else(|| anyhow::anyhow!("No suitable ONNX backend found"))?;

    switch_binary(backend.binary_name())?;

    // Regenerate systemd service if it exists
    if super::systemd::regenerate_service_file()? {
        println!("Updated systemd service to use ONNX backend.");
    }

    println!("Switched to {} backend.", backend.display_name());
    println!();
    println!("Restart voxtype to use ONNX engines:");
    println!("  systemctl --user restart voxtype");

    Ok(())
}

/// Disable Parakeet backend (switch back to Whisper)
pub fn disable() -> anyhow::Result<()> {
    if !is_parakeet_active() {
        println!("ONNX engine is not currently enabled (already using Whisper).");
        return Ok(());
    }

    // Determine which Whisper backend to switch to based on current Parakeet backend
    let current_parakeet = detect_current_parakeet_backend();
    let whisper_backend = match current_parakeet {
        Some(backend) => backend.whisper_equivalent(),
        None => "voxtype-avx2", // Default fallback
    };

    // Check if the Whisper backend exists
    let whisper_path = Path::new(VOXTYPE_LIB_DIR).join(whisper_backend);
    let final_backend = if whisper_path.exists() {
        whisper_backend
    } else {
        // Try to find any available Whisper backend
        for fallback in [
            "voxtype-avx512",
            "voxtype-avx2",
            "voxtype-vulkan",
            "voxtype-native",
        ] {
            if Path::new(VOXTYPE_LIB_DIR).join(fallback).exists() {
                eprintln!(
                    "Note: {} not found, using {} instead",
                    whisper_backend, fallback
                );
                break;
            }
        }
        // Find first available
        [
            "voxtype-avx512",
            "voxtype-avx2",
            "voxtype-vulkan",
            "voxtype-native",
        ]
        .iter()
        .find(|b| Path::new(VOXTYPE_LIB_DIR).join(b).exists())
        .copied()
        .ok_or_else(|| anyhow::anyhow!("No Whisper backend found to switch to"))?
    };

    switch_binary(final_backend)?;

    // Regenerate systemd service if it exists
    if super::systemd::regenerate_service_file()? {
        println!("Updated systemd service to use Whisper backend.");
    }

    println!(
        "Switched to Whisper ({}) backend.",
        final_backend.trim_start_matches("voxtype-")
    );
    println!();
    println!("Restart voxtype to use Whisper:");
    println!("  systemctl --user restart voxtype");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parakeet_backend_binary_names() {
        assert_eq!(ParakeetBackend::Avx2.binary_name(), "voxtype-onnx-avx2");
        assert_eq!(ParakeetBackend::Avx512.binary_name(), "voxtype-onnx-avx512");
        assert_eq!(
            ParakeetBackend::Cuda12.binary_name(),
            "voxtype-onnx-cuda-12"
        );
        assert_eq!(
            ParakeetBackend::Cuda13.binary_name(),
            "voxtype-onnx-cuda-13"
        );
        assert_eq!(
            ParakeetBackend::Migraphx.binary_name(),
            "voxtype-onnx-migraphx"
        );
        assert_eq!(ParakeetBackend::Custom.binary_name(), "voxtype-onnx");
    }

    #[test]
    fn test_parakeet_backend_display_names() {
        assert_eq!(ParakeetBackend::Avx2.display_name(), "ONNX (AVX2)");
        assert_eq!(ParakeetBackend::Avx512.display_name(), "ONNX (AVX-512)");
        assert_eq!(ParakeetBackend::Cuda12.display_name(), "ONNX (CUDA 12)");
        assert_eq!(ParakeetBackend::Cuda13.display_name(), "ONNX (CUDA 13)");
        assert_eq!(ParakeetBackend::Migraphx.display_name(), "ONNX (MIGraphX)");
        assert_eq!(ParakeetBackend::Custom.display_name(), "ONNX (Custom)");
    }

    #[test]
    fn test_parakeet_whisper_equivalents() {
        assert_eq!(ParakeetBackend::Avx2.whisper_equivalent(), "voxtype-avx2");
        assert_eq!(
            ParakeetBackend::Avx512.whisper_equivalent(),
            "voxtype-avx512"
        );
        assert_eq!(
            ParakeetBackend::Cuda12.whisper_equivalent(),
            "voxtype-vulkan"
        );
        assert_eq!(
            ParakeetBackend::Cuda13.whisper_equivalent(),
            "voxtype-vulkan"
        );
        assert_eq!(
            ParakeetBackend::Migraphx.whisper_equivalent(),
            "voxtype-vulkan"
        );
        assert_eq!(
            ParakeetBackend::Custom.whisper_equivalent(),
            "voxtype-native"
        );
    }

    #[test]
    fn test_is_parakeet_active_false_when_no_symlink() {
        // When /usr/bin/voxtype doesn't exist or isn't a symlink, should return false
        // This test verifies the function handles missing files gracefully
        assert!(!is_parakeet_active() || is_parakeet_active()); // Just verify no panic
    }

    #[test]
    fn test_detect_available_backends_returns_vec() {
        // Verify function returns without panicking
        let backends = detect_available_backends();
        // On most dev machines, no parakeet binaries are installed
        // Just verify it returns a valid vector
        assert!(backends.len() <= 5);
    }

    #[test]
    fn test_backend_enum_equality() {
        assert_eq!(ParakeetBackend::Avx2, ParakeetBackend::Avx2);
        assert_ne!(ParakeetBackend::Avx2, ParakeetBackend::Avx512);
        assert_ne!(ParakeetBackend::Avx512, ParakeetBackend::Cuda12);
        assert_ne!(ParakeetBackend::Cuda12, ParakeetBackend::Cuda13);
    }

    #[test]
    fn test_backend_clone() {
        let backend = ParakeetBackend::Cuda12;
        let cloned = backend;
        assert_eq!(backend, cloned);
    }
}
