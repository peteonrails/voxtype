pragma Singleton

// Voxtype Quickshell theme singleton.
//
// Mirrors the fallback palette from src/osd/visual.rs (Palette::fallback)
// and the OSD's sizing defaults from src/osd/config.rs (OsdConfig::default).
// Consumers can override any of these properties at runtime to match the
// active Omarchy theme or a user-provided palette:
//
//   import "voxtype-shared" as VT
//   VT.Theme.accentColor = "#6E89C2"
//
// Wave 2 will add a loader that reads
// `~/.config/omarchy/current/theme/colors.toml` and maps `accent`,
// `background`, `color1`/`2`/`3` onto these properties.

import QtQuick

QtObject {
    id: theme

    /// Window / card background. Translucent dark, alpha matches the
    /// Rust fallback (0.85) so the OSD reads as a glassy overlay rather
    /// than a solid panel.
    property color bgColor: Qt.rgba(0.10, 0.10, 0.12, 0.85)

    /// Theme accent. Used for the waveform fill and any "primary action"
    /// indicators. Default mirrors Palette::fallback().accent.
    property color accentColor: Qt.rgba(0.40, 0.78, 1.00, 1.0)

    /// Idle-state indicator color (when the OSD is visible but the
    /// daemon isn't actively recording). Matches the existing POC.
    property color idleColor: "#abb2bf"

    /// Recording-state indicator color. Voxtype's signature
    /// red/orange used for "we are capturing your voice right now."
    property color recordingColor: "#e06c75"

    /// Streaming-state indicator color (live partial-token output).
    property color streamingColor: "#61afef"

    /// Transcribing-state indicator color (model is processing the
    /// final audio buffer).
    property color transcribingColor: "#e5c07b"

    /// Foreground text color. Matches the OSD card label.
    property color textColor: "#dcdfe4"

    /// Waveform body color. Defaults to accent so a single tweak
    /// re-themes both the indicator and the meter.
    property color waveformColor: theme.accentColor

    /// Waveform held-peak tick color. Brighter than the body so the
    /// instantaneous peak is visible against the envelope.
    property color waveformPeakColor: "#FCFBF8"

    /// Peak meter "safe" zone (-inf..-12 dBFS). Mirrors the GTK4/native
    /// OSD's MeterZone::Low.
    property color meterLowColor: Qt.rgba(0.30, 0.85, 0.45, 1.0)

    /// Peak meter "warning" zone (-12..-3 dBFS). Mirrors MeterZone::Mid.
    property color meterMidColor: Qt.rgba(0.95, 0.80, 0.30, 1.0)

    /// Peak meter "danger" zone (-3..0 dBFS). Mirrors MeterZone::High.
    property color meterHighColor: Qt.rgba(0.95, 0.35, 0.30, 1.0)

    /// Corner radius for cards/panels. 12px matches the radius used by
    /// swayosd so voxtype's OSD blends with the rest of an Omarchy
    /// user's notification stack.
    property int cornerRadius: 12

    /// Inner padding for OSD cards (px between border and content).
    property int padding: 14

    /// Distance from the screen edge (px). Mirrors OsdConfig::margin_px.
    property int marginPx: 24

    /// Default OSD surface width (px). Mirrors OsdConfig::width_px so
    /// the QML frontend lands at the same size as the GTK4/wgpu ones.
    property int defaultWidthPx: 400

    /// Default OSD surface height (px). Mirrors OsdConfig::height_px.
    property int defaultHeightPx: 48

    /// Default background opacity. Mirrors OsdConfig::opacity.
    property real defaultOpacity: 0.95

    /// Visible waveform window in seconds. Mirrors
    /// OsdConfig::waveform_window_secs.
    property real waveformWindowSecs: 3.0

    /// Held-peak decay rate (dB/sec). Mirrors
    /// OsdConfig::peak_decay_db_per_sec; the held tick on the peak
    /// meter snaps up to the current peak then linearly decays at this
    /// rate while the signal sits below it.
    property real peakDecayDbPerSec: 6.0

    /// Visual gain applied to per-frame peak before drawing the
    /// waveform envelope. Mic-level voice peaks around 0.1..0.3 of
    /// full-scale, so the gain scales the envelope up to fill the
    /// available canvas height. Mirrors OsdConfig::waveform_gain.
    property real waveformGain: 10.0

    /// dBFS floor for the peak meter. Anything quieter renders as an
    /// empty bar. -60 dB matches the GTK4/native frontends.
    property real meterFloorDbfs: -60.0
}
