//! Why the evdev listener could not open any keyboard.
//!
//! The listener skips devices it gets EACCES on, so without this the only
//! trace of a missing `input` group is "No keyboard device found" in the
//! journal while the daemon reports itself running. Telling "not in the
//! group" apart from "in the group, but this session predates it" matters:
//! the second user already ran `usermod` and only needs to log out.

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
/// built-in hotkey is not working and how to fix it. `None` for errors that
/// already stop the daemon or are configuration mistakes reported elsewhere.
pub fn startup_failure_notification(err: &HotkeyError) -> Option<(&'static str, String)> {
    const ALTERNATIVE: &str = "Or bind `voxtype record toggle` in your compositor \
                               and set [hotkey] enabled = false.";
    let body = match err {
        HotkeyError::DeviceAccess(_) => format!(
            "Permission denied reading keyboards in /dev/input. \
             Run `sudo usermod -aG input $USER`, then log out and back in. {}",
            ALTERNATIVE
        ),
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
    Some(("Voxtype: hotkey not working", body))
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
        assert!(body.contains("sudo usermod -aG input $USER"));
        assert!(body.contains("log out"));
        assert!(body.contains("voxtype record toggle"));
        assert!(body.contains("[hotkey] enabled = false"));

        let (_, body) =
            startup_failure_notification(&HotkeyError::InputGroupInactive("x".into())).unwrap();
        assert!(body.contains("Log out completely"));
        assert!(!body.contains("usermod"));

        assert!(startup_failure_notification(&HotkeyError::UnknownKey("x".into())).is_none());
    }
}
