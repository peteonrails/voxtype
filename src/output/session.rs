//! Display session detection
//!
//! Detects whether the user is running under a Wayland compositor or an X11
//! session so that clipboard tooling can be dispatched correctly.
//!
//! Detection order:
//! 1. `WAYLAND_DISPLAY` is set and non-empty: Wayland
//! 2. `XDG_SESSION_TYPE` is `wayland` or `x11`: use that
//! 3. `DISPLAY` is set without `WAYLAND_DISPLAY`: X11
//! 4. Default to Wayland (matches historical voxtype behavior)

/// The user's display session type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplaySession {
    /// Wayland compositor (wl-copy / wl-paste apply)
    Wayland,
    /// X11 session (xclip / xsel apply)
    X11,
}

/// Detect the active display session from the process environment.
///
/// Production callers should use this. Tests should call
/// [`detect_with_env`] with an explicit lookup closure so that the
/// process-wide environment (which the test runner inherits from the
/// developer's interactive session) cannot leak in.
pub fn detect() -> DisplaySession {
    detect_with_env(|name| std::env::var(name).ok())
}

/// Detect the display session using a caller-supplied env lookup.
///
/// Returns `None` from the closure for "unset"; returns `Some(String)` for
/// any value (including the empty string, which has special meaning for
/// `WAYLAND_DISPLAY`).
///
/// This indirection exists so unit tests can run hermetically without
/// mutating `std::env`, which is process-global and racy under parallel
/// test execution.
fn detect_with_env<F>(get: F) -> DisplaySession
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(val) = get("WAYLAND_DISPLAY") {
        if !val.is_empty() {
            return DisplaySession::Wayland;
        }
    }

    if let Some(session) = get("XDG_SESSION_TYPE") {
        match session.as_str() {
            "wayland" => return DisplaySession::Wayland,
            "x11" => return DisplaySession::X11,
            _ => {}
        }
    }

    if get("DISPLAY").is_some() {
        return DisplaySession::X11;
    }

    DisplaySession::Wayland
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build an env lookup from a small literal map.
    ///
    /// These tests used to mutate `std::env` via `set_var` / `remove_var`,
    /// but that approach has two problems:
    ///
    /// 1. The cargo test harness runs tests in parallel by default, so
    ///    one test's `set_var("WAYLAND_DISPLAY", ...)` could be observed
    ///    by another test running concurrently.
    /// 2. `clear_env()` only removed the three vars the tests cared about,
    ///    but the test runner itself inherits `WAYLAND_DISPLAY` from the
    ///    developer's interactive shell on most laptop dev environments.
    ///    A test that "cleared" the env and then expected detect() to
    ///    return X11 would still see the inherited Wayland value if
    ///    another test had set it back, or if the clear ran before that
    ///    other test's set. The failure mode was order-dependent and
    ///    hard to reproduce, which is exactly what bit PR #372.
    ///
    /// The fix is to pass a hermetic `Fn(&str) -> Option<String>` into
    /// the detection logic and never touch the process env in tests.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn wayland_display_wins() {
        let get = env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]);
        assert_eq!(detect_with_env(get), DisplaySession::Wayland);
    }

    #[test]
    fn empty_wayland_display_does_not_win() {
        let get = env(&[("WAYLAND_DISPLAY", ""), ("XDG_SESSION_TYPE", "x11")]);
        assert_eq!(detect_with_env(get), DisplaySession::X11);
    }

    #[test]
    fn xdg_session_type_x11() {
        let get = env(&[("XDG_SESSION_TYPE", "x11")]);
        assert_eq!(detect_with_env(get), DisplaySession::X11);
    }

    #[test]
    fn xdg_session_type_wayland() {
        let get = env(&[("XDG_SESSION_TYPE", "wayland")]);
        assert_eq!(detect_with_env(get), DisplaySession::Wayland);
    }

    #[test]
    fn display_only_means_x11() {
        let get = env(&[("DISPLAY", ":0")]);
        assert_eq!(detect_with_env(get), DisplaySession::X11);
    }

    #[test]
    fn no_env_defaults_to_wayland() {
        let get = env(&[]);
        assert_eq!(detect_with_env(get), DisplaySession::Wayland);
    }
}
