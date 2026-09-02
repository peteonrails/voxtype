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
    CPU_HAS_AVX2.store(is_x86_feature_detected!("avx2"), Ordering::SeqCst);
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
    let msg = if !CPU_HAS_AVX2.load(Ordering::Relaxed) {
        PRE_AVX2_MESSAGE
    } else {
        concat!(
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
        )
    };

    unsafe {
        libc::write(
            libc::STDERR_FILENO,
            msg.as_ptr() as *const libc::c_void,
            msg.len(),
        );
        libc::_exit(1);
    }
}

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

/// Check CPU feature compatibility and warn if there might be issues.
/// Returns a warning message if potential problems are detected.
#[cfg(target_arch = "x86_64")]
pub fn check_cpu_compatibility() -> Option<String> {
    let in_vm = is_running_in_vm();
    let has_avx2 = std::arch::is_x86_feature_detected!("avx2");
    let has_avx512f = std::arch::is_x86_feature_detected!("avx512f");

    if !has_avx2 {
        return Some(
            "WARNING: Your CPU does not support AVX2. Voxtype requires AVX2 or newer.".to_string(),
        );
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
const PRE_AVX2_MESSAGE: &str = concat!(
    "\n",
    "═══════════════════════════════════════════════════════════════════\n",
    "  FATAL: Illegal CPU instruction (SIGILL)\n",
    "═══════════════════════════════════════════════════════════════════\n",
    "\n",
    "  This CPU does not support AVX2, and every prebuilt voxtype binary\n",
    "  requires it. Switching variants will not help: they are all built\n",
    "  for Haswell (2013) or newer.\n",
    "\n",
    "  To run voxtype here, build it for your own CPU:\n",
    "\n",
    "        RUSTFLAGS=\"-C target-cpu=native\" cargo build --release\n",
    "\n",
    "  See docs/INSTALL.md for the build dependencies.\n",
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

    /// The whole point of #612: this message must not send someone to a
    /// binary their CPU cannot execute, and must give them something that
    /// does work.
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
            PRE_AVX2_MESSAGE.contains("target-cpu=native"),
            "must tell the user how to get a binary that runs"
        );
        assert!(
            PRE_AVX2_MESSAGE.contains("AVX2"),
            "must name the missing feature so the cause is clear"
        );
    }
}
