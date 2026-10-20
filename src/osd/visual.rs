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

/// Quietest level the waveform shows, in dBFS at the reference gain.
/// A typical mic's noise floor sits below it, so silence reads as a flat
/// line while quiet speech (around -40..-30 dBFS) is already visible.
/// Normal speech (around -20 dBFS) lands near mid-height and only input
/// close to clipping reaches the top.
pub const WAVEFORM_FLOOR_DBFS: f32 = -48.0;

/// `waveform_gain` that leaves the display curve unshifted. It is the
/// default, so existing configs that never touched the key keep the
/// default look.
pub const WAVEFORM_REFERENCE_GAIN: f32 = 10.0;

/// Convert the `waveform_gain` config value into a dB shift of the display
/// curve: 10 is neutral and every doubling adds 6 dB. Non-positive or
/// non-finite values fall back to neutral.
pub fn waveform_gain_db(gain: f32) -> f32 {
    if gain.is_finite() && gain > 0.0 {
        20.0 * (gain / WAVEFORM_REFERENCE_GAIN).log10()
    } else {
        0.0
    }
}

/// Map a signed sample amplitude (-1.0..=1.0 of full scale) to a signed
/// display height in -1.0..=1.0 of the waveform's half-height.
///
/// The mapping is fixed and absolute: linear in decibels between
/// [`WAVEFORM_FLOOR_DBFS`] and 0 dBFS, shifted only by the configured
/// `gain_db` (see [`waveform_gain_db`]). There is no auto-gain or
/// normalization to recent input, so louder speech always draws taller
/// than normal speech, and since 0 dBFS (full scale) maps to the top at the
/// reference gain, normal speech never fills or overflows the card.
pub fn waveform_level(sample: f32, gain_db: f32) -> f32 {
    let magnitude = sample.abs();
    if magnitude.is_nan() || magnitude <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * magnitude.log10() + gain_db;
    let level = ((db - WAVEFORM_FLOOR_DBFS) / -WAVEFORM_FLOOR_DBFS).clamp(0.0, 1.0);
    level.copysign(sample)
}

/// [`waveform_level`] for a level given in dBFS, so the meter and the
/// waveform put the same loudness at the same relative height.
pub fn dbfs_level(dbfs: f32, gain_db: f32) -> f32 {
    if dbfs.is_nan() {
        return 0.0;
    }
    ((dbfs + gain_db - WAVEFORM_FLOOR_DBFS) / -WAVEFORM_FLOOR_DBFS).clamp(0.0, 1.0)
}

/// Inverse of [`dbfs_level`]: the dBFS value drawn at `level`.
pub fn level_dbfs(level: f32, gain_db: f32) -> f32 {
    WAVEFORM_FLOOR_DBFS * (1.0 - level.clamp(0.0, 1.0)) - gain_db
}

/// Time constant of the level meter's rise, in seconds.
pub const METER_ATTACK_SECS: f32 = 0.015;
/// Time constant of the level meter's fall, in seconds.
pub const METER_RELEASE_SECS: f32 = 0.3;

/// Level meter ballistics on the display scale (0.0..=1.0).
///
/// The meter is advanced once per rendered frame with the loudest level
/// that arrived since the previous one, so it moves at the display's
/// refresh rate instead of jumping with each socket delivery: it rises
/// with a short time constant and falls back with a longer one.
#[derive(Debug, Clone, Copy, Default)]
pub struct MeterBallistics {
    pub level: f32,
}

impl MeterBallistics {
    pub fn update(&mut self, target: f32, dt_secs: f32) -> f32 {
        let target = if target.is_finite() {
            target.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let tau = if target > self.level {
            METER_ATTACK_SECS
        } else {
            METER_RELEASE_SECS
        };
        let dt = dt_secs.clamp(0.0, 0.25);
        self.level += (target - self.level) * (1.0 - (-dt / tau).exp());
        self.level
    }
}

/// The OSD's level meter: what the frontends draw next to the waveform.
///
/// Socket deliveries only feed it ([`LevelMeter::push`]); the frontend
/// advances it once per drawn frame ([`LevelMeter::advance`]), so it moves
/// at the display's refresh rate with [`MeterBallistics`] instead of
/// jumping with each capture batch. Levels come out on the waveform's
/// scale ([`dbfs_level`]), so the same loudness reaches the same relative
/// height in both. The held-peak tick falls at `peak_decay_db_per_sec`.
#[derive(Debug, Clone, Copy)]
pub struct LevelMeter {
    ballistics: MeterBallistics,
    hold: PeakHold,
    pending_dbfs: Option<f32>,
    target_dbfs: f32,
}

impl LevelMeter {
    pub fn new(decay_db_per_sec: f32) -> Self {
        Self {
            ballistics: MeterBallistics::default(),
            hold: PeakHold::new(decay_db_per_sec),
            pending_dbfs: None,
            target_dbfs: -120.0,
        }
    }

    /// Record a delivered frame's peak.
    pub fn push(&mut self, peak_dbfs: f32) {
        if peak_dbfs.is_nan() {
            return;
        }
        self.pending_dbfs = Some(self.pending_dbfs.map_or(peak_dbfs, |p| p.max(peak_dbfs)));
    }

    /// Advance by one drawn frame. Returns `(level, held_level)` on the
    /// waveform's display scale. Without new deliveries the meter keeps
    /// heading for the last one, so it doesn't sag between capture batches.
    pub fn advance(&mut self, dt_secs: f32, gain_db: f32) -> (f32, f32) {
        if let Some(p) = self.pending_dbfs.take() {
            self.target_dbfs = p;
        }
        let level = self
            .ballistics
            .update(dbfs_level(self.target_dbfs, gain_db), dt_secs);
        self.hold.update(self.target_dbfs, dt_secs.clamp(0.0, 0.25));
        (level, dbfs_level(self.hold.held_dbfs, gain_db))
    }

    /// Forget the previous recording.
    pub fn reset(&mut self) {
        *self = Self::new(self.hold.decay_db_per_sec);
    }
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
    fn waveform_level_is_bounded_for_any_input() {
        for gain in [0.5, 1.0, 10.0, 40.0, 1000.0] {
            let g = waveform_gain_db(gain);
            for s in [-2.0, -1.0, -0.5, -0.01, 0.0, 0.001, 0.3, 1.0, 5.0] {
                let level = waveform_level(s, g);
                assert!((-1.0..=1.0).contains(&level), "{s} at gain {gain}");
                assert!(level == 0.0 || level.signum() == s.signum());
            }
        }
        assert_eq!(waveform_level(f32::NAN, 0.0), 0.0);
        assert_eq!(waveform_level(f32::INFINITY, 0.0), 1.0);
    }

    #[test]
    fn waveform_level_leaves_headroom_for_speech_at_default_gain() {
        let g = waveform_gain_db(10.0);
        let db = |dbfs: f32| waveform_level(10f32.powf(dbfs / 20.0), g);
        // Old mapping (sample * 10, clamped) pinned everything above
        // -20 dBFS to the edge. Now normal speech sits around mid-height,
        // louder speech draws taller, and only near-clipping input reaches
        // the top.
        assert!((0.45..=0.65).contains(&db(-20.0)));
        assert!(db(-12.0) < 0.8);
        assert!(db(-6.0) < 0.9);
        assert!(db(-3.0) > 0.9);
        // Quiet speech is still clearly visible; the noise floor is not.
        assert!(db(-40.0) > 0.15);
        assert_eq!(db(-60.0), 0.0);
        assert!((db(0.0) - 1.0).abs() < 1e-6);
        // Monotonic.
        assert!(db(-30.0) < db(-20.0) && db(-20.0) < db(-10.0));
    }

    #[test]
    fn waveform_gain_shifts_the_curve_in_db() {
        assert_eq!(waveform_gain_db(10.0), 0.0);
        assert!((waveform_gain_db(20.0) - 6.0206).abs() < 1e-3);
        assert!((waveform_gain_db(5.0) + 6.0206).abs() < 1e-3);
        assert_eq!(waveform_gain_db(0.0), 0.0);
        assert_eq!(waveform_gain_db(-3.0), 0.0);
        assert_eq!(waveform_gain_db(f32::NAN), 0.0);
        let quiet = 10f32.powf(-40.0 / 20.0);
        assert!(waveform_level(quiet, waveform_gain_db(20.0)) > waveform_level(quiet, 0.0));
    }

    #[test]
    fn meter_and_waveform_share_one_scale() {
        for dbfs in [-60.0, -40.0, -20.0, -6.0, -1.0, 0.0] {
            let amplitude = 10f32.powf(dbfs / 20.0);
            for gain in [5.0, 10.0, 20.0] {
                let g = waveform_gain_db(gain);
                assert!((dbfs_level(dbfs, g) - waveform_level(amplitude, g)).abs() < 1e-5);
            }
        }
        assert_eq!(dbfs_level(f32::NEG_INFINITY, 0.0), 0.0);
        assert_eq!(dbfs_level(f32::NAN, 0.0), 0.0);
        for level in [0.1, 0.5, 0.9] {
            assert!((dbfs_level(level_dbfs(level, 3.0), 3.0) - level).abs() < 1e-5);
        }
    }

    #[test]
    fn meter_attack_is_fast_and_release_is_slow() {
        let mut m = MeterBallistics::default();
        // 30 ms of loud input at 144 Hz reaches most of the way up.
        for _ in 0..4 {
            m.update(0.8, 1.0 / 144.0);
        }
        assert!(m.level > 0.6 && m.level <= 0.8, "{}", m.level);
        // 100 ms after the input stops it has fallen only part of the way.
        for _ in 0..14 {
            m.update(0.0, 1.0 / 144.0);
        }
        assert!(m.level > 0.4, "{}", m.level);
        // After a second it is back near the floor.
        for _ in 0..144 {
            m.update(0.0, 1.0 / 144.0);
        }
        assert!(m.level < 0.05, "{}", m.level);
    }

    #[test]
    fn meter_motion_does_not_depend_on_frame_rate() {
        let mut fast = MeterBallistics { level: 0.8 };
        let mut slow = MeterBallistics { level: 0.8 };
        for _ in 0..144 {
            fast.update(0.1, 1.0 / 144.0);
        }
        for _ in 0..60 {
            slow.update(0.1, 1.0 / 60.0);
        }
        assert!((fast.level - slow.level).abs() < 1e-3);
    }

    #[test]
    fn level_meter_moves_per_drawn_frame_not_per_delivery() {
        let g = waveform_gain_db(10.0);
        let mut m = LevelMeter::new(6.0);
        // A 40 ms capture batch of -20 dBFS frames arrives at once.
        for _ in 0..4 {
            m.push(-20.0);
        }
        let (first, _) = m.advance(1.0 / 144.0, g);
        assert!(first > 0.0 && first < dbfs_level(-20.0, g) * 0.5);
        // No more deliveries for a while: the meter keeps rising smoothly
        // toward -20 dBFS instead of dropping between batches.
        let mut prev = first;
        for _ in 0..10 {
            let (level, _) = m.advance(1.0 / 144.0, g);
            assert!(level >= prev);
            prev = level;
        }
        assert!((prev - dbfs_level(-20.0, g)).abs() < 0.02);
    }

    #[test]
    fn level_meter_holds_the_loudest_peak_and_decays_it() {
        let g = 0.0;
        let mut m = LevelMeter::new(6.0);
        m.push(-30.0);
        m.push(-6.0);
        m.push(-30.0);
        let (_, held) = m.advance(0.01, g);
        assert!((held - dbfs_level(-6.0, g)).abs() < 0.01);
        m.push(-40.0);
        for _ in 0..100 {
            m.advance(0.01, g);
        }
        let (_, held) = m.advance(0.0, g);
        // One second at 6 dB/s.
        assert!((held - dbfs_level(-12.0, g)).abs() < 0.01, "{held}");
        m.reset();
        assert_eq!(m.advance(0.01, g), (0.0, 0.0));
    }

    #[test]
    fn meter_ignores_bad_input() {
        let mut m = MeterBallistics { level: 0.5 };
        m.update(f32::NAN, 0.01);
        assert!(m.level.is_finite() && m.level < 0.5);
        m.update(5.0, 1.0);
        assert!(m.level <= 1.0);
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
