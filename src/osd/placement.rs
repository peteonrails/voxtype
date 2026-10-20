//! Where the OSD card sits on its output.
//!
//! Every frontend asks the compositor for a layer surface that spans the
//! full height of whichever output it lands on (the focused one, unless the
//! compositor is told otherwise), then places the card inside that surface
//! with these functions. Placement therefore only ever uses the geometry of
//! the output the OSD is actually on, in that output's logical pixels, and
//! never a monitor height looked up separately (which can belong to a
//! different output or be in different units).
//!
//! The Quickshell frontend mirrors this math in `quickshell/OsdSurface.qml`.

use crate::osd::config::OsdPosition;

/// `top_margin` used when the configured value is not a usable fraction.
pub const DEFAULT_TOP_MARGIN: f32 = 0.85;

/// Offset of the card's top edge from the top of the area the OSD may use
/// on its output, in the same units as `area_height` (logical pixels).
///
/// - Corner positions sit `margin_px` from the top or bottom edge.
/// - `bottom-center` puts the card's top edge at `top_margin` of the area's
///   height, like `swayosd-server --top-margin`.
/// - `top-center` uses `top_margin` the same way when it is in the upper
///   half. A value in the lower half (such as the 0.85 default) is mirrored,
///   so the card keeps the same distance from the top edge as
///   `bottom-center` keeps from the bottom one instead of landing near the
///   bottom of the screen.
///
/// The result is clamped so the card stays at least `margin_px` inside the
/// area, which means it can never be pushed off the output or under its
/// bottom edge. When the area is too short for that, the card is centered.
pub fn card_top(
    position: OsdPosition,
    area_height: f32,
    card_height: f32,
    margin_px: f32,
    top_margin: f32,
) -> f32 {
    let area_height = area_height.max(0.0);
    let card_height = card_height.max(0.0);
    let margin = margin_px.max(0.0);
    let fraction = if top_margin.is_finite() {
        top_margin.clamp(0.0, 1.0)
    } else {
        DEFAULT_TOP_MARGIN
    };

    let wanted = match position {
        OsdPosition::TopLeft | OsdPosition::TopRight => margin,
        OsdPosition::BottomLeft | OsdPosition::BottomRight => area_height - card_height - margin,
        OsdPosition::BottomCenter => area_height * fraction,
        OsdPosition::TopCenter if fraction <= 0.5 => area_height * fraction,
        OsdPosition::TopCenter => area_height * (1.0 - fraction) - card_height,
    };

    let lowest = area_height - card_height - margin;
    if lowest < margin {
        return ((area_height - card_height) / 2.0).max(0.0);
    }
    wanted.clamp(margin, lowest)
}

/// Offset of the card's left edge from the left of the area, for
/// frontends that position the card themselves rather than through
/// layer-shell anchors.
pub fn card_left(position: OsdPosition, area_width: f32, card_width: f32, margin_px: f32) -> f32 {
    let area_width = area_width.max(0.0);
    let card_width = card_width.max(0.0);
    let margin = margin_px.max(0.0);
    let rightmost = (area_width - card_width - margin).max(margin);
    match position {
        OsdPosition::TopLeft | OsdPosition::BottomLeft => margin,
        OsdPosition::TopRight | OsdPosition::BottomRight => rightmost,
        OsdPosition::TopCenter | OsdPosition::BottomCenter => {
            ((area_width - card_width) / 2.0).max(0.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The reporter's outputs, in logical pixels as Hyprland lays them out:
    // DP-2 3840x2160 at scale 1.333 (focused), DP-1 2560x1600 at scale 1.6,
    // placed at 2880,620. A 26 px top bar is reserved on both.
    const DP2_LOGICAL: (f32, f32) = (3840.0 / 1.333_333_4, 2160.0 / 1.333_333_4);
    const DP1_LOGICAL: (f32, f32) = (2560.0 / 1.6, 1600.0 / 1.6);
    const BAR: f32 = 26.0;

    fn bottom_gap(area_height: f32, top: f32, card_height: f32) -> f32 {
        area_height - top - card_height
    }

    #[test]
    fn bottom_center_uses_each_outputs_own_height() {
        let dp2 = DP2_LOGICAL.1 - BAR;
        let dp1 = DP1_LOGICAL.1 - BAR;
        let top2 = card_top(OsdPosition::BottomCenter, dp2, 48.0, 24.0, 0.85);
        let top1 = card_top(OsdPosition::BottomCenter, dp1, 48.0, 24.0, 0.85);
        assert!((top2 - dp2 * 0.85).abs() < 1e-3);
        assert!((top1 - dp1 * 0.85).abs() < 1e-3);
        // Both land in the bottom band of their own output, never past it.
        for (area, top) in [(dp2, top2), (dp1, top1)] {
            let gap = bottom_gap(area, top, 48.0);
            assert!(gap >= 24.0, "card must stay margin_px above the bottom");
            assert!(top > area / 2.0, "bottom-center must be in the lower half");
        }
    }

    #[test]
    fn using_the_other_outputs_height_is_what_went_wrong() {
        // The old GTK4 code turned top_margin into a pixel offset from the
        // first monitor GDK listed. DP-2's offset on DP-1 put the card off
        // the bottom; DP-1's offset on DP-2 put it near the middle. With
        // the area's own height, both are inside the bottom band.
        let dp2 = DP2_LOGICAL.1 - BAR;
        let dp1 = DP1_LOGICAL.1 - BAR;
        let wrong_on_dp1 = dp2 * 0.85;
        assert!(wrong_on_dp1 + 48.0 > dp1);
        let wrong_on_dp2 = dp1 * 0.85;
        assert!(wrong_on_dp2 < dp2 * 0.6);
        assert!(card_top(OsdPosition::BottomCenter, dp1, 48.0, 24.0, 0.85) + 48.0 <= dp1 - 24.0);
    }

    #[test]
    fn top_center_mirrors_a_lower_half_fraction() {
        let area = DP2_LOGICAL.1 - BAR;
        let bottom = card_top(OsdPosition::BottomCenter, area, 48.0, 24.0, 0.85);
        let top = card_top(OsdPosition::TopCenter, area, 48.0, 24.0, 0.85);
        assert!((top - bottom_gap(area, bottom, 48.0)).abs() < 1e-3);
        assert!(top < area / 2.0);
    }

    #[test]
    fn top_center_keeps_an_upper_half_fraction() {
        let top = card_top(OsdPosition::TopCenter, 1000.0, 48.0, 24.0, 0.1);
        assert!((top - 100.0).abs() < 1e-3);
    }

    #[test]
    fn corners_use_margin_px() {
        assert_eq!(
            card_top(OsdPosition::TopLeft, 1000.0, 48.0, 24.0, 0.85),
            24.0
        );
        assert_eq!(
            card_top(OsdPosition::TopRight, 1000.0, 48.0, 24.0, 0.85),
            24.0
        );
        assert_eq!(
            card_top(OsdPosition::BottomLeft, 1000.0, 48.0, 24.0, 0.85),
            928.0
        );
        assert_eq!(
            card_top(OsdPosition::BottomRight, 1000.0, 48.0, 24.0, 0.85),
            928.0
        );
    }

    #[test]
    fn extreme_top_margin_is_clamped_inside_the_output() {
        assert_eq!(
            card_top(OsdPosition::BottomCenter, 1000.0, 48.0, 24.0, 1.0),
            928.0
        );
        assert_eq!(
            card_top(OsdPosition::BottomCenter, 1000.0, 48.0, 24.0, 0.0),
            24.0
        );
        assert_eq!(
            card_top(OsdPosition::BottomCenter, 1000.0, 48.0, 24.0, 7.0),
            928.0
        );
        let nan = card_top(OsdPosition::BottomCenter, 1000.0, 48.0, 24.0, f32::NAN);
        assert!((nan - 850.0).abs() < 1e-3);
    }

    #[test]
    fn area_too_short_centers_the_card() {
        assert_eq!(
            card_top(OsdPosition::BottomCenter, 80.0, 48.0, 24.0, 0.85),
            16.0
        );
        assert_eq!(card_top(OsdPosition::TopLeft, 0.0, 48.0, 24.0, 0.85), 0.0);
    }

    #[test]
    fn horizontal_placement() {
        let w = DP1_LOGICAL.0;
        assert_eq!(card_left(OsdPosition::BottomLeft, w, 400.0, 24.0), 24.0);
        assert_eq!(card_left(OsdPosition::TopRight, w, 400.0, 24.0), w - 424.0);
        assert_eq!(
            card_left(OsdPosition::BottomCenter, w, 400.0, 24.0),
            (w - 400.0) / 2.0
        );
        assert_eq!(card_left(OsdPosition::TopRight, 300.0, 400.0, 24.0), 24.0);
    }
}
