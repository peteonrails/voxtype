//! Engine-agnostic voxtype binary inventory and switching.
//!
//! Voxtype ships seven prebuilt variants in `/usr/lib/voxtype/` (Whisper:
//! avx2/avx512/vulkan; ONNX: avx2/avx512/cuda/migraphx). `/usr/bin/voxtype` is a
//! symlink into that directory, and switching engines means updating that
//! symlink.
//!
//! Source builds typically live at `/usr/local/bin/voxtype` or `~/.cargo/bin/`
//! and are a single binary with whatever features were enabled at compile
//! time. They are reported as `InstallKind::Source` and switching is not
//! applicable.

use serde::Serialize;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const LIB_DIR: &str = "/usr/lib/voxtype";
pub const SYSTEM_BIN: &str = "/usr/bin/voxtype";

/// Install `/usr/bin/voxtype` so it dispatches to `binary_path`. CPU-only
/// variants get a plain symlink; GPU/ONNX variants whose binary lives in
/// a /usr/lib/voxtype/<variant>/ subdirectory next to companion ONNX
/// Runtime provider .so files get a thin shell wrapper that `exec`s the
/// canonical real binary path.
///
/// Why the wrapper: ORT's CUDA/MIGraphX EPs resolve their provider .so
/// files from `argv[0]`'s dirname (not /proc/self/exe). A plain symlink
/// at /usr/bin/voxtype leaves argv[0] = "/usr/bin/voxtype", so ORT
/// searches /usr/bin/ for libonnxruntime_providers_*.so, doesn't find
/// them, and silently falls back to CPU. `exec`ing the real binary path
/// replaces argv[0] with that path, so ORT searches the right subdir.
///
/// `binary_path` may be the top-level convenience symlink (e.g.
/// /usr/lib/voxtype/voxtype-onnx-migraphx) or the canonical real path;
/// this function canonicalizes before deciding wrapper vs symlink.
pub fn install_active_binary(active_bin: &str, binary_path: &Path) -> anyhow::Result<()> {
    let canonical = fs::canonicalize(binary_path).unwrap_or_else(|_| binary_path.to_path_buf());

    let needs_wrapper = canonical
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(|name| name.starts_with("cuda-") || name == "migraphx")
        .unwrap_or(false);

    if fs::symlink_metadata(active_bin).is_ok() {
        fs::remove_file(active_bin).map_err(|e| {
            anyhow::anyhow!(
                "Failed to remove existing {} (need sudo?): {}\n\
                 Try: sudo voxtype setup onnx --enable",
                active_bin,
                e
            )
        })?;
    }

    if needs_wrapper {
        // MIGraphX needs a writeable model-cache directory or its runtime
        // fails to save compiled graphs and inference errors out (silent
        // CPU fallback isn't available — the EP fails the call). Default
        // to $XDG_CACHE_HOME/voxtype/migraphx, honoring any user override.
        let is_migraphx = canonical
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            == Some("migraphx");
        let migraphx_env = if is_migraphx {
            "\
             : \"${ORT_MIGRAPHX_MODEL_CACHE_PATH:=${XDG_CACHE_HOME:-$HOME/.cache}/voxtype/migraphx}\"\n\
             mkdir -p \"$ORT_MIGRAPHX_MODEL_CACHE_PATH\"\n\
             export ORT_MIGRAPHX_MODEL_CACHE_PATH\n"
        } else {
            ""
        };
        let wrapper = format!(
            "#!/bin/sh\n\
             # voxtype dispatch wrapper.\n\
             # Execs the GPU/ONNX binary by canonical path so ORT's argv[0]\n\
             # based provider .so lookup resolves to the right subdirectory.\n\
             # Managed by `voxtype setup onnx --enable` and the AUR package's\n\
             # post_install / post_upgrade hooks; do not edit by hand.\n\
             {}\
             exec {} \"$@\"\n",
            migraphx_env,
            canonical.display()
        );
        let mut f = fs::File::create(active_bin).map_err(|e| {
            anyhow::anyhow!(
                "Failed to create {} (need sudo?): {}\n\
                 Try: sudo voxtype setup onnx --enable",
                active_bin,
                e
            )
        })?;
        f.write_all(wrapper.as_bytes())?;
        f.sync_all()?;
        let mut perms = fs::metadata(active_bin)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(active_bin, perms)?;
    } else {
        symlink(binary_path, active_bin).map_err(|e| {
            anyhow::anyhow!(
                "Failed to create symlink (need sudo?): {}\n\
                 Try: sudo voxtype setup onnx --enable",
                e
            )
        })?;
    }

    let _ = Command::new("restorecon").arg(active_bin).status();
    Ok(())
}

/// Read /usr/bin/voxtype and return the canonical real binary it dispatches
/// to, regardless of whether it's a symlink or a wrapper script. Used by
/// the AUR pre-upgrade flow (and equivalent) to preserve the user's chosen
/// backend across package upgrades.
pub fn resolve_active_binary(active_bin: &str) -> Option<PathBuf> {
    let meta = fs::symlink_metadata(active_bin).ok()?;
    if meta.file_type().is_symlink() {
        fs::canonicalize(active_bin).ok()
    } else if meta.file_type().is_file() {
        let content = fs::read_to_string(active_bin).ok()?;
        for line in content.lines() {
            if let Some(rest) = line.trim().strip_prefix("exec ") {
                return rest.split_whitespace().next().map(PathBuf::from);
            }
        }
        None
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineFamily {
    Whisper,
    Onnx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Acceleration {
    Avx2,
    Avx512,
    Vulkan,
    Cuda,
    /// AMD GPU acceleration via the MIGraphX execution provider in ONNX
    /// Runtime. Replaced ROCm in 0.7.0; old `voxtype-onnx-rocm` binary names
    /// still resolve to this variant via [`Variant::from_binary_name`] for
    /// the symlink-compat window.
    Migraphx,
    /// Source-built generic binary (no specific tier).
    Native,
}

/// Every binary name voxtype recognizes in `/usr/lib/voxtype/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Variant {
    WhisperAvx2,
    WhisperAvx512,
    WhisperVulkan,
    WhisperNative,
    OnnxAvx2,
    OnnxAvx512,
    /// CUDA 12.x (NVIDIA, ort built against libcudart.so.12).
    OnnxCuda12,
    /// CUDA 13.x (NVIDIA, ort built against libcudart.so.13, requires driver 580+).
    OnnxCuda13,
    /// Unversioned CUDA binary, present in source-built or pre-0.7.0 installs.
    OnnxCuda,
    OnnxMigraphx,
    OnnxNative,
}

impl Variant {
    pub const ALL: &'static [Variant] = &[
        Variant::WhisperAvx2,
        Variant::WhisperAvx512,
        Variant::WhisperVulkan,
        Variant::WhisperNative,
        Variant::OnnxAvx2,
        Variant::OnnxAvx512,
        // OnnxCuda first so the TUI's (Onnx, Cuda) matrix cell maps to the
        // generic CUDA variant; cu12 and cu13 are specific binaries that
        // live in the inventory list rather than a unique matrix cell.
        Variant::OnnxCuda,
        Variant::OnnxCuda12,
        Variant::OnnxCuda13,
        Variant::OnnxMigraphx,
        Variant::OnnxNative,
    ];

    pub const fn binary_name(self) -> &'static str {
        match self {
            Variant::WhisperAvx2 => "voxtype-avx2",
            Variant::WhisperAvx512 => "voxtype-avx512",
            Variant::WhisperVulkan => "voxtype-vulkan",
            Variant::WhisperNative => "voxtype-native",
            Variant::OnnxAvx2 => "voxtype-onnx-avx2",
            Variant::OnnxAvx512 => "voxtype-onnx-avx512",
            Variant::OnnxCuda12 => "voxtype-onnx-cuda-12",
            Variant::OnnxCuda13 => "voxtype-onnx-cuda-13",
            Variant::OnnxCuda => "voxtype-onnx-cuda",
            Variant::OnnxMigraphx => "voxtype-onnx-migraphx",
            Variant::OnnxNative => "voxtype-onnx",
        }
    }

    pub const fn family(self) -> EngineFamily {
        match self {
            Variant::WhisperAvx2
            | Variant::WhisperAvx512
            | Variant::WhisperVulkan
            | Variant::WhisperNative => EngineFamily::Whisper,
            Variant::OnnxAvx2
            | Variant::OnnxAvx512
            | Variant::OnnxCuda12
            | Variant::OnnxCuda13
            | Variant::OnnxCuda
            | Variant::OnnxMigraphx
            | Variant::OnnxNative => EngineFamily::Onnx,
        }
    }

    pub const fn acceleration(self) -> Acceleration {
        match self {
            Variant::WhisperAvx2 | Variant::OnnxAvx2 => Acceleration::Avx2,
            Variant::WhisperAvx512 | Variant::OnnxAvx512 => Acceleration::Avx512,
            Variant::WhisperVulkan => Acceleration::Vulkan,
            Variant::OnnxCuda12 | Variant::OnnxCuda13 | Variant::OnnxCuda => Acceleration::Cuda,
            Variant::OnnxMigraphx => Acceleration::Migraphx,
            Variant::WhisperNative | Variant::OnnxNative => Acceleration::Native,
        }
    }

    pub const fn display(self) -> &'static str {
        match self {
            Variant::WhisperAvx2 => "Whisper (AVX2)",
            Variant::WhisperAvx512 => "Whisper (AVX-512)",
            Variant::WhisperVulkan => "Whisper (Vulkan)",
            Variant::WhisperNative => "Whisper (native)",
            Variant::OnnxAvx2 => "ONNX (AVX2)",
            Variant::OnnxAvx512 => "ONNX (AVX-512)",
            Variant::OnnxCuda12 => "ONNX (CUDA 12)",
            Variant::OnnxCuda13 => "ONNX (CUDA 13)",
            Variant::OnnxCuda => "ONNX (CUDA)",
            Variant::OnnxMigraphx => "ONNX (MIGraphX)",
            Variant::OnnxNative => "ONNX (native)",
        }
    }

    /// Reverse lookup. Accepts current names plus legacy `voxtype-parakeet*`
    /// names from before the ONNX rename.
    pub fn from_binary_name(name: &str) -> Option<Self> {
        match name {
            "voxtype-avx2" => Some(Variant::WhisperAvx2),
            "voxtype-avx512" => Some(Variant::WhisperAvx512),
            "voxtype-vulkan" => Some(Variant::WhisperVulkan),
            "voxtype-native" => Some(Variant::WhisperNative),
            "voxtype-onnx-avx2" | "voxtype-parakeet-avx2" => Some(Variant::OnnxAvx2),
            "voxtype-onnx-avx512" | "voxtype-parakeet-avx512" => Some(Variant::OnnxAvx512),
            "voxtype-onnx-cuda-12" => Some(Variant::OnnxCuda12),
            "voxtype-onnx-cuda-13" => Some(Variant::OnnxCuda13),
            "voxtype-onnx-cuda" | "voxtype-parakeet-cuda" => Some(Variant::OnnxCuda),
            // Canonical name for the MIGraphX-based ONNX binary (0.7.0+).
            "voxtype-onnx-migraphx" => Some(Variant::OnnxMigraphx),
            // Legacy ROCm names continue to resolve during the symlink-compat
            // window so `voxtype-bin` users with the old AUR symlink still see
            // the variant correctly identified.
            "voxtype-onnx-rocm" | "voxtype-parakeet-rocm" => Some(Variant::OnnxMigraphx),
            "voxtype-onnx" | "voxtype-parakeet" => Some(Variant::OnnxNative),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallKind {
    /// `/usr/bin/voxtype` resolves into `/usr/lib/voxtype/`. Switching is
    /// supported by rewriting that symlink.
    Package,
    /// The running binary lives outside `/usr/lib/voxtype/`. Single binary,
    /// switching not applicable.
    Source,
}

#[derive(Debug, Clone, Serialize)]
pub struct Cpu {
    pub avx2: bool,
    pub avx512: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gpus {
    pub nvidia: bool,
    pub amd: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct VariantStatus {
    pub variant: Variant,
    pub binary_name: String,
    pub installed: bool,
    pub runs_on_this_cpu: bool,
    /// True if the variant has no GPU requirement, or its required GPU vendor
    /// is detected.
    pub gpu_available: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Inventory {
    pub install_kind: InstallKind,
    pub binary_path: PathBuf,
    pub package_lib_dir: Option<PathBuf>,
    pub active_variant: Option<Variant>,
    /// Empty for `InstallKind::Source`.
    pub variants: Vec<VariantStatus>,
    pub cpu: Cpu,
    pub gpus: Gpus,
    pub compiled_features: Vec<&'static str>,
    pub recommendation: Recommendation,
}

/// Hardware-driven recommendations: best variant per engine family for the
/// detected CPU/GPU.
#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub whisper: Variant,
    pub whisper_reason: &'static str,
    pub onnx: Variant,
    pub onnx_reason: &'static str,
    /// Single overall pick when the user has no engine preference. Defaults to
    /// the Whisper recommendation since voxtype's default engine is Whisper.
    pub primary: Variant,
}

pub fn recommend(cpu: &Cpu, gpus: &Gpus) -> Recommendation {
    let whisper = recommend_whisper(cpu, gpus);
    let onnx = recommend_onnx(cpu, gpus);
    Recommendation {
        whisper: whisper.0,
        whisper_reason: whisper.1,
        onnx: onnx.0,
        onnx_reason: onnx.1,
        primary: whisper.0,
    }
}

fn recommend_whisper(cpu: &Cpu, gpus: &Gpus) -> (Variant, &'static str) {
    if gpus.nvidia || gpus.amd {
        // Vulkan covers all GPU vendors and is the most reliable Whisper GPU path.
        return (
            Variant::WhisperVulkan,
            "GPU detected; Vulkan covers NVIDIA, AMD, and Intel in one binary.",
        );
    }
    if cpu.avx512 {
        return (
            Variant::WhisperAvx512,
            "AVX-512 CPU, no GPU; this is the fastest CPU-only Whisper build.",
        );
    }
    (
        Variant::WhisperAvx2,
        "AVX2-only CPU, no GPU; the safe default for Whisper.",
    )
}

fn recommend_onnx(cpu: &Cpu, gpus: &Gpus) -> (Variant, &'static str) {
    // CUDA/MIGraphX bundles ship with AVX-512 ONNX Runtime, so the CPU has to
    // support it before we can recommend a GPU variant.
    if gpus.nvidia && cpu.avx512 {
        return (
            Variant::OnnxCuda,
            "NVIDIA GPU + AVX-512 CPU; CUDA execution provider is the fastest Parakeet path.",
        );
    }
    if gpus.amd && cpu.avx512 {
        return (
            Variant::OnnxAvx512,
            "AMD GPU detected. The MIGraphX execution provider is new and may not register on \
             every driver version; ONNX (AVX-512) on CPU is the safe default. Try ONNX (MIGraphX) \
             once you've verified it works on your card.",
        );
    }
    if cpu.avx512 {
        return (
            Variant::OnnxAvx512,
            "AVX-512 CPU, no compatible GPU; this is the fastest CPU-only ONNX build.",
        );
    }
    (
        Variant::OnnxAvx2,
        "AVX2-only CPU; ONNX (AVX2) keeps Parakeet/Moonshine/etc. available without GPU.",
    )
}

pub fn detect_cpu() -> Cpu {
    Cpu {
        #[cfg(target_arch = "x86_64")]
        avx2: std::arch::is_x86_feature_detected!("avx2"),
        #[cfg(target_arch = "x86_64")]
        avx512: std::arch::is_x86_feature_detected!("avx512f"),
        #[cfg(not(target_arch = "x86_64"))]
        avx2: false,
        #[cfg(not(target_arch = "x86_64"))]
        avx512: false,
    }
}

pub fn detect_gpus() -> Gpus {
    Gpus {
        nvidia: detect_nvidia_gpu(),
        amd: detect_amd_gpu(),
    }
}

fn detect_nvidia_gpu() -> bool {
    if let Ok(output) = Command::new("nvidia-smi")
        .arg("--query-gpu=name")
        .arg("--format=csv,noheader")
        .output()
    {
        if output.status.success() && !output.stdout.is_empty() {
            return true;
        }
    }
    Path::new("/dev/nvidia0").exists()
}

fn detect_amd_gpu() -> bool {
    if let Ok(output) = Command::new("lspci").output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).to_lowercase();
            if s.contains("amd") || s.contains("radeon") {
                return true;
            }
        }
    }
    if let Ok(entries) = fs::read_dir("/dev/dri") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(num) = name.strip_prefix("renderD") {
                    let card_num = num.parse::<i32>().unwrap_or(128) - 128;
                    let vendor_path = format!("/sys/class/drm/card{}/device/vendor", card_num);
                    if let Ok(vendor) = fs::read_to_string(&vendor_path) {
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

/// Path of the currently running binary, with all symlinks resolved.
pub fn current_binary_path() -> PathBuf {
    fs::read_link("/proc/self/exe").unwrap_or_else(|_| PathBuf::from(SYSTEM_BIN))
}

pub fn detect_install_kind(binary_path: &Path) -> InstallKind {
    let canonical = fs::canonicalize(binary_path).unwrap_or_else(|_| binary_path.to_path_buf());
    if canonical.starts_with(LIB_DIR) {
        InstallKind::Package
    } else {
        InstallKind::Source
    }
}

/// Read the `/usr/bin/voxtype` symlink to learn which packaged variant is
/// active. Returns `None` for source installs, missing symlinks, or unknown
/// targets.
pub fn active_variant() -> Option<Variant> {
    // Handle both shapes /usr/bin/voxtype can take: a symlink (CPU
    // variants) or a wrapper script (GPU/ONNX variants whose binary
    // lives in a /usr/lib/voxtype/<variant>/ subdir alongside companion
    // .so files). resolve_active_binary returns the canonical real
    // binary path in both cases; we look up the variant from its
    // filename. Falls back to the legacy fs::read_link path for
    // robustness on edge cases.
    let target = resolve_active_binary(SYSTEM_BIN)
        .or_else(|| fs::read_link(SYSTEM_BIN).ok())?;
    let name = target.file_name()?.to_str()?;
    Variant::from_binary_name(name)
}

pub fn enumerate_installed() -> Vec<Variant> {
    Variant::ALL
        .iter()
        .filter(|v| Path::new(LIB_DIR).join(v.binary_name()).exists())
        .copied()
        .collect()
}

fn variant_runs_on_cpu(v: Variant, cpu: &Cpu) -> bool {
    match v.acceleration() {
        Acceleration::Avx512 => cpu.avx512,
        // ONNX GPU binaries bundle an ONNX Runtime built with AVX-512.
        // Runtime CPU dispatch in ORT mostly handles fallback, but the
        // binary itself can still trip SIGILL on init. Treat AVX-512 as
        // a hard requirement for CUDA/MIGraphX variants.
        Acceleration::Cuda | Acceleration::Migraphx => cpu.avx512,
        Acceleration::Avx2 | Acceleration::Vulkan | Acceleration::Native => cpu.avx2,
    }
}

fn variant_gpu_available(v: Variant, g: &Gpus) -> bool {
    match v.acceleration() {
        Acceleration::Cuda => g.nvidia,
        Acceleration::Migraphx => g.amd,
        _ => true,
    }
}

pub fn compiled_features() -> Vec<&'static str> {
    let mut f = Vec::new();
    if cfg!(feature = "parakeet") {
        f.push("parakeet");
    }
    if cfg!(feature = "gpu-vulkan") {
        f.push("gpu-vulkan");
    }
    if cfg!(feature = "gpu-cuda") {
        f.push("gpu-cuda");
    }
    if cfg!(feature = "gpu-hipblas") {
        f.push("gpu-hipblas");
    }
    if cfg!(feature = "gpu-metal") {
        f.push("gpu-metal");
    }
    f
}

pub fn inventory() -> Inventory {
    let cpu = detect_cpu();
    let gpus = detect_gpus();
    let binary_path = current_binary_path();
    let install_kind = detect_install_kind(&binary_path);
    let active = active_variant();

    let variants = if install_kind == InstallKind::Package {
        Variant::ALL
            .iter()
            .map(|&v| VariantStatus {
                variant: v,
                binary_name: v.binary_name().to_string(),
                installed: Path::new(LIB_DIR).join(v.binary_name()).exists(),
                runs_on_this_cpu: variant_runs_on_cpu(v, &cpu),
                gpu_available: variant_gpu_available(v, &gpus),
                active: active == Some(v),
            })
            .collect()
    } else {
        Vec::new()
    };

    let package_lib_dir = if Path::new(LIB_DIR).is_dir() {
        Some(PathBuf::from(LIB_DIR))
    } else {
        None
    };

    let recommendation = recommend(&cpu, &gpus);

    Inventory {
        install_kind,
        binary_path,
        package_lib_dir,
        active_variant: active,
        variants,
        cpu,
        gpus,
        compiled_features: compiled_features(),
        recommendation,
    }
}

/// Rewrite `/usr/bin/voxtype` to point at the requested variant's binary.
/// Requires write access to `/usr/bin/`; callers should run with sudo.
pub fn switch_to(variant: Variant) -> anyhow::Result<()> {
    let binary_path = Path::new(LIB_DIR).join(variant.binary_name());

    if !binary_path.exists() {
        anyhow::bail!(
            "Binary not found: {}\n\
             Install the appropriate voxtype package variant.",
            binary_path.display()
        );
    }

    install_active_binary(SYSTEM_BIN, &binary_path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_names_are_unique() {
        let mut names: Vec<&str> = Variant::ALL.iter().map(|v| v.binary_name()).collect();
        names.sort();
        let original_len = names.len();
        names.dedup();
        assert_eq!(names.len(), original_len, "duplicate binary names");
    }

    #[test]
    fn round_trip_binary_names() {
        for v in Variant::ALL {
            assert_eq!(Variant::from_binary_name(v.binary_name()), Some(*v));
        }
    }

    #[test]
    fn legacy_parakeet_names_resolve() {
        assert_eq!(
            Variant::from_binary_name("voxtype-parakeet-avx2"),
            Some(Variant::OnnxAvx2)
        );
        assert_eq!(
            Variant::from_binary_name("voxtype-parakeet-cuda"),
            Some(Variant::OnnxCuda)
        );
        assert_eq!(
            Variant::from_binary_name("voxtype-parakeet"),
            Some(Variant::OnnxNative)
        );
    }

    #[test]
    fn unknown_binary_name_is_none() {
        assert_eq!(Variant::from_binary_name("voxtype-totally-fake"), None);
        assert_eq!(Variant::from_binary_name(""), None);
    }

    #[test]
    fn family_partition() {
        let whisper = Variant::ALL
            .iter()
            .filter(|v| v.family() == EngineFamily::Whisper)
            .count();
        let onnx = Variant::ALL
            .iter()
            .filter(|v| v.family() == EngineFamily::Onnx)
            .count();
        assert_eq!(whisper, 4);
        assert_eq!(onnx, 7);
        assert_eq!(whisper + onnx, Variant::ALL.len());
    }

    #[test]
    fn cpu_gating() {
        let no_avx512 = Cpu {
            avx2: true,
            avx512: false,
        };
        assert!(variant_runs_on_cpu(Variant::WhisperAvx2, &no_avx512));
        assert!(!variant_runs_on_cpu(Variant::WhisperAvx512, &no_avx512));
        assert!(!variant_runs_on_cpu(Variant::OnnxCuda, &no_avx512));
        assert!(variant_runs_on_cpu(Variant::WhisperVulkan, &no_avx512));

        let full = Cpu {
            avx2: true,
            avx512: true,
        };
        assert!(variant_runs_on_cpu(Variant::WhisperAvx512, &full));
        assert!(variant_runs_on_cpu(Variant::OnnxCuda, &full));

        let nothing = Cpu {
            avx2: false,
            avx512: false,
        };
        assert!(!variant_runs_on_cpu(Variant::WhisperAvx2, &nothing));
        assert!(!variant_runs_on_cpu(Variant::WhisperNative, &nothing));
    }

    #[test]
    fn gpu_gating() {
        let nvidia_only = Gpus {
            nvidia: true,
            amd: false,
        };
        assert!(variant_gpu_available(Variant::OnnxCuda, &nvidia_only));
        assert!(!variant_gpu_available(Variant::OnnxMigraphx, &nvidia_only));
        assert!(variant_gpu_available(Variant::WhisperVulkan, &nvidia_only));

        let none = Gpus {
            nvidia: false,
            amd: false,
        };
        assert!(!variant_gpu_available(Variant::OnnxCuda, &none));
        assert!(!variant_gpu_available(Variant::OnnxMigraphx, &none));
        assert!(variant_gpu_available(Variant::WhisperAvx2, &none));
    }

    #[test]
    fn detect_install_kind_classifies_package_vs_source() {
        assert_eq!(
            detect_install_kind(Path::new("/usr/lib/voxtype/voxtype-avx2")),
            InstallKind::Package
        );
        assert_eq!(
            detect_install_kind(Path::new("/usr/local/bin/voxtype")),
            InstallKind::Source
        );
        assert_eq!(
            detect_install_kind(Path::new("/home/user/.cargo/bin/voxtype")),
            InstallKind::Source
        );
    }

    #[test]
    fn recommendations_match_hardware() {
        // No GPU, AVX2 only → Whisper AVX2 + ONNX AVX2.
        let r = recommend(
            &Cpu { avx2: true, avx512: false },
            &Gpus { nvidia: false, amd: false },
        );
        assert_eq!(r.whisper, Variant::WhisperAvx2);
        assert_eq!(r.onnx, Variant::OnnxAvx2);
        assert_eq!(r.primary, Variant::WhisperAvx2);

        // No GPU, AVX-512 → Whisper AVX-512 + ONNX AVX-512.
        let r = recommend(
            &Cpu { avx2: true, avx512: true },
            &Gpus { nvidia: false, amd: false },
        );
        assert_eq!(r.whisper, Variant::WhisperAvx512);
        assert_eq!(r.onnx, Variant::OnnxAvx512);

        // NVIDIA + AVX-512 → Whisper Vulkan + ONNX CUDA.
        let r = recommend(
            &Cpu { avx2: true, avx512: true },
            &Gpus { nvidia: true, amd: false },
        );
        assert_eq!(r.whisper, Variant::WhisperVulkan);
        assert_eq!(r.onnx, Variant::OnnxCuda);

        // NVIDIA but no AVX-512 → CUDA bundle won't load, fall back to ONNX AVX2.
        let r = recommend(
            &Cpu { avx2: true, avx512: false },
            &Gpus { nvidia: true, amd: false },
        );
        assert_eq!(r.whisper, Variant::WhisperVulkan);
        assert_eq!(r.onnx, Variant::OnnxAvx2);

        // AMD + AVX-512 → Vulkan for Whisper, AVX-512 (not MIGraphX) for ONNX.
        let r = recommend(
            &Cpu { avx2: true, avx512: true },
            &Gpus { nvidia: false, amd: true },
        );
        assert_eq!(r.whisper, Variant::WhisperVulkan);
        assert_eq!(r.onnx, Variant::OnnxAvx512);
    }

    #[test]
    fn inventory_runs_without_panicking() {
        let inv = inventory();
        assert!(matches!(
            inv.install_kind,
            InstallKind::Package | InstallKind::Source
        ));
        let _ = inv.recommendation;
    }
}
