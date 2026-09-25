//! Environment workarounds for GTK4 bugs the GTK OSD frontend runs into.
//!
//! Kept free of GTK types so they are unit-testable without the `osd-gtk4`
//! feature; `voxtype-osd-gtk4` feeds in the runtime GTK version and applies
//! the result before GDK opens the Wayland display.

/// Wayland interface GDK binds for dmabuf format feedback.
pub const LINUX_DMABUF_INTERFACE: &str = "zwp_linux_dmabuf_v1";

/// Whether this GTK release lacks the fix for GNOME/gtk#8366.
///
/// Affected releases `munmap()` the heap-allocated `DmabufFormats` struct
/// instead of the mmap'd format table whenever the compositor resends
/// linux-dmabuf feedback, which Hyprland does on output hotplug, lid close,
/// idle-lock output teardown and dock unplug. The next feedback `done` event
/// then frees through the unmapped page and SIGSEGVs (#580), or the allocator
/// crashes later on the corrupted heap (#656). `GDK_DISABLE=dmabuf` does not
/// help: GDK binds the interface and listens for feedback regardless.
///
/// Fixed in 4.22.5 and in the 4.23 development series at 4.23.3. Releases
/// before 4.22 are treated as affected; the OSD loses nothing by skipping the
/// protocol, so erring toward the workaround is cheap.
pub fn gtk_lacks_dmabuf_feedback_fix(major: u32, minor: u32, micro: u32) -> bool {
    match (major, minor) {
        (4, 22) => micro < 5,
        (4, 23) => micro < 3,
        (4, m) => m < 22,
        _ => false,
    }
}

/// Add `interface` to a `GDK_WAYLAND_DISABLE` value, keeping whatever the
/// user already disabled. GDK splits the list on `:;, \t`.
pub fn wayland_disable_with(existing: Option<&str>, interface: &str) -> String {
    let existing = existing.unwrap_or("").trim();
    let already = existing
        .split([':', ';', ',', ' ', '\t'])
        .any(|p| p == interface);
    if already {
        existing.to_string()
    } else if existing.is_empty() {
        interface.to_string()
    } else {
        format!("{existing},{interface}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gtk_releases_before_the_fix_are_affected() {
        assert!(gtk_lacks_dmabuf_feedback_fix(4, 22, 4));
        assert!(gtk_lacks_dmabuf_feedback_fix(4, 22, 0));
        assert!(gtk_lacks_dmabuf_feedback_fix(4, 23, 2));
        assert!(gtk_lacks_dmabuf_feedback_fix(4, 18, 6));
    }

    #[test]
    fn fixed_gtk_releases_are_left_alone() {
        assert!(!gtk_lacks_dmabuf_feedback_fix(4, 22, 5));
        assert!(!gtk_lacks_dmabuf_feedback_fix(4, 22, 9));
        assert!(!gtk_lacks_dmabuf_feedback_fix(4, 23, 3));
        assert!(!gtk_lacks_dmabuf_feedback_fix(4, 24, 0));
        assert!(!gtk_lacks_dmabuf_feedback_fix(5, 0, 0));
    }

    #[test]
    fn wayland_disable_is_set_when_unset() {
        assert_eq!(
            wayland_disable_with(None, LINUX_DMABUF_INTERFACE),
            "zwp_linux_dmabuf_v1"
        );
        assert_eq!(
            wayland_disable_with(Some("  "), LINUX_DMABUF_INTERFACE),
            "zwp_linux_dmabuf_v1"
        );
    }

    #[test]
    fn wayland_disable_keeps_the_users_entries() {
        assert_eq!(
            wayland_disable_with(
                Some("wp_fractional_scale_manager_v1"),
                LINUX_DMABUF_INTERFACE
            ),
            "wp_fractional_scale_manager_v1,zwp_linux_dmabuf_v1"
        );
    }

    #[test]
    fn wayland_disable_does_not_duplicate() {
        assert_eq!(
            wayland_disable_with(Some("foo:zwp_linux_dmabuf_v1"), LINUX_DMABUF_INTERFACE),
            "foo:zwp_linux_dmabuf_v1"
        );
        // A different interface that merely contains the name is not a match.
        assert_eq!(
            wayland_disable_with(Some("zwp_linux_dmabuf_v1_extra"), LINUX_DMABUF_INTERFACE),
            "zwp_linux_dmabuf_v1_extra,zwp_linux_dmabuf_v1"
        );
    }
}
