//! `[meeting] my_name`: the name shown for the user's own side of a
//! meeting instead of "You", and the one-time suggestion of it.
//!
//! The name is never set on its own. When it is unset, voxtype suggests the
//! full name from the user's account (the GECOS field `getent passwd`
//! shows), once, in the terminal; `voxtype meeting my-name --json` offers
//! the same suggestion to a GUI.

use std::path::{Path, PathBuf};

/// Marks that the suggestion has been shown, under `$XDG_STATE_HOME/voxtype`.
pub const SUGGESTED_MARKER: &str = "my_name_suggested";

/// The full name in a passwd GECOS field: the part before the first comma.
pub fn name_from_gecos(gecos: &str) -> Option<String> {
    let name = gecos.split(',').next().unwrap_or("").trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// The current user's full name from the account database, if it has one.
#[cfg(unix)]
pub fn account_full_name() -> Option<String> {
    let uid = unsafe { libc::getuid() };
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc != 0 || result.is_null() || pwd.pw_gecos.is_null() {
        return None;
    }
    let gecos = unsafe { std::ffi::CStr::from_ptr(pwd.pw_gecos) }.to_string_lossy();
    name_from_gecos(&gecos)
}

#[cfg(not(unix))]
pub fn account_full_name() -> Option<String> {
    None
}

pub fn marker_path() -> PathBuf {
    crate::config::Config::state_dir().join(SUGGESTED_MARKER)
}

/// The suggestion to show now, if any: `my_name` is unset, the account has
/// a full name, and the suggestion has not been shown before. Records that
/// it was shown.
pub fn suggest_once(
    configured: Option<&str>,
    account: Option<String>,
    marker: &Path,
) -> Option<String> {
    if configured.is_some_and(|n| !n.trim().is_empty()) || marker.exists() {
        return None;
    }
    let name = account?;
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(marker, &name);
    Some(name)
}

/// The terminal line for a suggestion.
pub fn suggestion_text(name: &str) -> String {
    format!(
        "Tip: meetings show your side as \"You\". To show your name instead, run:\n  \
         voxtype config set meeting.my_name \"{}\"",
        name.replace('"', "\\\"")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gecos_gives_the_full_name_only() {
        assert_eq!(
            name_from_gecos("Pete Jackson,Room 1,555-1234,,"),
            Some("Pete Jackson".into())
        );
        assert_eq!(name_from_gecos("Dana"), Some("Dana".into()));
        assert_eq!(name_from_gecos(",,,"), None);
        assert_eq!(name_from_gecos(""), None);
        assert_eq!(name_from_gecos("   "), None);
    }

    #[test]
    fn suggests_once_and_only_when_unset() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("voxtype").join(SUGGESTED_MARKER);
        let pete = || Some("Pete Jackson".to_string());
        assert_eq!(
            suggest_once(Some("Pete"), pete(), &marker),
            None,
            "already set"
        );
        assert!(!marker.exists());
        assert_eq!(suggest_once(None, None, &marker), None, "no account name");
        assert!(!marker.exists(), "nothing shown, nothing recorded");
        assert_eq!(
            suggest_once(Some("  "), pete(), &marker),
            pete(),
            "blank counts as unset"
        );
        assert!(marker.exists());
        assert_eq!(suggest_once(None, pete(), &marker), None, "only once");
    }

    #[test]
    fn suggestion_names_the_command() {
        assert_eq!(
            suggestion_text("Pete \"PJ\" Jackson"),
            "Tip: meetings show your side as \"You\". To show your name instead, run:\n  \
             voxtype config set meeting.my_name \"Pete \\\"PJ\\\" Jackson\""
        );
    }

    #[test]
    fn account_lookup_does_not_fail() {
        // Whatever the sandbox's passwd says, the lookup returns cleanly.
        let _ = account_full_name();
    }
}
