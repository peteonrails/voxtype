//! Why the evdev listener could not open any keyboard.
//!
//! The listener skips devices it gets EACCES on, so without this the only
//! trace of a missing `input` group is "No keyboard device found" in the
//! journal while the daemon reports itself running. Telling "not in the
//! group" apart from "in the group, but this session predates it" matters:
//! the second user already ran `usermod` and only needs to log out.

use std::path::{Path, PathBuf};

use crate::error::HotkeyError;

/// Where the user stands with respect to the `input` group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputGroupStatus {
    /// Not a member, or no `input` group exists on this system.
    NotMember,
    /// Listed as a member in the group database, but the running process
    /// does not carry the group: the session started before `usermod`.
    MemberNotActive,
    /// The process already has the group, so the denial has another cause
    /// (unusual device ownership or mode).
    Active,
}

/// Classify `input` group membership from the group database entry and the
/// process credentials. Pure so it can be tested without touching the system.
///
/// `group` is the `input` group's gid and member list, `None` when the group
/// does not exist. `primary_gid` is the user's login group from the passwd
/// entry, which counts as membership without being listed in the group's
/// members.
pub fn classify_input_group(
    group: Option<(u32, &[String])>,
    user_name: Option<&str>,
    primary_gid: Option<u32>,
    process_gids: &[u32],
) -> InputGroupStatus {
    let Some((gid, members)) = group else {
        return InputGroupStatus::NotMember;
    };
    if process_gids.contains(&gid) {
        return InputGroupStatus::Active;
    }
    let listed = user_name.is_some_and(|name| members.iter().any(|m| m == name));
    if listed || primary_gid == Some(gid) {
        InputGroupStatus::MemberNotActive
    } else {
        InputGroupStatus::NotMember
    }
}

/// Look up the current user's `input` group status on this system.
pub fn input_group_status() -> InputGroupStatus {
    use nix::unistd::{getegid, geteuid, getgroups, Group, User};

    let group = Group::from_name("input").ok().flatten();
    let user = User::from_uid(geteuid()).ok().flatten();
    let mut process_gids: Vec<u32> = getgroups()
        .unwrap_or_default()
        .into_iter()
        .map(|g| g.as_raw())
        .collect();
    process_gids.push(getegid().as_raw());

    classify_input_group(
        group.as_ref().map(|g| (g.gid.as_raw(), g.mem.as_slice())),
        user.as_ref().map(|u| u.name.as_str()),
        user.as_ref().map(|u| u.gid.as_raw()),
        &process_gids,
    )
}

/// The error to report when no keyboard could be opened. `denied` is the
/// first device that failed with EACCES, if any did.
pub fn no_keyboard_error(denied: Option<&str>, status: InputGroupStatus) -> HotkeyError {
    let Some(path) = denied else {
        return HotkeyError::NoKeyboard;
    };
    let path = path.to_string();
    match status {
        InputGroupStatus::NotMember => HotkeyError::DeviceAccess(path),
        InputGroupStatus::MemberNotActive => HotkeyError::InputGroupInactive(path),
        InputGroupStatus::Active => HotkeyError::DeviceDenied(path),
    }
}

/// Title and body for the desktop notification that tells the user the
/// built-in hotkey could not start and how to fix it. `None` for errors that
/// already stop the daemon or are configuration mistakes reported elsewhere.
///
/// The title says "built-in": someone who binds `voxtype record toggle` in
/// their compositor and is not in the `input` group has a working hotkey,
/// and the body tells them how to turn this one off.
pub fn startup_failure_notification(err: &HotkeyError) -> Option<(&'static str, String)> {
    const ALTERNATIVE: &str = "If you bind `voxtype record toggle` in your compositor \
                               instead, set [hotkey] enabled = false (or start with \
                               --no-hotkey) to turn the built-in hotkey off and silence this.";
    let body = match err {
        HotkeyError::DeviceAccess(_) => "The built-in hotkey can't read keyboards in /dev/input. \
             Bind `voxtype record toggle` (or record start/stop) in your compositor and set \
             [hotkey] enabled = false (or start with --no-hotkey) to turn the built-in hotkey \
             off and silence this. Adding yourself to the 'input' group also works but lets \
             every program read your keystrokes."
            .to_string(),
        HotkeyError::InputGroupInactive(_) => format!(
            "You are in the 'input' group, but this session started before you were added. \
             Log out completely and back in (or reboot). {}",
            ALTERNATIVE
        ),
        HotkeyError::DeviceDenied(path) => format!(
            "Permission denied opening {} although voxtype has the 'input' group. \
             Check `ls -l /dev/input/event*`. {}",
            path, ALTERNATIVE
        ),
        HotkeyError::NoKeyboard => format!(
            "No keyboard found in /dev/input. Plug one in and restart voxtype. {}",
            ALTERNATIVE
        ),
        HotkeyError::UnknownKey(_) | HotkeyError::Evdev(_) => return None,
    };
    Some(("Voxtype: built-in hotkey unavailable", body))
}

/// Marker under the state directory holding the cause last notified, so the
/// notice is shown once per install rather than on every daemon start. The
/// same scheme as the doctor's startup summary, kept across reboots.
pub fn notice_marker_path() -> PathBuf {
    crate::config::Config::state_dir().join("hotkey-unavailable-notified")
}

/// The cause of `err` as stored in the marker. A different cause (not in
/// the group, then in it but not logged out) has a different fix, so it
/// notifies again.
fn failure_key(err: &HotkeyError) -> &'static str {
    match err {
        HotkeyError::DeviceAccess(_) => "not-in-input-group",
        HotkeyError::InputGroupInactive(_) => "input-group-inactive",
        HotkeyError::DeviceDenied(_) => "device-denied",
        HotkeyError::NoKeyboard => "no-keyboard",
        HotkeyError::UnknownKey(_) | HotkeyError::Evdev(_) => "other",
    }
}

/// [`startup_failure_notification`] for a listener that failed to start,
/// unless `marker` says this cause was already notified. Records the cause
/// when it returns a notice.
pub fn notify_once(err: &HotkeyError, marker: &Path) -> Option<(&'static str, String)> {
    let notice = startup_failure_notification(err)?;
    let key = failure_key(err);
    let previous = std::fs::read_to_string(marker).ok();
    if previous.as_deref().map(str::trim) == Some(key) {
        return None;
    }
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(marker, key) {
        tracing::debug!("Could not write {:?}: {}", marker, e);
    }
    Some(notice)
}

/// The listener opened a keyboard: forget the notice, so a later breakage
/// is reported again.
pub fn clear_notice(marker: &Path) {
    let _ = std::fs::remove_file(marker);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_input_group_is_not_member() {
        assert_eq!(
            classify_input_group(None, Some("alice"), Some(1000), &[1000]),
            InputGroupStatus::NotMember
        );
    }

    #[test]
    fn unlisted_user_is_not_member() {
        let m = members(&["bob"]);
        assert_eq!(
            classify_input_group(Some((97, &m)), Some("alice"), Some(1000), &[1000, 998]),
            InputGroupStatus::NotMember
        );
    }

    #[test]
    fn listed_but_not_in_process_groups_needs_relogin() {
        let m = members(&["bob", "alice"]);
        assert_eq!(
            classify_input_group(Some((97, &m)), Some("alice"), Some(1000), &[1000, 998]),
            InputGroupStatus::MemberNotActive
        );
    }

    #[test]
    fn primary_group_counts_as_membership() {
        assert_eq!(
            classify_input_group(Some((97, &[])), Some("alice"), Some(97), &[1000]),
            InputGroupStatus::MemberNotActive
        );
    }

    #[test]
    fn process_with_group_is_active() {
        let m = members(&["alice"]);
        assert_eq!(
            classify_input_group(Some((97, &m)), Some("alice"), Some(1000), &[1000, 97]),
            InputGroupStatus::Active
        );
    }

    #[test]
    fn no_denied_device_means_no_keyboard() {
        assert!(matches!(
            no_keyboard_error(None, InputGroupStatus::NotMember),
            HotkeyError::NoKeyboard
        ));
    }

    #[test]
    fn denied_device_maps_by_group_status() {
        let p = Some("/dev/input/event3");
        assert!(matches!(
            no_keyboard_error(p, InputGroupStatus::NotMember),
            HotkeyError::DeviceAccess(_)
        ));
        assert!(matches!(
            no_keyboard_error(p, InputGroupStatus::MemberNotActive),
            HotkeyError::InputGroupInactive(_)
        ));
        assert!(matches!(
            no_keyboard_error(p, InputGroupStatus::Active),
            HotkeyError::DeviceDenied(_)
        ));
    }

    #[test]
    fn notification_names_the_fix() {
        let (_, body) =
            startup_failure_notification(&HotkeyError::DeviceAccess("x".into())).unwrap();
        // The compositor binding leads; the group is a trade-off, not the
        // fix (Omarchy removed it on purpose).
        assert!(!body.contains("usermod"));
        assert!(body.contains("every program read your keystrokes"));
        let binding = body.find("voxtype record toggle").unwrap();
        assert!(binding < body.find("'input' group").unwrap());
        assert!(body.contains("[hotkey] enabled = false"));
        assert!(body.contains("--no-hotkey"));
        assert!(body.contains("silence this"));

        let (_, body) =
            startup_failure_notification(&HotkeyError::InputGroupInactive("x".into())).unwrap();
        assert!(body.contains("Log out completely"));
        assert!(!body.contains("usermod"));

        assert!(startup_failure_notification(&HotkeyError::UnknownKey("x".into())).is_none());
    }

    #[test]
    fn title_does_not_claim_a_compositor_hotkey_is_broken() {
        for err in [
            HotkeyError::DeviceAccess("x".into()),
            HotkeyError::InputGroupInactive("x".into()),
            HotkeyError::DeviceDenied("x".into()),
            HotkeyError::NoKeyboard,
        ] {
            let (title, _) = startup_failure_notification(&err).unwrap();
            assert_eq!(title, "Voxtype: built-in hotkey unavailable");
        }
    }

    #[test]
    fn notice_is_shown_once_until_the_hotkey_works_again() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir
            .path()
            .join("voxtype")
            .join("hotkey-unavailable-notified");
        let denied = HotkeyError::DeviceAccess("/dev/input/event3".into());

        // First failed start notifies and writes the marker.
        assert!(notify_once(&denied, &marker).is_some());
        assert!(marker.exists());
        // Later starts with the same cause stay quiet.
        assert!(notify_once(&denied, &marker).is_none());
        assert!(notify_once(&denied, &marker).is_none());

        // A successful start clears it, so a new breakage is reported.
        clear_notice(&marker);
        assert!(!marker.exists());
        assert!(notify_once(&denied, &marker).is_some());

        // Clearing an absent marker is fine.
        clear_notice(&marker);
        clear_notice(&marker);
    }

    #[test]
    fn a_different_cause_notifies_again() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("hotkey-unavailable-notified");
        assert!(notify_once(&HotkeyError::DeviceAccess("x".into()), &marker).is_some());
        // After `usermod`, the fix changes to "log out": worth one more notice.
        let relogin = HotkeyError::InputGroupInactive("x".into());
        assert!(notify_once(&relogin, &marker).is_some());
        assert!(notify_once(&relogin, &marker).is_none());
    }

    #[test]
    fn errors_without_a_notice_leave_the_marker_alone() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("hotkey-unavailable-notified");
        assert!(notify_once(&HotkeyError::UnknownKey("x".into()), &marker).is_none());
        assert!(!marker.exists());
    }
}
