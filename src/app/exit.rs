//! The exit path for one-shot CLI commands.
//!
//! A one-shot command that returns from `main` still tears the process down
//! through `_dl_fini`, and that runs the static destructors the ONNX Runtime
//! stack registered. The teardown is unsafe here: `ort` 2.0.0-rc.12 registers
//! `release_env_on_exit` in `.fini_array`, and that cleanup releases memory
//! ONNX Runtime has already released, so glibc catches the damage on its next
//! free and aborts with `corrupted double-linked list` (exit 134). The command
//! has already printed its result by then, so the only visible damage is a
//! crash notification and a multi-hundred-megabyte core dump per invocation
//! ([#772](https://github.com/peteonrails/voxtype/issues/772)).
//!
//! The daemon hit this class of bug first and stopped unwinding on the way out
//! (commit `525a0a6d`, `src/daemon.rs`). One-shot commands need the same
//! treatment: they own nothing that a static destructor releases, so there is
//! nothing to lose by leaving early and a core dump to lose by staying.
//!
//! This is a workaround, not a fix. The defect is upstream in `ort` and ONNX
//! Runtime; [pykeio/ort#610](https://github.com/pykeio/ort/pull/610) replaces
//! the exit hook with an explicitly managed environment, and it is merged but
//! unreleased as of `ort` 2.0.0-rc.13. Revisit this module when voxtype's ONNX
//! engines pin a release that contains it.

use std::io::Write;

/// Terminate the process with `code`, without running static destructors.
///
/// Call this only when the command has finished. It never returns: it skips
/// `_dl_fini`, `atexit` handlers, and every remaining stack frame, so nothing
/// scheduled after this call runs.
///
/// Both stdio streams are flushed first, because `_exit(2)` skips that too.
pub(crate) fn without_destructors(code: i32) -> ! {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // SAFETY: `_exit` takes no pointers, cannot fail, and is async-signal-safe.
    // It terminates the calling process immediately.
    unsafe { libc::_exit(code) }
}
