//! Pure visual logic shared by both OSD frontends.
//!
//! Nothing in this module touches Wayland, GTK, wgpu, or Cairo. It exists
//! so the rendering math is identical across frontends and unit-testable
//! without a graphics context.

use crate::audio::levels::AudioFrame;

/// RGBA color, components in 0.0..=1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub fn with_alpha(mut self, a: f32) -> Self {
        self.a = a;
        self
    }
}

/// Color palette resolved from the active Omarchy theme (or the fallback).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Window background color (typically dark).
    pub background: Color,
    /// Waveform fill color (theme accent).
    pub accent: Color,
    /// Peak meter "safe" zone (-inf..-12 dBFS).
    pub meter_low: Color,
    /// Peak meter "warning" zone (-12..-3 dBFS).
    pub meter_mid: Color,
    /// Peak meter "danger" zone (-3..0 dBFS).
    pub meter_high: Color,
    /// Foreground / text color (used for held-peak tick, segment dividers).
    pub foreground: Color,
}

impl Palette {
    /// Fallback palette used until an Omarchy theme is parsed. Designed to
    /// look passable on a dark background.
    pub const fn fallback() -> Self {
        Self {
            background: Color::rgba(0.10, 0.10, 0.12, 0.85),
            accent: Color::rgb(0.40, 0.78, 1.00),
            meter_low: Color::rgb(0.30, 0.85, 0.45),
            meter_mid: Color::rgb(0.95, 0.80, 0.30),
            meter_high: Color::rgb(0.95, 0.35, 0.30),
            foreground: Color::rgb(0.92, 0.92, 0.95),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::fallback()
    }
}

/// Peak meter zone, used to color the lit segment of the bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MeterZone {
    Low,
    Mid,
    High,
}

impl MeterZone {
    /// Classify a peak level (dBFS) into a meter zone.
    ///
    /// Boundaries match the BRIEF: green to -12, yellow -12..-3, red -3..0.
    pub fn from_dbfs(peak_dbfs: f32) -> Self {
        if peak_dbfs >= -3.0 {
            MeterZone::High
        } else if peak_dbfs >= -12.0 {
            MeterZone::Mid
        } else {
            MeterZone::Low
        }
    }

    pub fn color(self, palette: &Palette) -> Color {
        match self {
            MeterZone::Low => palette.meter_low,
            MeterZone::Mid => palette.meter_mid,
            MeterZone::High => palette.meter_high,
        }
    }
}

/// Held-peak state for the peak meter's decaying tick.
///
/// Per BRIEF: held-peak rises instantly to the current peak and decays at
/// `peak_decay_db_per_sec` dB/sec while the live peak sits below it.
#[derive(Debug, Clone, Copy)]
pub struct PeakHold {
    /// Current held peak in dBFS. -inf-equivalent is represented as -120.0.
    pub held_dbfs: f32,
    /// Decay rate in dB per second.
    pub decay_db_per_sec: f32,
}

impl PeakHold {
    pub fn new(decay_db_per_sec: f32) -> Self {
        Self {
            held_dbfs: -120.0,
            decay_db_per_sec,
        }
    }

    /// Update the hold given the current peak and the time delta since the
    /// last update (seconds).
    pub fn update(&mut self, current_peak_dbfs: f32, dt_secs: f32) {
        update_peak_hold(
            current_peak_dbfs,
            &mut self.held_dbfs,
            self.decay_db_per_sec,
            dt_secs,
        );
    }
}

/// Free-function peak-hold update; matches the formula in BRIEF.md verbatim.
///
/// `held` snaps up to `current_peak` instantly when louder, otherwise
/// decays linearly at `decay_db_per_sec`. The held value floors at -120.0
/// so a quiet signal doesn't underflow toward -infinity.
pub fn update_peak_hold(current_peak: f32, held: &mut f32, decay_db_per_sec: f32, dt_secs: f32) {
    if current_peak > *held {
        *held = current_peak;
    } else {
        *held -= decay_db_per_sec * dt_secs;
        if *held < -120.0 {
            *held = -120.0;
        }
    }
}

/// One column of the waveform envelope: min/max amplitude in -1.0..=1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeColumn {
    pub min: f32,
    pub max: f32,
}

impl EnvelopeColumn {
    pub const SILENT: Self = Self { min: 0.0, max: 0.0 };
}

/// Project a fixed time window onto pixel columns, oldest on the left.
///
/// `right_offset` is the fractional frame position relative to the newest
/// sample. Interpolation lets the window scroll between audio deliveries.
/// Missing history is silence, so starting a recording does not stretch or
/// rescale the time axis. When zoomed out, retain every peak within a column.
pub fn project_envelope(
    frames: &[AudioFrame],
    n_columns: usize,
    window_frames: f64,
    right_offset: f64,
) -> Vec<EnvelopeColumn> {
    let mut out = vec![EnvelopeColumn::SILENT; n_columns];
    if frames.is_empty() || n_columns == 0 {
        return out;
    }
    let right = (frames.len() - 1) as f64 + right_offset;
    let step = window_frames.max(1.0) / n_columns.saturating_sub(1).max(1) as f64;
    let sample = |index: isize| {
        usize::try_from(index)
            .ok()
            .and_then(|i| frames.get(i))
            .map(|f| EnvelopeColumn {
                min: f.min,
                max: f.max,
            })
            .unwrap_or(EnvelopeColumn::SILENT)
    };
    let interpolate = |position: f64| {
        let index = position.floor() as isize;
        let fraction = (position - position.floor()) as f32;
        let a = sample(index);
        let b = sample(index + 1);
        EnvelopeColumn {
            min: a.min + (b.min - a.min) * fraction,
            max: a.max + (b.max - a.max) * fraction,
        }
    };
    for (col, out) in out.iter_mut().enumerate() {
        let position = right - (n_columns - 1 - col) as f64 * step;
        *out = interpolate(position);
        if step > 1.0 {
            let start = position - step;
            let edge = interpolate(start);
            out.min = out.min.min(edge.min);
            out.max = out.max.max(edge.max);
            let first = (start.ceil().max(0.0) as usize).min(frames.len());
            let end = ((position.floor() + 1.0).max(0.0) as usize).min(frames.len());
            for f in &frames[first..end] {
                out.min = out.min.min(f.min);
                out.max = out.max.max(f.max);
            }
        }
    }
    out
}

/// Map a dBFS peak to a normalized 0.0..=1.0 fill level for the meter.
///
/// `floor_dbfs` is the dBFS value that maps to 0.0 (typically -60 dBFS for
/// a usable visual range). 0 dBFS maps to 1.0.
pub fn peak_meter_fraction(peak_dbfs: f32, floor_dbfs: f32) -> f32 {
    if !peak_dbfs.is_finite() || peak_dbfs <= floor_dbfs {
        return 0.0;
    }
    let clipped = peak_dbfs.min(0.0);
    let span = -floor_dbfs;
    if span <= 0.0 {
        return 0.0;
    }
    ((clipped - floor_dbfs) / span).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seq: u32, min: f32, max: f32, peak_dbfs: f32) -> AudioFrame {
        AudioFrame {
            seq,
            min,
            max,
            peak_dbfs,
        }
    }

    #[test]
    fn meter_zone_boundaries() {
        assert_eq!(MeterZone::from_dbfs(-30.0), MeterZone::Low);
        assert_eq!(MeterZone::from_dbfs(-12.0), MeterZone::Mid);
        assert_eq!(MeterZone::from_dbfs(-6.0), MeterZone::Mid);
        assert_eq!(MeterZone::from_dbfs(-3.0), MeterZone::High);
        assert_eq!(MeterZone::from_dbfs(0.0), MeterZone::High);
    }

    #[test]
    fn peak_hold_snaps_up_instantly() {
        let mut hold = PeakHold::new(6.0);
        hold.update(-10.0, 0.01);
        assert!((hold.held_dbfs - -10.0).abs() < 1e-6);
        hold.update(-3.0, 0.01);
        assert!((hold.held_dbfs - -3.0).abs() < 1e-6);
    }

    #[test]
    fn peak_hold_decays_linearly() {
        let mut hold = PeakHold::new(6.0);
        hold.update(-3.0, 0.0);
        assert!((hold.held_dbfs - -3.0).abs() < 1e-6);
        // 1 second at 6 dB/sec = -9 dBFS
        hold.update(-30.0, 1.0);
        assert!((hold.held_dbfs - -9.0).abs() < 1e-3);
    }

    #[test]
    fn peak_hold_floor_at_minus_120() {
        let mut held = -10.0;
        update_peak_hold(-100.0, &mut held, 6.0, 1000.0); // huge dt
        assert_eq!(held, -120.0);
    }

    #[test]
    fn peak_meter_fraction_basic() {
        assert_eq!(peak_meter_fraction(-60.0, -60.0), 0.0);
        assert_eq!(peak_meter_fraction(0.0, -60.0), 1.0);
        let half = peak_meter_fraction(-30.0, -60.0);
        assert!((half - 0.5).abs() < 1e-3);
    }

    #[test]
    fn peak_meter_fraction_clamps_silence() {
        assert_eq!(peak_meter_fraction(-120.0, -60.0), 0.0);
        assert_eq!(peak_meter_fraction(f32::NEG_INFINITY, -60.0), 0.0);
    }

    #[test]
    fn envelope_partial_preserves_time_scale() {
        let frames = vec![frame(0, -0.1, 0.1, -20.0), frame(1, -0.2, 0.2, -14.0)];
        let cols = project_envelope(&frames, 5, 4.0, 0.0);
        assert_eq!(&cols[..3], &[EnvelopeColumn::SILENT; 3]);
        assert_eq!(cols[3].max, 0.1);
        assert_eq!(cols[4].max, 0.2);
    }

    #[test]
    fn envelope_interpolates_fractional_scroll() {
        let frames = vec![frame(0, -0.2, 0.4, -8.0), frame(1, -0.6, 0.8, -2.0)];
        let cols = project_envelope(&frames, 3, 1.0, -0.25);
        assert!((cols[2].max - 0.7).abs() < 1e-6);
        assert!((cols[2].min + 0.5).abs() < 1e-6);
        assert!((cols[1].max - 0.5).abs() < 1e-6);
    }

    #[test]
    fn envelope_preserves_transients_when_zoomed_out() {
        let mut frames = vec![frame(0, 0.0, 0.0, -120.0); 101];
        frames[47] = frame(47, -0.9, 1.0, 0.0);
        let cols = project_envelope(&frames, 5, 100.0, -0.5);
        assert!(cols.iter().any(|c| c.max == 1.0 && c.min == -0.9));
    }

    #[test]
    fn envelope_handles_empty_and_single_column() {
        assert_eq!(
            project_envelope(&[], 4, 300.0, 0.0),
            vec![EnvelopeColumn::SILENT; 4]
        );
        let frames = [frame(0, -0.1, 0.2, -14.0)];
        assert!(project_envelope(&frames, 0, 300.0, 0.0).is_empty());
        assert_eq!(project_envelope(&frames, 1, 300.0, 0.0)[0].max, 0.2);
    }
}
