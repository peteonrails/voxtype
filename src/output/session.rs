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

/// Detect the active display session from environment variables.
pub fn detect() -> DisplaySession {
    if let Ok(val) = std::env::var("WAYLAND_DISPLAY") {
        if !val.is_empty() {
            return DisplaySession::Wayland;
        }
    }

    if let Ok(session) = std::env::var("XDG_SESSION_TYPE") {
        match session.as_str() {
            "wayland" => return DisplaySession::Wayland,
            "x11" => return DisplaySession::X11,
            _ => {}
        }
    }

    if std::env::var("DISPLAY").is_ok() {
        return DisplaySession::X11;
    }

    DisplaySession::Wayland
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to evaluate detect() with a controlled environment.
    ///
    /// These tests mutate process-wide env vars and therefore must not run
    /// concurrently with anything else that reads them. We serialize by
    /// running each in its own `#[test]` and clearing all relevant vars at
    /// the start.
    fn clear_env() {
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::remove_var("XDG_SESSION_TYPE");
        std::env::remove_var("DISPLAY");
    }

    #[test]
    fn wayland_display_wins() {
        clear_env();
        std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
        std::env::set_var("DISPLAY", ":0");
        assert_eq!(detect(), DisplaySession::Wayland);
        clear_env();
    }

    #[test]
    fn empty_wayland_display_does_not_win() {
        clear_env();
        std::env::set_var("WAYLAND_DISPLAY", "");
        std::env::set_var("XDG_SESSION_TYPE", "x11");
        assert_eq!(detect(), DisplaySession::X11);
        clear_env();
    }

    #[test]
    fn xdg_session_type_x11() {
        clear_env();
        std::env::set_var("XDG_SESSION_TYPE", "x11");
        assert_eq!(detect(), DisplaySession::X11);
        clear_env();
    }

    #[test]
    fn xdg_session_type_wayland() {
        clear_env();
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
        assert_eq!(detect(), DisplaySession::Wayland);
        clear_env();
    }

    #[test]
    fn display_only_means_x11() {
        clear_env();
        std::env::set_var("DISPLAY", ":0");
        assert_eq!(detect(), DisplaySession::X11);
        clear_env();
    }

    #[test]
    fn no_env_defaults_to_wayland() {
        clear_env();
        assert_eq!(detect(), DisplaySession::Wayland);
    }
}
