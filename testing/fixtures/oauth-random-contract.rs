// The extracted product function is appended to this dependency-free seam.
// No test calls the host's random syscalls or starves kernel entropy. Only the
// selected platform's libc API exists, so a missing cfg boundary fails to build.
use std::sync::atomic::{AtomicUsize, Ordering};

static MODE: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);

mod libc {
    use super::{Ordering, CALLS, MODE};
    use std::ffi::c_void;

    unsafe extern "C" {
        fn __errno_location() -> *mut i32;
    }

    unsafe fn fail(errno: i32) {
        *__errno_location() = errno;
    }

    #[cfg(test_linux_rng)]
    pub unsafe fn getrandom(buf: *mut c_void, len: usize, flags: u32) -> isize {
        // flags=0 is the initialization-safe Linux contract. A nonblocking or
        // insecure flag must not silently replace it for PKCE or OAuth state.
        assert_eq!(flags, 0);
        let call = CALLS.fetch_add(1, Ordering::SeqCst);
        match MODE.load(Ordering::SeqCst) {
            1 if call == 0 => {
                fail(4);
                return -1;
            } // EINTR
            2 => {
                fail(5);
                return -1;
            } // EIO, never fall back
            3 => return 0, // No progress must fail closed, not spin.
            _ => {}
        }
        let count = len.min(7);
        std::ptr::write_bytes(buf.cast::<u8>(), 0x5a, count);
        count as isize
    }

    #[cfg(not(test_linux_rng))]
    pub unsafe fn getentropy(buf: *mut c_void, len: usize) -> i32 {
        assert!(len <= 256, "macOS getentropy has a 256-byte request limit");
        CALLS.fetch_add(1, Ordering::SeqCst);
        if MODE.load(Ordering::SeqCst) != 0 {
            fail(5);
            return -1;
        }
        std::ptr::write_bytes(buf.cast::<u8>(), 0x5a, len);
        0
    }
}

fn reset(mode: usize) {
    MODE.store(mode, Ordering::SeqCst);
    CALLS.store(0, Ordering::SeqCst);
}

#[test]
fn platform_entropy_contract() {
    for len in [0, 1, 24, 48, 257] {
        reset(0);
        let bytes = random_bytes(len);
        #[cfg(test_linux_rng)]
        let expected_calls = len.div_ceil(7);
        #[cfg(not(test_linux_rng))]
        let expected_calls = len.div_ceil(256);
        // Deterministic API selection/call accounting, not output inequality.
        assert_eq!(CALLS.load(Ordering::SeqCst), expected_calls);
        assert_eq!(bytes, vec![0x5a; len]);
    }
    reset(1);
    #[cfg(test_linux_rng)]
    {
        assert_eq!(random_bytes(24), vec![0x5a; 24]);
        assert_eq!(CALLS.load(Ordering::SeqCst), 5); // EINTR + four short reads
    }
    #[cfg(not(test_linux_rng))]
    {
        assert!(std::panic::catch_unwind(|| random_bytes(24)).is_err());
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }
    for mode in [2, 3] {
        reset(mode);
        assert!(std::panic::catch_unwind(|| random_bytes(48)).is_err());
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }
}
