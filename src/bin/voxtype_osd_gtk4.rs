//! `voxtype-osd-gtk4` — GTK4 + gtk4-layer-shell on-screen mic visualizer
//! for the Voxtype daemon.
//!
//! Renders either the classic waveform + peak meter or a GNOME-style pill
//! with voice-history bars. Audio frames arrive on
//! the daemon's audio Unix socket via [`voxtype::osd::ipc::run_ipc_loop`],
//! decoded into [`AudioFrame`]s by a tokio runtime on a worker thread, and
//! pushed into a shared [`FrameRing`] + [`PeakHold`]. The GTK side polls a
//! ~60 Hz `glib::timeout_add_local` callback that redraws the
//! `DrawingArea` whenever new frames have arrived.
//!
//! When the IPC socket is silent for `idle_timeout_secs` (Idle proxy) the
//! window is hidden so the binary does no rendering work and consumes
//! effectively zero CPU. It reappears when frames resume.
//!
//! Run with `RUST_LOG=debug` for verbose logs.

use std::cell::Cell;
use std::f64::consts::PI;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cairo::{Context, RectangleInt, Region};
use clap::Parser;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, DrawingArea};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use voxtype::audio::levels::{AudioFrame, FRAME_HZ};
use voxtype::config::Config as VoxtypeConfig;
use voxtype::osd::config::{Gtk4Variant, OsdConfig, OsdPosition};
use voxtype::osd::ipc::{resolve_socket_path, run_ipc_loop, FrameRing, DEFAULT_RING_DEPTH};
use voxtype::osd::theme::ThemeWatcher;
use voxtype::osd::visual::{
    peak_meter_fraction, project_bar_levels, project_envelope, MeterZone, Palette, PeakHold,
};

/// Load the `[osd]` section from the voxtype config file, falling back to
/// `OsdConfig::default()` on any error (file missing, unreadable, parse
/// failure, or `[osd]` section absent).
///
/// We deliberately ignore parse errors instead of returning them: the OSD
/// is a side car, and a malformed config shouldn't prevent it from running
/// with sensible defaults — the user will see the daemon complain about
/// the same file separately.
fn load_osd_config_from_file(explicit: Option<&std::path::Path>) -> (OsdConfig, GeometryOverrides) {
    let path = explicit
        .map(std::path::Path::to_path_buf)
        .or_else(VoxtypeConfig::default_path);
    let Some(path) = path else {
        return (OsdConfig::default(), GeometryOverrides::default());
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return (OsdConfig::default(), GeometryOverrides::default()),
    };

    #[derive(serde::Deserialize, Default)]
    struct PartialConfig {
        #[serde(default)]
        osd: Option<OsdConfig>,
    }

    let document = match toml::from_str::<toml::Value>(&content) {
        Ok(document) => document,
        Err(_) => return (OsdConfig::default(), GeometryOverrides::default()),
    };
    let overrides = document
        .get("osd")
        .and_then(toml::Value::as_table)
        .map(GeometryOverrides::from_table)
        .unwrap_or_default();
    let config = document
        .try_into::<PartialConfig>()
        .ok()
        .and_then(|partial| partial.osd)
        .unwrap_or_default();
    (config, overrides)
}

/// Application id for the GTK4 frontend.
const APP_ID: &str = "io.voxtype.OsdGtk4";

/// Render tick period (~60 Hz). The redraw is gated on whether new frames
/// have arrived since the last paint, so this is a cheap upper bound.
const RENDER_TICK_MS: u32 = 16;

/// How long we wait without frames before treating the daemon as idle and
/// hiding the surface. Matches the BRIEF's "Idle: surface destroyed" rule.
const IDLE_TIMEOUT_SECS: f32 = 0.15;

/// Number of segments in the vertical peak meter.
const METER_SEGMENTS: usize = 10;

/// dBFS floor for the peak meter (maps to "empty bar").
const METER_FLOOR_DBFS: f32 = -60.0;

const PILL_HORIZONTAL_PADDING: f64 = 18.0;

/// Geometry fields explicitly present in `[osd]`.
///
/// Serde fills omitted fields with `OsdConfig::default()`, so this mask lets
/// variant defaults fill only omitted values without overriding user choices.
#[derive(Default)]
struct GeometryOverrides {
    width_px: bool,
    height_px: bool,
    margin_px: bool,
    opacity: bool,
}

impl GeometryOverrides {
    fn from_table(table: &toml::map::Map<String, toml::Value>) -> Self {
        Self {
            width_px: table.contains_key("width_px"),
            height_px: table.contains_key("height_px"),
            margin_px: table.contains_key("margin_px"),
            opacity: table.contains_key("opacity"),
        }
    }
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "voxtype-osd-gtk4",
    version,
    about = "Voxtype on-screen mic visualizer (GTK4 + gtk4-layer-shell)"
)]
struct Args {
    /// Path to the voxtype config file. Defaults to
    /// `~/.config/voxtype/config.toml`. Only the `[osd]` section is read.
    #[arg(long, env = "VOXTYPE_CONFIG")]
    config: Option<PathBuf>,

    /// Path to the audio-frame Unix socket. Defaults to
    /// `$XDG_RUNTIME_DIR/voxtype/audio.sock`.
    #[arg(long, env = "VOXTYPE_OSD_SOCKET")]
    socket: Option<PathBuf>,

    /// Seconds to wait between reconnect attempts when the daemon is down.
    #[arg(long, default_value = "1.0", env = "VOXTYPE_OSD_RECONNECT_SECS")]
    reconnect_secs: f32,

    /// Print one debug line per N frames received (0 = quiet).
    #[arg(long, default_value = "0", env = "VOXTYPE_OSD_LOG_EVERY")]
    log_every: u32,

    /// GTK4 visual variant.
    #[arg(
        long,
        env = "VOXTYPE_OSD_GTK4_VARIANT",
        value_parser = ["classic", "pill"]
    )]
    variant: Option<String>,

    /// Held-peak decay rate in dB/sec.
    #[arg(long, default_value = "6.0", env = "VOXTYPE_OSD_PEAK_DECAY")]
    peak_decay_db_per_sec: f32,

    /// Surface width in physical pixels.
    #[arg(long, env = "VOXTYPE_OSD_WIDTH")]
    width_px: Option<u32>,

    /// Surface height in physical pixels.
    #[arg(long, env = "VOXTYPE_OSD_HEIGHT")]
    height_px: Option<u32>,

    /// Margin from the screen edge in physical pixels.
    #[arg(long, env = "VOXTYPE_OSD_MARGIN")]
    margin_px: Option<u32>,

    /// Background opacity 0.0..=1.0.
    #[arg(long, env = "VOXTYPE_OSD_OPACITY")]
    opacity: Option<f32>,

    /// Visual gain applied to audio samples before drawing the waveform.
    /// Higher = waveform fills more of the vertical for quiet inputs.
    /// Reduce for hot mics (e.g. 4.0); raise for quiet sources (e.g. 14.0).
    #[arg(long, env = "VOXTYPE_OSD_GAIN")]
    waveform_gain: Option<f32>,
}

/// State shared between the IPC worker and the GTK redraw timer.
struct SharedState {
    ring: Mutex<FrameRing>,
    peak: Mutex<PeakHold>,
    last_seq: Mutex<u64>,
    last_frame_at: Mutex<Instant>,
}

impl SharedState {
    fn new(decay_db_per_sec: f32) -> Self {
        Self {
            ring: Mutex::new(FrameRing::new(DEFAULT_RING_DEPTH)),
            peak: Mutex::new(PeakHold::new(decay_db_per_sec)),
            last_seq: Mutex::new(0),
            last_frame_at: Mutex::new(Instant::now() - Duration::from_secs(3600)),
        }
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let socket_path = resolve_socket_path(args.socket.clone());

    // Layer config: defaults < config file [osd] < CLI/env overrides.
    let (mut osd_cfg, geometry_overrides) = load_osd_config_from_file(args.config.as_deref());
    let variant = args
        .variant
        .as_deref()
        .and_then(Gtk4Variant::parse_str)
        .unwrap_or(osd_cfg.gtk4_variant);
    apply_variant_defaults(&mut osd_cfg, variant, &geometry_overrides);
    if let Some(w) = args.width_px {
        osd_cfg.width_px = w;
    }
    if let Some(h) = args.height_px {
        osd_cfg.height_px = h;
    }
    if let Some(m) = args.margin_px {
        osd_cfg.margin_px = m;
    }
    if let Some(o) = args.opacity {
        osd_cfg.opacity = o.clamp(0.0, 1.0);
    }
    if let Some(g) = args.waveform_gain {
        osd_cfg.waveform_gain = g;
    }
    // peak_decay_db_per_sec has a clap default value, so this always
    // overrides whatever the file said. That's intentional: if the user
    // passes the flag, honor it; if they don't, the clap default kicks in.
    osd_cfg.peak_decay_db_per_sec = args.peak_decay_db_per_sec;

    tracing::info!(
        "voxtype-osd-gtk4 starting; variant={:?} socket={:?} size={}x{} margin={} pos={:?}",
        variant,
        socket_path,
        osd_cfg.width_px,
        osd_cfg.height_px,
        osd_cfg.margin_px,
        osd_cfg.position,
    );

    let theme = ThemeWatcher::new();
    let palette = theme.palette();

    let state = Arc::new(SharedState::new(osd_cfg.peak_decay_db_per_sec));

    // Spawn the tokio IPC worker on a side thread.
    spawn_ipc_worker(
        state.clone(),
        socket_path,
        args.reconnect_secs,
        args.log_every,
        variant == Gtk4Variant::Pill,
    );

    // GTK application owns the main thread.
    let app = Application::builder().application_id(APP_ID).build();

    let cfg = osd_cfg.clone();
    let state_for_activate = state.clone();
    app.connect_activate(move |app| {
        build_window(app, &cfg, palette, variant, state_for_activate.clone());
    });

    // GTK's run() consumes argv; we've already parsed via clap, so feed
    // it an empty vector to keep it from re-parsing.
    let exit = app.run_with_args::<&str>(&[]);
    let code: u8 = exit.into();
    if code != 0 {
        anyhow::bail!("GTK application exited with status {}", code);
    }
    Ok(())
}

fn apply_variant_defaults(
    config: &mut OsdConfig,
    variant: Gtk4Variant,
    overrides: &GeometryOverrides,
) {
    let defaults = variant.defaults();
    if !overrides.width_px {
        config.width_px = defaults.width_px;
    }
    if !overrides.height_px {
        config.height_px = defaults.height_px;
    }
    if !overrides.margin_px {
        config.margin_px = defaults.margin_px;
    }
    if !overrides.opacity {
        config.opacity = defaults.opacity;
    }
}

/// Spawn the tokio runtime + IPC loop on a dedicated thread.
fn spawn_ipc_worker(
    state: Arc<SharedState>,
    socket_path: PathBuf,
    reconnect_secs: f32,
    log_every: u32,
    reset_history_after_idle: bool,
) {
    std::thread::Builder::new()
        .name("voxtype-osd-ipc".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!("Failed to build tokio runtime: {e}");
                    return;
                }
            };

            let dt_per_frame = 1.0 / FRAME_HZ as f32;
            let mut total: u64 = 0;
            let mut last_log = Instant::now();

            let on_frame = move |frame: AudioFrame| {
                let now = Instant::now();
                let new_recording = state
                    .last_frame_at
                    .lock()
                    .map(|mut last_frame_at| {
                        let was_idle = last_frame_at.elapsed().as_secs_f32() > IDLE_TIMEOUT_SECS;
                        *last_frame_at = now;
                        was_idle
                    })
                    .unwrap_or(false);
                if let Ok(mut r) = state.ring.lock() {
                    if reset_history_after_idle && new_recording {
                        r.clear();
                    }
                    r.push(frame);
                }
                if let Ok(mut p) = state.peak.lock() {
                    p.update(frame.peak_dbfs, dt_per_frame);
                }
                if let Ok(mut s) = state.last_seq.lock() {
                    *s = s.wrapping_add(1);
                }

                total += 1;
                if log_every > 0 && total.is_multiple_of(u64::from(log_every)) {
                    let elapsed = last_log.elapsed().as_secs_f32();
                    let rate = if elapsed > 0.0 {
                        log_every as f32 / elapsed
                    } else {
                        0.0
                    };
                    tracing::debug!(
                        target: "osd::frames",
                        frontend = "gtk4",
                        seq = frame.seq,
                        peak_dbfs = frame.peak_dbfs,
                        min = frame.min,
                        max = frame.max,
                        rate_hz = rate,
                        "frame batch"
                    );
                    last_log = Instant::now();
                }
            };

            rt.block_on(run_ipc_loop(socket_path, reconnect_secs, on_frame));
        })
        .expect("spawn ipc worker thread");
}

/// Best-effort monitor height in physical pixels for translating the
/// fractional `top_margin` config into a layer-shell pixel offset.
///
/// Tries the GDK display's currently-focused monitor first (which is the
/// monitor the user is most likely looking at, and the one swayosd targets
/// via `--monitor`). If that fails — display unavailable, no monitors
/// enumerated — returns None and the caller falls back to a conservative
/// default.
fn focused_monitor_height_px() -> Option<i32> {
    use gtk4::gdk;
    let display = gdk::Display::default()?;
    let monitors = display.monitors();
    // `monitors` is a GListModel; pull the first item with a non-zero
    // height. On multi-monitor setups this picks whichever the compositor
    // ordered first, which lines up with the layer-shell default in
    // practice.
    for i in 0..monitors.n_items() {
        if let Some(obj) = monitors.item(i) {
            if let Ok(monitor) = obj.downcast::<gdk::Monitor>() {
                let h = monitor.geometry().height();
                if h > 0 {
                    return Some(h);
                }
            }
        }
    }
    None
}

fn edge_anchors(position: OsdPosition) -> (bool, bool, bool, bool) {
    match position {
        OsdPosition::BottomCenter => (false, true, false, false),
        OsdPosition::BottomLeft => (false, true, true, false),
        OsdPosition::BottomRight => (false, true, false, true),
        OsdPosition::TopCenter => (true, false, false, false),
        OsdPosition::TopLeft => (true, false, true, false),
        OsdPosition::TopRight => (true, false, false, true),
    }
}

/// Build the GTK window, attach layer-shell config, mount the DrawingArea,
/// and start the redraw tick.
fn build_window(
    app: &Application,
    cfg: &OsdConfig,
    palette: Palette,
    variant: Gtk4Variant,
    state: Arc<SharedState>,
) {
    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(cfg.width_px as i32)
        .default_height(cfg.height_px as i32)
        .resizable(false)
        .decorated(false)
        .build();
    if variant == Gtk4Variant::Pill {
        install_transparent_window_style(&window);
    }

    // Layer-shell setup: top layer, no keyboard, anchored per config.
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_namespace(Some("voxtype-osd"));

    // Centered positions use swayosd-style fractional placement:
    // anchor only to Edge::Top with a margin of `top_margin × monitor_height`
    // and let the layer shell center horizontally. Matches what users
    // already see for volume/brightness/media-key feedback so the voxtype
    // OSD lands in the familiar band.
    //
    // Corner positions still use the absolute `margin_px` model — fractional
    // doesn't make sense there.
    let centered = matches!(
        cfg.position,
        OsdPosition::BottomCenter | OsdPosition::TopCenter
    );

    if centered && variant == Gtk4Variant::Classic {
        // Resolve monitor height to translate the fractional offset into
        // pixels. Falls back to a conservative 1080 if the display can't be
        // queried (extremely rare on Wayland-only systems where layer-shell
        // is supported at all).
        let monitor_height = focused_monitor_height_px().unwrap_or(1080);
        let top_px = (cfg.top_margin.clamp(0.0, 1.0) * monitor_height as f32) as i32;

        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Bottom, false);
        // Don't anchor Left/Right -> layer shell auto-centers horizontally.
        window.set_anchor(Edge::Left, false);
        window.set_anchor(Edge::Right, false);
        window.set_margin(Edge::Top, top_px);
    } else {
        // Pill positions use direct edge anchoring; classic corner positions
        // retain the same uniform margin behavior.
        let (anchor_top, anchor_bottom, anchor_left, anchor_right) = edge_anchors(cfg.position);
        window.set_anchor(Edge::Top, anchor_top);
        window.set_anchor(Edge::Bottom, anchor_bottom);
        window.set_anchor(Edge::Left, anchor_left);
        window.set_anchor(Edge::Right, anchor_right);

        let m = cfg.margin_px as i32;
        if anchor_top {
            window.set_margin(Edge::Top, m);
        }
        if anchor_bottom {
            window.set_margin(Edge::Bottom, m);
        }
        if anchor_left {
            window.set_margin(Edge::Left, m);
        }
        if anchor_right {
            window.set_margin(Edge::Right, m);
        }
    }

    // Don't reserve space on the output: the OSD floats over windows.
    window.set_exclusive_zone(0);

    // The drawing area fills the window.
    let drawing_area = DrawingArea::new();
    drawing_area.set_content_width(cfg.width_px as i32);
    drawing_area.set_content_height(cfg.height_px as i32);
    let state_for_draw = state.clone();
    let gain = cfg.waveform_gain as f64;
    let opacity = cfg.opacity.clamp(0.0, 1.0);
    drawing_area.set_draw_func(move |_area, cr, w, h| match variant {
        Gtk4Variant::Classic => draw_classic(cr, w, h, &palette, &state_for_draw, gain, opacity),
        Gtk4Variant::Pill => draw_pill(cr, w, h, &state_for_draw, gain, opacity),
    });
    window.set_child(Some(&drawing_area));

    // Click-through: install an empty input region on the window's surface
    // once it's realized. Until then `realize` hasn't allocated a surface.
    {
        let window_ref = window.clone();
        window.connect_realize(move |_| {
            apply_click_through(&window_ref);
        });
    }

    // Redraw timer. We only call queue_draw() when the IPC has produced a
    // newer seq than the last paint, so this is cheap when idle.
    let redraw_state = state.clone();
    let redraw_area = drawing_area.clone();
    let redraw_window = window.clone();
    let last_drawn_seq = Cell::new(0u64);
    // Tracks GTK visibility. Starts true because `window.present()` below maps
    // the surface, the first tick's idle check then hides it.
    let visible = Cell::new(true);

    glib::timeout_add_local(Duration::from_millis(RENDER_TICK_MS as u64), move || {
        let cur_seq = redraw_state.last_seq.lock().map(|s| *s).unwrap_or(0);
        let last_at = redraw_state
            .last_frame_at
            .lock()
            .map(|t| *t)
            .unwrap_or_else(|_| Instant::now() - Duration::from_secs(3600));
        let idle = last_at.elapsed().as_secs_f32() > IDLE_TIMEOUT_SECS;

        if idle {
            if visible.get() {
                tracing::info!("hiding (idle for {:.2}s)", last_at.elapsed().as_secs_f32());
                redraw_window.set_visible(false);
                visible.set(false);
            }
            return glib::ControlFlow::Continue;
        }

        if !visible.get() {
            tracing::info!(
                "showing (frame seq={}, last_at={:.3}s ago)",
                cur_seq,
                last_at.elapsed().as_secs_f32()
            );
            redraw_window.set_visible(true);
            visible.set(true);
        }

        // Decay the held peak even when no new frame arrived this tick.
        if let Ok(mut p) = redraw_state.peak.lock() {
            let dt = (RENDER_TICK_MS as f32) / 1000.0;
            // We pass the most recent peak from the ring as the "current"
            // value so a stale update doesn't snap the held value back up.
            let cur_peak = redraw_state
                .ring
                .lock()
                .ok()
                .and_then(|r| r.latest())
                .map(|f| f.peak_dbfs)
                .unwrap_or(-120.0);
            // Only decay; the IPC callback already snapped up on each
            // received frame. Calling update here with a non-louder peak
            // keeps the linear decay running at render rate.
            if cur_peak <= p.held_dbfs {
                p.update(cur_peak, dt);
            }
        }

        if cur_seq != last_drawn_seq.get() {
            redraw_area.queue_draw();
            last_drawn_seq.set(cur_seq);
        }
        glib::ControlFlow::Continue
    });

    // Map the layer-shell surface once. The redraw timer will hide it
    // immediately on its first tick (no frames yet → idle), and toggle
    // visibility from there. Mapping once at startup keeps Hyprland's
    // layer-shell state machine happy across hide/show cycles.
    window.present();
}

fn install_transparent_window_style(window: &ApplicationWindow) {
    let Some(display) = gtk4::gdk::Display::default() else {
        return;
    };
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(
        "window.voxtype-osd-pill { background-color: transparent; background-image: none; box-shadow: none; }",
    );
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    window.add_css_class("voxtype-osd-pill");
}

/// Set an empty input region on the GdkSurface so clicks pass through.
fn apply_click_through(window: &ApplicationWindow) {
    let Some(surface) = window.surface() else {
        tracing::warn!("Window has no surface yet; click-through not applied");
        return;
    };
    let empty = Region::create_rectangle(&RectangleInt::new(0, 0, 0, 0));
    surface.set_input_region(Some(&empty));
}

/// Render the waveform + peak meter into the given Cairo context.
fn draw_classic(
    cr: &Context,
    width: i32,
    height: i32,
    palette: &Palette,
    state: &Arc<SharedState>,
    gain: f64,
    opacity: f32,
) {
    let w = width as f64;
    let h = height as f64;
    if w <= 0.0 || h <= 0.0 {
        return;
    }

    // Clear background.
    cr.set_source_rgba(
        palette.background.r as f64,
        palette.background.g as f64,
        palette.background.b as f64,
        f64::from(opacity),
    );
    cr.set_operator(cairo::Operator::Source);
    cr.paint().ok();
    cr.set_operator(cairo::Operator::Over);

    // Layout: waveform area on the left (~92% width), gap (1%), then peak
    // meter on the right (~7% width).
    let meter_width = (w * 0.07).max(8.0);
    let gap = (w * 0.01).max(2.0);
    let wave_width = (w - meter_width - gap).max(0.0);

    draw_waveform(cr, wave_width, h, palette, state, gain);
    draw_peak_meter(cr, wave_width + gap, 0.0, meter_width, h, palette, state);
}

fn draw_waveform(
    cr: &Context,
    w: f64,
    h: f64,
    palette: &Palette,
    state: &Arc<SharedState>,
    gain: f64,
) {
    if w < 1.0 {
        return;
    }
    let n_columns = w.floor() as usize;
    if n_columns == 0 {
        return;
    }

    // Collect frames as a Vec snapshot under the lock, then drop.
    let frames: Vec<AudioFrame> = match state.ring.lock() {
        Ok(r) => r.iter().collect(),
        Err(_) => return,
    };
    let cols = project_envelope(&frames, n_columns);

    let mid = h * 0.5;
    let half = h * 0.5;

    // Mirrored envelope filled polygon. We trace the top edge left-to-right
    // following `max`, then the bottom edge right-to-left following `min`.
    cr.set_source_rgba(
        palette.accent.r as f64,
        palette.accent.g as f64,
        palette.accent.b as f64,
        palette.accent.a as f64,
    );

    cr.new_path();
    // Top edge.
    for (i, col) in cols.iter().enumerate() {
        let px = i as f64 + 0.5;
        let py = mid - sample_to_pixels(col.max, half, gain);
        if i == 0 {
            cr.move_to(px, py);
        } else {
            cr.line_to(px, py);
        }
    }
    // Bottom edge, right-to-left.
    for (i, col) in cols.iter().enumerate().rev() {
        let px = i as f64 + 0.5;
        let py = mid - sample_to_pixels(col.min, half, gain);
        cr.line_to(px, py);
    }
    cr.close_path();
    cr.fill().ok();

    // Subtle centerline.
    cr.set_source_rgba(
        palette.foreground.r as f64,
        palette.foreground.g as f64,
        palette.foreground.b as f64,
        0.15,
    );
    cr.set_line_width(1.0);
    cr.move_to(0.0, mid);
    cr.line_to(w, mid);
    cr.stroke().ok();
}

fn sample_to_pixels(sample: f32, half_height: f64, gain: f64) -> f64 {
    // Apply visual gain, then clamp to -1.0..=1.0, then scale to half_height.
    let s = (sample as f64 * gain).clamp(-1.0, 1.0);
    s * half_height
}

fn draw_peak_meter(
    cr: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    palette: &Palette,
    state: &Arc<SharedState>,
) {
    if w < 1.0 || h < 1.0 {
        return;
    }

    let (latest_peak, held_peak) = {
        let latest = state
            .ring
            .lock()
            .ok()
            .and_then(|r| r.latest())
            .map(|f| f.peak_dbfs)
            .unwrap_or(f32::NEG_INFINITY);
        let held = state
            .peak
            .lock()
            .map(|p| p.held_dbfs)
            .unwrap_or(f32::NEG_INFINITY);
        (latest, held)
    };

    let fill_frac = peak_meter_fraction(latest_peak, METER_FLOOR_DBFS) as f64;

    let segments = METER_SEGMENTS;
    let gap = 1.5_f64;
    let total_gap = gap * (segments as f64 - 1.0);
    let seg_h = ((h - total_gap) / segments as f64).max(1.0);

    for i in 0..segments {
        // Segment 0 is the bottom of the bar.
        let frac_top = (i as f64 + 1.0) / segments as f64;
        let lit = fill_frac >= (i as f64 + 0.5) / segments as f64;
        // dBFS at the *top* of this segment for color zone classification.
        let seg_top_db = METER_FLOOR_DBFS + (frac_top as f32) * (-METER_FLOOR_DBFS);
        let zone = MeterZone::from_dbfs(seg_top_db);
        let zone_color = zone.color(palette);

        let py = y + h - (i as f64 + 1.0) * seg_h - i as f64 * gap;

        if lit {
            cr.set_source_rgba(
                zone_color.r as f64,
                zone_color.g as f64,
                zone_color.b as f64,
                zone_color.a as f64,
            );
        } else {
            cr.set_source_rgba(
                zone_color.r as f64,
                zone_color.g as f64,
                zone_color.b as f64,
                0.18,
            );
        }
        cr.rectangle(x, py, w, seg_h);
        cr.fill().ok();
    }

    // Held-peak tick (1.5 px line at the held position).
    if held_peak.is_finite() && held_peak > METER_FLOOR_DBFS {
        let held_frac = peak_meter_fraction(held_peak, METER_FLOOR_DBFS) as f64;
        let py = y + h - held_frac * h;
        cr.set_source_rgba(
            palette.foreground.r as f64,
            palette.foreground.g as f64,
            palette.foreground.b as f64,
            0.95,
        );
        cr.set_line_width(1.5);
        cr.move_to(x, py);
        cr.line_to(x + w, py);
        cr.stroke().ok();
    }
}

fn draw_pill(
    cr: &Context,
    width: i32,
    height: i32,
    state: &Arc<SharedState>,
    gain: f64,
    opacity: f32,
) {
    let width = f64::from(width);
    let height = f64::from(height);
    if width <= 0.0 || height <= 0.0 {
        return;
    }

    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    cr.paint().ok();
    cr.set_operator(cairo::Operator::Over);

    rounded_rectangle(cr, 0.0, 0.0, width, height, height * 0.5);
    cr.set_source_rgba(0.180, 0.180, 0.200, f64::from(opacity));
    cr.fill().ok();

    let bars_x = PILL_HORIZONTAL_PADDING;
    let bars_width = (width - PILL_HORIZONTAL_PADDING * 2.0).max(1.0);
    let bar_count = (bars_width / 8.0).round().clamp(12.0, 42.0) as usize;
    let (frames, capacity) = match state.ring.lock() {
        Ok(ring) => (ring.iter().collect::<Vec<_>>(), ring.capacity()),
        Err(_) => return,
    };
    let levels = project_bar_levels(&frames, bar_count, capacity, gain as f32);
    draw_history_bars(cr, bars_x, height * 0.5, bars_width, height, &levels);
}

fn draw_history_bars(cr: &Context, x: f64, center_y: f64, width: f64, height: f64, levels: &[f32]) {
    if levels.is_empty() {
        return;
    }
    let slot = width / levels.len() as f64;
    let bar_width = (slot * 0.48).clamp(2.5, 5.0);
    let min_height = (height * 0.075).max(3.0);
    let max_height = height * 0.52;

    for (index, level) in levels.iter().enumerate() {
        let response = f64::from(*level).sqrt();
        let bar_height = min_height + response * (max_height - min_height);
        let age = (index + 1) as f64 / levels.len() as f64;
        let alpha = if *level > 0.0 {
            0.42 + age * 0.56
        } else {
            0.14
        };
        let bar_x = x + index as f64 * slot + (slot - bar_width) * 0.5;
        rounded_rectangle(
            cr,
            bar_x,
            center_y - bar_height * 0.5,
            bar_width,
            bar_height,
            bar_width * 0.5,
        );
        cr.set_source_rgba(0.96, 0.96, 0.97, alpha);
        cr.fill().ok();
    }
}

fn rounded_rectangle(cr: &Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width * 0.5).min(height * 0.5).max(0.0);
    cr.new_sub_path();
    cr.arc(x + width - radius, y + radius, radius, -PI * 0.5, 0.0);
    cr.arc(
        x + width - radius,
        y + height - radius,
        radius,
        0.0,
        PI * 0.5,
    );
    cr.arc(x + radius, y + height - radius, radius, PI * 0.5, PI);
    cr.arc(x + radius, y + radius, radius, PI, PI * 1.5);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pill_defaults_preserve_explicit_geometry_and_position() {
        let mut config = OsdConfig {
            width_px: 320,
            position: OsdPosition::TopCenter,
            ..OsdConfig::default()
        };
        let overrides = GeometryOverrides {
            width_px: true,
            ..GeometryOverrides::default()
        };

        apply_variant_defaults(&mut config, Gtk4Variant::Pill, &overrides);

        assert_eq!(config.width_px, 320);
        assert_eq!(config.height_px, 58);
        assert_eq!(config.margin_px, 59);
        assert_eq!(config.opacity, 1.0);
        assert_eq!(config.position, OsdPosition::TopCenter);

        // Pill center positions anchor to the requested edge rather than
        // sharing classic's fractional top-margin path.
        assert_eq!(
            edge_anchors(OsdPosition::TopCenter),
            (true, false, false, false)
        );
    }
}
