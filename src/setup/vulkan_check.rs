//! Can this machine run the Vulkan build without crashing?
//!
//! Switching `/usr/bin/voxtype` to `voxtype-vulkan` on a machine that cannot
//! run it does not degrade gracefully. Two cases turn it into a crash loop
//! under systemd's `Restart=on-failure`:
//!
//! - The prebuilt `voxtype-vulkan` is compiled with `-C target-cpu=haswell`
//!   (Dockerfile.vulkan), so a CPU without AVX2 dies with SIGILL on start.
//! - ggml's Vulkan backend needs `storageBuffer16BitAccess`. When no GPU
//!   offers it, ggml falls back to the first non-CPU device anyway, throws a
//!   C++ exception from its init, and Rust aborts on the foreign exception.
//!   Haswell's `hasvk` driver is the case reported in the wild (Omarchy
//!   #11370, about 127,000 core dumps on one machine).
//!
//! Both are cheap to check before the switch, so `setup gpu --enable`,
//! `setup variant --to voxtype-vulkan` and `setup gpu --status` ask here first.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Whether the Vulkan device ggml would pick offers 16-bit storage buffers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Storage16 {
    /// ggml will find a device it can use, or none at all (then it runs on
    /// the CPU without crashing).
    Supported,
    /// ggml will fall back to a device without 16-bit storage and abort.
    /// Holds the device names, for the message.
    Unsupported(Vec<String>),
    /// No way to tell: `vulkaninfo` is missing or failed, its output had no
    /// answer, and the PCI fallback did not recognise the GPU.
    Unknown,
}

/// What would make the Vulkan build crash on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VulkanBlocker {
    /// The prebuilt Vulkan binary needs AVX2 and the CPU has none.
    CpuLacksAvx2,
    /// No usable GPU offers 16-bit storage; ggml would abort on load.
    No16BitStorage(Vec<String>),
}

/// Inputs to [`blocker`], gathered by [`detect`] or injected by tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VulkanFacts {
    /// The Vulkan binary was built for the Haswell (AVX2) floor. True for
    /// packaged installs. A source build compiles it for the machine it was
    /// built on, so the AVX2 floor does not apply there.
    pub binary_needs_avx2: bool,
    pub cpu_has_avx2: bool,
    pub storage16: Storage16,
}

/// The reason switching to Vulkan would crash, or `None` when it is safe or
/// cannot be judged. Unknown never blocks: refusing every machine without
/// vulkan-tools would take GPU acceleration away from people it works for.
pub fn blocker(facts: &VulkanFacts) -> Option<VulkanBlocker> {
    if facts.binary_needs_avx2 && !facts.cpu_has_avx2 {
        return Some(VulkanBlocker::CpuLacksAvx2);
    }
    match &facts.storage16 {
        Storage16::Unsupported(devices) => Some(VulkanBlocker::No16BitStorage(devices.clone())),
        Storage16::Supported | Storage16::Unknown => None,
    }
}

impl VulkanBlocker {
    /// One short line for `setup gpu --status`.
    pub fn summary(&self) -> String {
        match self {
            VulkanBlocker::CpuLacksAvx2 => {
                "not usable here: the Vulkan build needs a CPU with AVX2".to_string()
            }
            VulkanBlocker::No16BitStorage(devices) => format!(
                "not usable here: {} lacks the 16-bit storage Vulkan dictation needs",
                join_devices(devices)
            ),
        }
    }

    /// The full refusal, ending with what to do instead. `cpu_hint` names
    /// the CPU build to stay on or switch to.
    pub fn refusal(&self, cpu_hint: &str) -> String {
        let why = match self {
            VulkanBlocker::CpuLacksAvx2 => {
                "The prebuilt Vulkan build (voxtype-vulkan) is compiled for CPUs \
                 with AVX2, and this CPU does not have AVX2. It would crash with \
                 an illegal instruction every time it started."
                    .to_string()
            }
            VulkanBlocker::No16BitStorage(devices) => format!(
                "{} does not support 16-bit storage buffers, which the Vulkan \
                 backend requires. The Vulkan build would abort every time it \
                 loaded a model.",
                join_devices(devices)
            ),
        };
        format!(
            "GPU (Vulkan) acceleration is not available on this machine.\n\n{why}\n\n{cpu_hint}"
        )
    }
}

fn join_devices(devices: &[String]) -> String {
    match devices {
        [] => "This GPU".to_string(),
        [one] => format!("The GPU ({one})"),
        many => format!("The GPUs ({})", many.join(", ")),
    }
}

/// Gather the facts for this machine. Runs `vulkaninfo`, so call it only
/// from setup commands, never from the daemon's hot path.
pub fn detect(binary_needs_avx2: bool) -> VulkanFacts {
    VulkanFacts {
        binary_needs_avx2,
        cpu_has_avx2: crate::cpu::cpu_has_avx2(),
        storage16: probe_storage16(),
    }
}

fn probe_storage16() -> Storage16 {
    let from_vulkaninfo = run_vulkaninfo()
        .map(|text| ggml_outcome(&parse_vulkaninfo(&text)))
        .unwrap_or(Storage16::Unknown);
    if from_vulkaninfo != Storage16::Unknown {
        return from_vulkaninfo;
    }
    // Without vulkan-tools, recognise the one GPU family known to fail.
    match haswell_only_gpu(Path::new("/sys/bus/pci/devices")) {
        Some(names) => Storage16::Unsupported(names),
        None => Storage16::Unknown,
    }
}

/// Full `vulkaninfo` output, or `None` if it is missing, fails or hangs.
/// A broken driver can wedge it, so it gets a deadline.
fn run_vulkaninfo() -> Option<String> {
    let mut child = Command::new("vulkaninfo")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = reader.join().ok()?;
    // vulkaninfo exits non-zero when one of several devices misbehaves but
    // still prints the rest, so judge by what it printed, not its status.
    let text = String::from_utf8_lossy(&out).into_owned();
    (!text.is_empty()).then_some(text)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceType {
    Discrete,
    Integrated,
    Cpu,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VkDevice {
    name: String,
    kind: DeviceType,
    /// Set once the first `deviceType` is read; nested property lists
    /// (layered API entries) can repeat the key.
    typed: bool,
    /// `None` when the output never stated it.
    storage16: Option<bool>,
}

/// Per-device facts from full `vulkaninfo` output. Each device's section
/// starts at a `GPUn:` line; within it, `deviceName`, `deviceType` and every
/// `storageBuffer16BitAccess` line (Vulkan 1.1 features and the 16-bit
/// storage extension both report it) are read.
fn parse_vulkaninfo(text: &str) -> Vec<VkDevice> {
    let mut devices: Vec<VkDevice> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("GPU") {
            if rest
                .strip_suffix(':')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            {
                devices.push(VkDevice {
                    name: String::new(),
                    kind: DeviceType::Other,
                    typed: false,
                    storage16: None,
                });
                continue;
            }
        }
        let Some(dev) = devices.last_mut() else {
            continue;
        };
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "deviceName" if dev.name.is_empty() => dev.name = value.to_string(),
            "deviceType" if !dev.typed => {
                dev.typed = true;
                dev.kind = match value {
                    v if v.contains("DISCRETE_GPU") => DeviceType::Discrete,
                    v if v.contains("INTEGRATED_GPU") => DeviceType::Integrated,
                    v if v.ends_with("TYPE_CPU") => DeviceType::Cpu,
                    _ => DeviceType::Other,
                }
            }
            "storageBuffer16BitAccess" => {
                let v = match value {
                    "true" | "1" => Some(true),
                    "false" | "0" => Some(false),
                    _ => None,
                };
                // Either report saying true is enough.
                dev.storage16 = match (dev.storage16, v) {
                    (Some(true), _) | (_, Some(true)) => Some(true),
                    (_, v @ Some(false)) => v,
                    (old, None) => old,
                };
            }
            _ => {}
        }
    }
    devices
}

/// Replays ggml-vulkan's default device choice (`ggml_vk_instance_init` in
/// whisper.cpp): every discrete or integrated GPU with 16-bit storage is
/// used. If there is none, ggml takes the first non-CPU device regardless,
/// and that is where the unsupported-device exception comes from. With only
/// CPU devices it uses no Vulkan device at all, which is slow but safe.
///
/// `GGML_VK_VISIBLE_DEVICES` overrides this choice; it is not modelled.
fn ggml_outcome(devices: &[VkDevice]) -> Storage16 {
    if devices.is_empty() {
        return Storage16::Unknown;
    }
    let gpu = |d: &&VkDevice| matches!(d.kind, DeviceType::Discrete | DeviceType::Integrated);
    if devices
        .iter()
        .filter(gpu)
        .any(|d| d.storage16 == Some(true))
    {
        return Storage16::Supported;
    }
    let Some(fallback) = devices.iter().find(|d| d.kind != DeviceType::Cpu) else {
        return Storage16::Supported;
    };
    match fallback.storage16 {
        Some(true) => Storage16::Supported,
        Some(false) => {
            let mut names: Vec<String> = devices
                .iter()
                .filter(|d| d.kind != DeviceType::Cpu)
                .map(|d| d.name.clone())
                .filter(|n| !n.is_empty())
                .collect();
            names.dedup();
            Storage16::Unsupported(names)
        }
        None => Storage16::Unknown,
    }
}

/// Intel Haswell (Gen7.5) graphics PCI IDs, including ULT, SDV and Crystal
/// Well: `0x0[4acd][012][26abe]`. Mesa drives these with `hasvk`, which has
/// no 16-bit storage. The same pattern Omarchy's PR #11481 uses.
fn is_haswell_gpu_id(device: &str) -> bool {
    let id = device.trim().to_ascii_lowercase();
    let Some(hex) = id.strip_prefix("0x") else {
        return false;
    };
    let b = hex.as_bytes();
    b.len() == 4
        && b[0] == b'0'
        && matches!(b[1], b'4' | b'a' | b'c' | b'd')
        && matches!(b[2], b'0' | b'1' | b'2')
        && matches!(b[3], b'2' | b'6' | b'a' | b'b' | b'e')
}

/// When `vulkaninfo` is unavailable: a Haswell GPU as the only display
/// device. Another GPU next to it would be picked by ggml instead, so that
/// case is left as unknown. Reads `<root>/*/{vendor,device,class}` in sysfs.
fn haswell_only_gpu(root: &Path) -> Option<Vec<String>> {
    let read = |dir: &Path, f: &str| {
        std::fs::read_to_string(dir.join(f))
            .ok()
            .map(|s| s.trim().to_ascii_lowercase())
    };
    let mut haswell = Vec::new();
    let mut others = 0;
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let dir = entry.path();
        let (Some(vendor), Some(device), Some(class)) = (
            read(&dir, "vendor"),
            read(&dir, "device"),
            read(&dir, "class"),
        ) else {
            continue;
        };
        // PCI class 0x03xxxx is a display controller.
        if !class.starts_with("0x03") {
            continue;
        }
        if vendor == "0x8086" && is_haswell_gpu_id(&device) {
            haswell.push(format!("Intel Haswell graphics, PCI {device}"));
        } else {
            others += 1;
        }
    }
    (!haswell.is_empty() && others == 0).then_some(haswell)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(needs: bool, avx2: bool, s: Storage16) -> VulkanFacts {
        VulkanFacts {
            binary_needs_avx2: needs,
            cpu_has_avx2: avx2,
            storage16: s,
        }
    }

    #[test]
    fn prebuilt_vulkan_is_refused_without_avx2() {
        assert_eq!(
            blocker(&facts(true, false, Storage16::Supported)),
            Some(VulkanBlocker::CpuLacksAvx2)
        );
    }

    /// A source build's Vulkan binary is compiled for the machine it runs on.
    #[test]
    fn source_build_is_not_held_to_the_avx2_floor() {
        assert_eq!(blocker(&facts(false, false, Storage16::Supported)), None);
    }

    #[test]
    fn missing_16bit_storage_is_refused_even_with_avx2() {
        let s = Storage16::Unsupported(vec!["Iris Pro P5200".into()]);
        assert_eq!(
            blocker(&facts(true, true, s)),
            Some(VulkanBlocker::No16BitStorage(vec!["Iris Pro P5200".into()]))
        );
    }

    /// No vulkan-tools must not take GPU acceleration away.
    #[test]
    fn unknown_storage_does_not_block() {
        assert_eq!(blocker(&facts(true, true, Storage16::Unknown)), None);
        assert_eq!(blocker(&facts(true, true, Storage16::Supported)), None);
    }

    #[test]
    fn refusal_names_the_cpu_fallback() {
        let text = VulkanBlocker::CpuLacksAvx2.refusal("Staying on CPU (baseline).");
        assert!(text.contains("AVX2"), "{text}");
        assert!(text.ends_with("Staying on CPU (baseline)."), "{text}");
        let text = VulkanBlocker::No16BitStorage(vec!["HSW GT3".into()]).refusal("x");
        assert!(
            text.contains("HSW GT3") && text.contains("16-bit"),
            "{text}"
        );
    }

    /// Shape of the report in Omarchy #11370: one Haswell iGPU on hasvk,
    /// plus llvmpipe, which ggml never uses.
    const HASWELL: &str = "\
Device Properties and Extensions:
=================================
GPU0:
VkPhysicalDeviceProperties:
---------------------------
\tapiVersion        = 1.2.335
\tdeviceType        = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU
\tdeviceName        = Intel(R) Iris(R) Pro Graphics P5200 (HSW GT3)
VkPhysicalDeviceVulkan11Features:
---------------------------------
\tstorageBuffer16BitAccess           = false
\tuniformAndStorageBuffer16BitAccess = false
GPU1:
VkPhysicalDeviceProperties:
---------------------------
\tdeviceType        = PHYSICAL_DEVICE_TYPE_CPU
\tdeviceName        = llvmpipe (LLVM 21.1.0, 256 bits)
VkPhysicalDeviceVulkan11Features:
---------------------------------
\tstorageBuffer16BitAccess           = true
";

    #[test]
    fn haswell_hasvk_is_unsupported() {
        assert_eq!(
            ggml_outcome(&parse_vulkaninfo(HASWELL)),
            Storage16::Unsupported(vec!["Intel(R) Iris(R) Pro Graphics P5200 (HSW GT3)".into()])
        );
    }

    /// Captured from a Radeon RX 7800 XT + Raphael iGPU host, trimmed.
    #[test]
    fn modern_gpus_are_supported() {
        let text = "\
GPU0:
\tdeviceType        = PHYSICAL_DEVICE_TYPE_DISCRETE_GPU
\tdeviceName        = AMD Radeon RX 7800 XT (RADV NAVI32)
VkPhysicalDeviceVulkan11Features:
\tstorageBuffer16BitAccess           = true
GPU1:
\tdeviceType        = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU
\tdeviceName        = AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO)
VkPhysicalDeviceVulkan11Features:
\tstorageBuffer16BitAccess           = true
";
        assert_eq!(ggml_outcome(&parse_vulkaninfo(text)), Storage16::Supported);
    }

    /// ggml skips an unsupported iGPU when another GPU qualifies, so a
    /// Haswell laptop with a discrete card is fine.
    #[test]
    fn a_supported_dgpu_rescues_a_haswell_igpu() {
        let text = format!(
            "{HASWELL}GPU2:\n\tdeviceType = PHYSICAL_DEVICE_TYPE_DISCRETE_GPU\n\
             \tdeviceName = NVIDIA GeForce GTX 860M\n\tstorageBuffer16BitAccess = true\n"
        );
        assert_eq!(ggml_outcome(&parse_vulkaninfo(&text)), Storage16::Supported);
    }

    /// Only a software rasterizer: ggml uses no Vulkan device and runs on the
    /// CPU, which is pointless but does not crash.
    #[test]
    fn cpu_only_vulkan_does_not_block() {
        let text = "GPU0:\n\tdeviceType = PHYSICAL_DEVICE_TYPE_CPU\n\tdeviceName = llvmpipe\n\
                    \tstorageBuffer16BitAccess = true\n";
        assert_eq!(ggml_outcome(&parse_vulkaninfo(text)), Storage16::Supported);
    }

    /// Layered API property lists can repeat deviceName and deviceType
    /// inside a device's section; the first ones describe the device.
    #[test]
    fn nested_device_keys_do_not_overwrite_the_device() {
        let text = "GPU0:\n\tdeviceType = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU\n\
                    \tdeviceName = Haswell\nVkPhysicalDeviceLayeredApiPropertiesListKHR:\n\
                    \t\tdeviceType = PHYSICAL_DEVICE_TYPE_CPU\n\t\tdeviceName = layered\n\
                    \tstorageBuffer16BitAccess = false\n";
        assert_eq!(
            ggml_outcome(&parse_vulkaninfo(text)),
            Storage16::Unsupported(vec!["Haswell".into()])
        );
    }

    #[test]
    fn unparseable_output_is_unknown() {
        assert_eq!(ggml_outcome(&parse_vulkaninfo("")), Storage16::Unknown);
        assert_eq!(
            ggml_outcome(&parse_vulkaninfo("ERROR: no ICD\n")),
            Storage16::Unknown
        );
        // A device whose output never states the feature.
        let text = "GPU0:\n\tdeviceType = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU\n\tdeviceName = X\n";
        assert_eq!(ggml_outcome(&parse_vulkaninfo(text)), Storage16::Unknown);
    }

    /// `--summary` style "GPU id" lines and group listings are not device
    /// headers.
    #[test]
    fn only_gpun_colon_lines_start_a_device() {
        let text = "Presentable Surfaces:\nGPU id : 0 (AMD)\nDevice Groups:\nGroup 0:\n";
        assert!(parse_vulkaninfo(text).is_empty());
    }

    #[test]
    fn haswell_ids_match_the_omarchy_pattern() {
        for id in ["0x0d26", "0x0416", "0x0a2e", "0x0c02", "0x0402", "0x0D26"] {
            assert!(is_haswell_gpu_id(id), "{id}");
        }
        // Ivy Bridge, Broadwell, Skylake, malformed.
        for id in ["0x0166", "0x1616", "0x1912", "0x0d2", "0d26", ""] {
            assert!(!is_haswell_gpu_id(id), "{id}");
        }
    }

    fn pci(root: &Path, slot: &str, vendor: &str, device: &str, class: &str) {
        let d = root.join(slot);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("vendor"), format!("{vendor}\n")).unwrap();
        std::fs::write(d.join("device"), format!("{device}\n")).unwrap();
        std::fs::write(d.join("class"), format!("{class}\n")).unwrap();
    }

    #[test]
    fn pci_fallback_flags_a_lone_haswell_gpu() {
        let tmp = tempfile::tempdir().unwrap();
        pci(tmp.path(), "0000:00:02.0", "0x8086", "0x0d26", "0x030000");
        // A non-display Intel device does not count as a second GPU.
        pci(tmp.path(), "0000:00:14.0", "0x8086", "0x8c31", "0x0c0330");
        assert_eq!(
            haswell_only_gpu(tmp.path()),
            Some(vec!["Intel Haswell graphics, PCI 0x0d26".into()])
        );
    }

    #[test]
    fn pci_fallback_leaves_hybrid_and_modern_machines_alone() {
        let tmp = tempfile::tempdir().unwrap();
        pci(tmp.path(), "0000:00:02.0", "0x8086", "0x0416", "0x030000");
        pci(tmp.path(), "0000:01:00.0", "0x10de", "0x1392", "0x030200");
        assert_eq!(haswell_only_gpu(tmp.path()), None);

        let tmp = tempfile::tempdir().unwrap();
        pci(tmp.path(), "0000:00:02.0", "0x8086", "0x9a49", "0x030000");
        assert_eq!(haswell_only_gpu(tmp.path()), None);

        assert_eq!(haswell_only_gpu(Path::new("/nonexistent/pci")), None);
    }
}
