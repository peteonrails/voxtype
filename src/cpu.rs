//! CPU compatibility checks and SIGILL handling
//!
//! Provides graceful error messages when running on incompatible CPUs,
//! particularly in virtualized environments where the hypervisor may not
//! expose all host CPU features.
//!
//! The SIGILL handler is installed via a .init_array constructor, which runs
//! before main() - this is critical because AVX-512 instructions can appear
//! in library initialization code, before our Rust main() even starts.

use std::sync::atomic::{AtomicBool, Ordering};

static SIGILL_HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Whether this CPU has AVX2, sampled when the handler is installed.
///
/// The handler cannot check this itself: it runs in signal context, where only
/// async-signal-safe calls are allowed. Reading an atomic is safe, so the
/// detection happens once up front and the handler only picks a message.
static CPU_HAS_AVX2: AtomicBool = AtomicBool::new(false);

/// Whether this CPU meets x86-64-v2, the floor every prebuilt binary is
/// compiled for, sampled alongside [`CPU_HAS_AVX2`].
static CPU_MEETS_V2: AtomicBool = AtomicBool::new(true);

/// Built at the x86-64-v2 floor rather than Haswell: the baseline daemon,
/// and the OSD, meetings window and audio bridge from every Dockerfile.
/// Those are the binaries that must not send users to the baseline build.
const BUILT_FOR_V2_FLOOR: bool = cfg!(target_arch = "x86_64") && !cfg!(target_feature = "avx2");

/// Constructor function that runs before main() via platform-specific init section
/// This ensures the SIGILL handler is installed before any library
/// initialization code that might use unsupported instructions.
#[used]
#[cfg_attr(target_os = "linux", link_section = ".init_array")]
#[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
static INIT_SIGILL_HANDLER: extern "C" fn() = {
    extern "C" fn init() {
        install_sigill_handler();
    }
    init
};

/// Install a signal handler for SIGILL that prints a helpful error message
/// instead of core dumping.
///
/// This is called automatically before main() via .init_array, but can also
/// be called manually if needed.
pub fn install_sigill_handler() {
    // Only install once
    if SIGILL_HANDLER_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    // Sampled before the handler can ever fire. On a pre-AVX2 CPU the old
    // message told users to switch to the AVX2 binary, which was both the one
    // they were already running and the one their CPU cannot execute (#612).
    #[cfg(target_arch = "x86_64")]
    {
        CPU_HAS_AVX2.store(cpuid_has_avx2(), Ordering::SeqCst);
        CPU_MEETS_V2.store(cpuid_meets_x86_64_v2(), Ordering::SeqCst);
    }
    #[cfg(not(target_arch = "x86_64"))]
    CPU_HAS_AVX2.store(true, Ordering::SeqCst);

    unsafe {
        libc::signal(
            libc::SIGILL,
            sigill_handler as *const () as libc::sighandler_t,
        );
    }
}

extern "C" fn sigill_handler(_sig: i32) {
    // SAFETY: We can only use async-signal-safe functions here.
    // write() to stderr is safe, println! is not.
    let msg = sigill_message(
        is_baseline_build() || BUILT_FOR_V2_FLOOR,
        CPU_HAS_AVX2.load(Ordering::Relaxed),
        CPU_MEETS_V2.load(Ordering::Relaxed),
    );

    unsafe {
        libc::write(
            libc::STDERR_FILENO,
            msg.as_ptr() as *const libc::c_void,
            msg.len(),
        );
        libc::_exit(1);
    }
}

/// Pick the SIGILL message. Pure, so the choice can be tested; the handler
/// feeds it values sampled before any signal could arrive.
///
/// - Below x86-64-v2, no prebuilt voxtype binary runs, the baseline build
///   and the companions (OSD, audio bridge) included (#7883 on Omarchy: a
///   Core 2 was told to switch to the baseline build it was already below).
/// - A v2-floor build on a v2 CPU should never SIGILL; that is our bug.
/// - A Haswell-floor build on a CPU without AVX2 wants the baseline build.
fn sigill_message(
    built_for_v2_floor: bool,
    cpu_has_avx2: bool,
    cpu_meets_v2: bool,
) -> &'static str {
    if !cpu_meets_v2 {
        BELOW_V2_FLOOR_MESSAGE
    } else if built_for_v2_floor {
        BASELINE_SIGILL_MESSAGE
    } else if !cpu_has_avx2 {
        PRE_AVX2_MESSAGE
    } else {
        GENERIC_SIGILL_MESSAGE
    }
}

const GENERIC_SIGILL_MESSAGE: &str = concat!(
    "\n",
    "═══════════════════════════════════════════════════════════════════\n",
    "  FATAL: Illegal CPU instruction (SIGILL)\n",
    "═══════════════════════════════════════════════════════════════════\n",
    "\n",
    "  Your CPU doesn't support an instruction this binary requires.\n",
    "\n",
    "  This commonly happens when:\n",
    "  • Running in a VM that doesn't expose all host CPU features\n",
    "  • Using the AVX-512 binary on a CPU without AVX-512 support\n",
    "\n",
    "  Solutions:\n",
    "  1. If using voxtype-bin, switch to the AVX2 binary:\n",
    "        sudo ln -sf /usr/lib/voxtype/voxtype-avx2 /usr/bin/voxtype\n",
    "\n",
    "  2. If running in a VM, enable CPU passthrough or use the AVX2 binary\n",
    "\n",
    "  3. Run 'voxtype setup check' to verify system compatibility\n",
    "\n",
    "═══════════════════════════════════════════════════════════════════\n",
);

/// Check if running in a virtual machine by checking the hypervisor CPUID bit.
#[cfg(target_arch = "x86_64")]
pub fn is_running_in_vm() -> bool {
    // CPUID leaf 1, ECX bit 31 is the hypervisor present bit
    #[cfg(target_arch = "x86_64")]
    {
        #[allow(unused_unsafe)]
        let result = unsafe { std::arch::x86_64::__cpuid(1) };
        (result.ecx & (1 << 31)) != 0
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn is_running_in_vm() -> bool {
    false
}

/// Whether *this* binary was compiled requiring AVX2.
///
/// True for every variant built with `-C target-cpu=haswell` or higher, false
/// for the baseline build. Without this the baseline binary told users
/// "Voxtype requires AVX2" while running perfectly well on a CPU that has
/// none, which is the binary calling itself impossible.
#[cfg(target_arch = "x86_64")]
const BUILT_REQUIRING_AVX2: bool = cfg!(target_feature = "avx2");

/// Ask the silicon, not the compiler: `is_x86_feature_detected!` statically
/// evaluates to `true` when the queried feature is enabled at compile time,
/// and every non-baseline x86_64 variant is built with `-C target-cpu=haswell`
/// or higher. That folded the pre-AVX2 SIGILL message selection to "has AVX2"
/// in precisely the binaries that crash on pre-AVX2 CPUs - proven on the Ivy
/// Bridge test VM, where the AVX2 binary printed the old message despite the
/// fix being compiled in. CPUID itself executes on any x86-64 CPU.
#[cfg(target_arch = "x86_64")]
// CPUID executes on every x86-64 CPU. Newer Rust declares __cpuid safe;
// Rust 1.93 and earlier (Ubuntu 26.04's default toolchain) still require
// `unsafe`, which the newer compilers then flag as unused.
#[allow(unused_unsafe)]
fn cpuid_has_avx2() -> bool {
    // AVX2: CPUID leaf 7 subleaf 0, EBX bit 5.
    unsafe {
        core::arch::x86_64::__cpuid(0).eax >= 7
            && (core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 5)) != 0
    }
}

/// x86-64-v2: CMPXCHG16B, LAHF/SAHF in 64-bit mode, POPCNT, SSE3, SSSE3,
/// SSE4.1 and SSE4.2. Intel Nehalem (2008) and AMD Bulldozer (2011) and
/// later. Core 2 lacks SSE4.2 and POPCNT.
#[cfg(target_arch = "x86_64")]
#[allow(unused_unsafe)]
fn cpuid_meets_x86_64_v2() -> bool {
    unsafe {
        let ecx = core::arch::x86_64::__cpuid(1).ecx;
        // SSE3 0, SSSE3 9, CX16 13, SSE4.1 19, SSE4.2 20, POPCNT 23.
        let leaf1 = [0, 9, 13, 19, 20, 23].iter().all(|b| ecx & (1 << b) != 0);
        let lahf = core::arch::x86_64::__cpuid(0x8000_0000).eax >= 0x8000_0001
            && core::arch::x86_64::__cpuid(0x8000_0001).ecx & 1 != 0;
        leaf1 && lahf
    }
}

#[cfg(target_arch = "x86_64")]
#[allow(unused_unsafe)]
fn cpuid_has_avx512f() -> bool {
    // AVX-512F: CPUID leaf 7 subleaf 0, EBX bit 16.
    unsafe {
        core::arch::x86_64::__cpuid(0).eax >= 7
            && (core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 16)) != 0
    }
}

/// Whether the CPU has AVX2, asked of the silicon. Use this rather than
/// `is_x86_feature_detected!`, which a Haswell-targeted build folds to
/// `true` at compile time (see [`cpuid_has_avx2`]).
#[cfg(target_arch = "x86_64")]
pub fn cpu_has_avx2() -> bool {
    cpuid_has_avx2()
}

#[cfg(not(target_arch = "x86_64"))]
pub fn cpu_has_avx2() -> bool {
    false
}

/// Whether the CPU has AVX-512F, asked of the silicon. See [`cpu_has_avx2`].
#[cfg(target_arch = "x86_64")]
pub fn cpu_has_avx512f() -> bool {
    cpuid_has_avx512f()
}

#[cfg(not(target_arch = "x86_64"))]
pub fn cpu_has_avx512f() -> bool {
    false
}

/// Check CPU feature compatibility and warn if there might be issues.
/// Returns a warning message if potential problems are detected.
#[cfg(target_arch = "x86_64")]
pub fn check_cpu_compatibility() -> Option<String> {
    let in_vm = is_running_in_vm();
    let has_avx2 = cpuid_has_avx2();
    let has_avx512f = cpuid_has_avx512f();

    if !cpuid_meets_x86_64_v2() {
        return Some(
            "WARNING: This CPU is below x86-64-v2 (it lacks SSE4.2 or POPCNT), the \
             minimum every prebuilt voxtype binary is built for. Expect crashes. \
             Building from source with RUSTFLAGS=\"-C target-cpu=native\" may work."
                .to_string(),
        );
    }

    if !has_avx2 && BUILT_REQUIRING_AVX2 {
        return Some(
            "WARNING: Your CPU does not support AVX2, which this binary requires. \
             Install the baseline variant: voxtype setup variant --to baseline"
                .to_string(),
        );
    }

    // No AVX2 and not built for it: this is the baseline variant doing its job.
    if !has_avx2 {
        return None;
    }

    // If we're in a VM and don't have AVX-512, warn that the AVX-512 binary won't work
    if in_vm && !has_avx512f {
        return Some(
            "NOTE: Running in a VM without AVX-512. Use the AVX2 binary for best compatibility."
                .to_string(),
        );
    }

    None
}

#[cfg(not(target_arch = "x86_64"))]
pub fn check_cpu_compatibility() -> Option<String> {
    None
}

/// Shown when the CPU lacks AVX2, where no shipped x86-64 binary can run:
/// every variant is built with `-C target-cpu=haswell` or higher. The old
/// message sent these users to the AVX2 binary, which was the problem (#612).
/// True when this binary is the baseline (x86-64-v2) variant.
/// Dockerfile.baseline sets VOXTYPE_VARIANT=baseline at build time; every
/// other build leaves it unset. Comparing two 'static strs is pure memory
/// access, so this is safe to call from the signal handler.
fn is_baseline_build() -> bool {
    matches!(option_env!("VOXTYPE_VARIANT"), Some("baseline"))
}

/// SIGILL inside the baseline build itself. The generic messages both point
/// at another variant; from the baseline there is nowhere lower to go, and
/// the correct advice is to report it (#740 was found by a user this message
/// would have saved a round trip).
const BASELINE_SIGILL_MESSAGE: &str = concat!(
    "\n",
    "═══════════════════════════════════════════════════════════════════\n",
    "  FATAL: Illegal CPU instruction (SIGILL)\n",
    "═══════════════════════════════════════════════════════════════════\n",
    "\n",
    "  This binary is built for x86-64-v2 (the baseline variant, the OSD\n",
    "  and the audio bridge all are), and your CPU meets that, so it\n",
    "  should run. A SIGILL here is a bug in the build, not a problem\n",
    "  with your machine.\n",
    "\n",
    "  Please report it, including your CPU model (from /proc/cpuinfo):\n",
    "\n",
    "        https://github.com/peteonrails/voxtype/issues\n",
    "\n",
    "  If this is a VM, also note the hypervisor and CPU model setting.\n",
    "\n",
    "═══════════════════════════════════════════════════════════════════\n",
);

/// SIGILL on a CPU below x86-64-v2. Nothing prebuilt runs there: pointing at
/// the baseline build, as the pre-AVX2 message does, sends the user to
/// another binary that crashes the same way.
const BELOW_V2_FLOOR_MESSAGE: &str = concat!(
    "\n",
    "═══════════════════════════════════════════════════════════════════\n",
    "  FATAL: Illegal CPU instruction (SIGILL)\n",
    "═══════════════════════════════════════════════════════════════════\n",
    "\n",
    "  This CPU is below x86-64-v2, the minimum every prebuilt voxtype\n",
    "  binary is built for: the daemon's baseline build, the OSD and the\n",
    "  audio bridge. It lacks SSE4.2 or POPCNT (Intel Core 2 and older,\n",
    "  AMD before Bulldozer), so no prebuilt variant will run here.\n",
    "\n",
    "  Building from source with RUSTFLAGS=\"-C target-cpu=native\" may\n",
    "  work. If only the OSD crashes, turn it off with [osd] enabled =\n",
    "  false in config.toml.\n",
    "\n",
    "  If this is a VM, the host may well meet x86-64-v2 and the\n",
    "  hypervisor may simply not be exposing it. Enable CPU passthrough.\n",
    "\n",
    "═══════════════════════════════════════════════════════════════════\n",
);

const PRE_AVX2_MESSAGE: &str = concat!(
    "\n",
    "═══════════════════════════════════════════════════════════════════\n",
    "  FATAL: Illegal CPU instruction (SIGILL)\n",
    "═══════════════════════════════════════════════════════════════════\n",
    "\n",
    "  This CPU does not support AVX2, and the binary you are running\n",
    "  requires it.\n",
    "\n",
    "  Switch to the baseline variant, which is built for x86-64-v2 and\n",
    "  runs without AVX2:\n",
    "\n",
    "        voxtype setup variant --to baseline\n",
    "\n",
    "  If your install has no baseline binary, download\n",
    "  voxtype-<version>-linux-x86_64-baseline from the release, or build\n",
    "  from source with RUSTFLAGS=\"-C target-cpu=native\".\n",
    "\n",
    "  If this is a VM, the host may well have AVX2 and the hypervisor may\n",
    "  simply not be exposing it. Enabling CPU passthrough is easier than\n",
    "  building from source.\n",
    "\n",
    "═══════════════════════════════════════════════════════════════════\n",
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for #612. An Ivy Bridge MacBook Pro (avx yes, avx2 no) was
    /// told to switch to the AVX2 binary it was already running.
    ///
    /// The flag is sampled outside signal context, so pin that the installer
    /// actually sets it rather than leaving the AtomicBool at its `false`
    /// default, which would send every user down the pre-AVX2 path.
    /// Test builds use the generic target-cpu, so the std macro performs a
    /// real runtime probe here and must agree with the raw-CPUID helpers.
    /// (In release variants built with target-cpu=haswell the macro constant-
    /// folds to true, which is the bug the helpers exist to avoid; that case
    /// is only observable on pre-AVX2 hardware, e.g. the Ivy Bridge VM.)
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn cpuid_helpers_agree_with_runtime_detection_in_generic_builds() {
        assert_eq!(cpuid_has_avx2(), is_x86_feature_detected!("avx2"));
        assert_eq!(cpuid_has_avx512f(), is_x86_feature_detected!("avx512f"));
    }

    #[test]
    fn avx2_detection_is_sampled_when_the_handler_is_installed() {
        install_sigill_handler();
        let sampled = CPU_HAS_AVX2.load(Ordering::SeqCst);

        #[cfg(target_arch = "x86_64")]
        assert_eq!(
            sampled,
            is_x86_feature_detected!("avx2"),
            "flag must reflect this CPU, not the AtomicBool default"
        );
        #[cfg(not(target_arch = "x86_64"))]
        assert!(sampled, "non-x86_64 must not take the pre-AVX2 path");
    }

    /// A binary that does not require AVX2 must not warn that AVX2 is
    /// required. The baseline build shipped this exact contradiction on the
    /// Ivy Bridge VM: "Voxtype requires AVX2 or newer", printed by a process
    /// that was running fine without it.
    #[test]
    fn the_avx2_warning_matches_what_this_binary_actually_needs() {
        let warning = check_cpu_compatibility();

        #[cfg(target_arch = "x86_64")]
        {
            let has_avx2 = std::arch::is_x86_feature_detected!("avx2");
            // Below x86-64-v2 every binary warns, whatever it was built
            // for. Exercised under `qemu-x86_64 -cpu core2duo`.
            if !cpuid_meets_x86_64_v2() {
                let w = warning.expect("must warn below the x86-64-v2 floor");
                assert!(w.contains("x86-64-v2"), "{w}");
                return;
            }
            if !has_avx2 && !BUILT_REQUIRING_AVX2 {
                assert!(
                    warning.is_none(),
                    "a baseline binary on a pre-AVX2 CPU is working as designed \
                     and must not warn: {warning:?}"
                );
            }
            if !has_avx2 && BUILT_REQUIRING_AVX2 {
                let w = warning
                    .clone()
                    .expect("must warn when it genuinely cannot run");
                assert!(
                    w.contains("baseline"),
                    "the warning must name the variant that would work: {w}"
                );
            }
        }
        let _ = warning;
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn v2_cpuid_check_agrees_with_runtime_detection() {
        let runtime = is_x86_feature_detected!("sse3")
            && is_x86_feature_detected!("ssse3")
            && is_x86_feature_detected!("sse4.1")
            && is_x86_feature_detected!("sse4.2")
            && is_x86_feature_detected!("popcnt")
            && is_x86_feature_detected!("cmpxchg16b");
        // lahfsahf has no std detection macro; every CPU with the others has it.
        assert_eq!(cpuid_meets_x86_64_v2(), runtime);
    }

    /// Omarchy #7883: a Core 2 (below x86-64-v2) running the OSD or audio
    /// bridge was told to switch to the baseline build, which it cannot run
    /// either. Below the floor, every binary says so.
    #[test]
    fn below_the_v2_floor_no_binary_points_at_baseline() {
        for v2_build in [true, false] {
            for avx2 in [true, false] {
                assert_eq!(
                    sigill_message(v2_build, avx2, false),
                    BELOW_V2_FLOOR_MESSAGE
                );
            }
        }
        assert!(BELOW_V2_FLOOR_MESSAGE.contains("x86-64-v2"));
        assert!(!BELOW_V2_FLOOR_MESSAGE.contains("--to baseline"));
    }

    /// A companion (OSD, audio bridge) is a v2-floor build like the
    /// baseline daemon: on a v2 CPU without AVX2 it must report a build bug,
    /// not advise switching to the baseline daemon.
    #[test]
    fn v2_floor_builds_on_a_v2_cpu_report_a_build_bug() {
        assert_eq!(sigill_message(true, false, true), BASELINE_SIGILL_MESSAGE);
        assert_eq!(sigill_message(true, true, true), BASELINE_SIGILL_MESSAGE);
    }

    #[test]
    fn haswell_floor_builds_keep_their_messages() {
        assert_eq!(sigill_message(false, false, true), PRE_AVX2_MESSAGE);
        assert_eq!(sigill_message(false, true, true), GENERIC_SIGILL_MESSAGE);
    }

    /// The whole point of #612: this message must not send someone to a
    /// binary their CPU cannot execute, and must give them something that
    /// does work. Verified on the voxtype-ivybridge VM, where the baseline
    /// binary runs and reports no CPU warning at all.
    #[test]
    fn the_pre_avx2_message_gives_advice_that_can_succeed() {
        assert!(
            !PRE_AVX2_MESSAGE.contains("voxtype-avx2"),
            "must not point at a binary this CPU cannot run"
        );
        assert!(
            !PRE_AVX2_MESSAGE.contains("ln -sf"),
            "must not suggest relinking to another prebuilt variant"
        );
        assert!(
            PRE_AVX2_MESSAGE.contains("baseline"),
            "must point at the baseline variant, which does run here"
        );
        assert!(
            PRE_AVX2_MESSAGE.contains("target-cpu=native"),
            "must keep the build-from-source fallback for installs without it"
        );
        assert!(
            PRE_AVX2_MESSAGE.contains("AVX2"),
            "must name the missing feature so the cause is clear"
        );
    }
}
