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
