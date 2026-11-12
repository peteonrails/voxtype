//! Reset SIGPIPE to default behavior (terminate process) instead of the Rust
//! default of ignoring it. This prevents panics when stdout is piped through
//! commands like `head` that close the pipe early.

#[cfg(unix)]
pub(crate) fn reset_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
pub(crate) fn reset_sigpipe() {
    // No-op on non-Unix platforms
}

/// Ignore SIGPIPE for the daemon. The default disposition suits short CLI
/// commands piped into `head`, but the daemon writes into the stdin of
/// helpers it spawns (wl-copy, xclip, paste helpers, the post-process
/// command, dotool), and one that exits before reading, such as wl-copy
/// with no Wayland session, would kill the whole daemon silently. Ignored,
/// the write fails with EPIPE, which each output method already turns into
/// an error and a fallback.
#[cfg(unix)]
pub(crate) fn ignore_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

#[cfg(not(unix))]
pub(crate) fn ignore_sigpipe() {}
