//! CPU compatibility checks and SIGILL handling
//!
//! Provides graceful error messages when running on incompatible CPUs,
//! particularly in virtualized environments where the hypervisor may not
//! expose all host CPU features.

use std::sync::atomic::{AtomicBool, Ordering};

static SIGILL_HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Install a signal handler for SIGILL that prints a helpful error message
/// instead of core dumping.
///
/// This should be called early in main(), before loading the whisper model.
pub fn install_sigill_handler() {
    // Only install once
    if SIGILL_HANDLER_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    unsafe {
        libc::signal(libc::SIGILL, sigill_handler as libc::sighandler_t);
    }
}

extern "C" fn sigill_handler(_sig: i32) {
    // SAFETY: We can only use async-signal-safe functions here.
    // write() to stderr is safe, println! is not.
    let msg = concat!(
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
