//! `voxtype-osd-gtk4` — GTK4 + gtk4-layer-shell on-screen mic visualizer
//! for the Voxtype daemon.
//!
//! Renders a click-through, layer-shell-anchored window containing either the
//! original scrolling waveform or an optional compact status pill. Audio
//! frames arrive on
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

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cairo::{Context, LinearGradient, RectangleInt, Region};
use clap::Parser;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, CssProvider, DrawingArea};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use voxtype::audio::levels::{AudioFrame, FRAME_HZ};
use voxtype::config::{load_config, Config as VoxtypeConfig};
use voxtype::osd::config::{OsdConfig, OsdPosition, OsdStyle};
use voxtype::osd::ipc::{resolve_socket_path, run_ipc_loop, FrameRing, DEFAULT_RING_DEPTH};
use voxtype::osd::theme::ThemeWatcher;
use voxtype::osd::visual::{peak_meter_fraction, project_envelope, MeterZone, Palette, PeakHold};

/// Load the `[osd]` section from the voxtype config file, falling back to
/// `OsdConfig::default()` on any error (file missing, unreadable, parse
/// failure, or `[osd]` section absent).
///
/// We deliberately ignore parse errors instead of returning them: the OSD
/// is a side car, and a malformed config shouldn't prevent it from running
/// with sensible defaults — the user will see the daemon complain about
/// the same file separately.
fn load_osd_config_from_file(explicit: Option<&std::path::Path>) -> OsdConfig {
    let path = explicit
        .map(std::path::Path::to_path_buf)
        .or_else(VoxtypeConfig::default_path);
    let Some(path) = path else {
        return OsdConfig::default();
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return OsdConfig::default(),
    };

    #[derive(serde::Deserialize, Default)]
    struct PartialConfig {
        #[serde(default)]
        osd: Option<OsdConfig>,
    }

    match toml::from_str::<PartialConfig>(&content) {
        Ok(p) => p.osd.unwrap_or_default(),
        Err(_) => OsdConfig::default(),
    }
}

/// Application id for the GTK4 frontend.
const APP_ID: &str = "io.voxtype.OsdGtk4";

/// Render tick period (~60 Hz). The redraw is gated on whether new frames
/// have arrived since the last paint, so this is a cheap upper bound.
const RENDER_TICK_MS: u32 = 16;

/// How long we wait without frames before treating the daemon as idle and
/// hiding the surface. Matches the BRIEF's "Idle: surface destroyed" rule.
const IDLE_TIMEOUT_SECS: f32 = 0.15;

/// dBFS floor for the peak meter (maps to "empty bar").
const METER_FLOOR_DBFS: f32 = -60.0;

/// Number of segments in the waveform style's vertical peak meter.
const METER_SEGMENTS: usize = 10;

/// Odd number of bars so the voice glyph has a true visual center.
const REACTIVE_BARS: usize = 15;

/// Background room noise is ignored below this visual-energy level.
const NOISE_GATE_ENTER: f64 = 0.24;
const NOISE_GATE_EXIT: f64 = 0.14;

/// Easing constants for the voice indicator.
const ENERGY_ATTACK: f64 = 0.32;
const ENERGY_RELEASE: f64 = 0.12;
const REVEAL_IN: f64 = 0.34;
const REVEAL_OUT: f64 = 0.18;
const SUCCESS_HOLD_MS: u128 = 860;
const SUCCESS_DRAW_MS: u128 = 340;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OsdMode {
    Recording,
    Processing,
    Success,
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

    /// Visual gain applied to audio samples before drawing the waveform.
    /// Higher = waveform fills more of the vertical for quiet inputs.
    /// Reduce for hot mics (e.g. 4.0); raise for quiet sources (e.g. 14.0).
    #[arg(long, env = "VOXTYPE_OSD_GAIN")]
    waveform_gain: Option<f32>,

    /// Visual style to render: "waveform" or "compact-pill".
    #[arg(long, env = "VOXTYPE_OSD_STYLE")]
    style: Option<String>,
}

/// State shared between the IPC worker and the GTK redraw timer.
struct SharedState {
    ring: Mutex<FrameRing>,
    peak: Mutex<PeakHold>,
    last_seq: Mutex<u64>,
    last_frame_at: Mutex<Instant>,
}

struct VisualState {
    energy: Cell<f64>,
    reveal: Cell<f64>,
    processing_phase: Cell<f64>,
    success_progress: Cell<f64>,
    success_started_at: RefCell<Option<Instant>>,
    was_processing: Cell<bool>,
    was_outputting: Cell<bool>,
    mode: Cell<OsdMode>,
    gate_open: Cell<bool>,
}

impl VisualState {
    fn new() -> Self {
        Self {
            energy: Cell::new(0.0),
            reveal: Cell::new(0.0),
            processing_phase: Cell::new(0.0),
            success_progress: Cell::new(0.0),
            success_started_at: RefCell::new(None),
            was_processing: Cell::new(false),
            was_outputting: Cell::new(false),
            mode: Cell::new(OsdMode::Recording),
            gate_open: Cell::new(false),
        }
    }
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
    let mut osd_cfg = load_osd_config_from_file(args.config.as_deref());
    if let Some(w) = args.width_px {
        osd_cfg.width_px = w;
    }
    if let Some(h) = args.height_px {
        osd_cfg.height_px = h;
    }
    if let Some(m) = args.margin_px {
        osd_cfg.margin_px = m;
    }
    if let Some(g) = args.waveform_gain {
        osd_cfg.waveform_gain = g;
    }
    if let Some(style) = args.style.as_deref() {
        match OsdStyle::parse_str(style) {
            Some(style) => osd_cfg.style = style,
            None => tracing::warn!("Ignoring unknown OSD style '{style}'"),
        }
    }
    // peak_decay_db_per_sec has a clap default value, so this always
    // overrides whatever the file said. That's intentional: if the user
    // passes the flag, honor it; if they don't, the clap default kicks in.
    osd_cfg.peak_decay_db_per_sec = args.peak_decay_db_per_sec;

    tracing::info!(
        "voxtype-osd-gtk4 starting; socket={:?} size={}x{} margin={} pos={:?} style={:?}",
        socket_path,
        osd_cfg.width_px,
        osd_cfg.height_px,
        osd_cfg.margin_px,
        osd_cfg.position,
        osd_cfg.style,
    );

    let theme = ThemeWatcher::new();
    let palette = theme.palette();

    let state_file_path = if matches!(osd_cfg.style, OsdStyle::CompactPill) {
        load_config(args.config.as_deref())
            .ok()
            .and_then(|cfg| cfg.resolve_state_file())
    } else {
        None
    };
    let state = Arc::new(SharedState::new(osd_cfg.peak_decay_db_per_sec));

    // Spawn the tokio IPC worker on a side thread.
    spawn_ipc_worker(
        state.clone(),
        socket_path,
        args.reconnect_secs,
        args.log_every,
    );

    // GTK application owns the main thread.
    let app = Application::builder().application_id(APP_ID).build();

    let cfg = osd_cfg.clone();
    let state_for_activate = state.clone();
    app.connect_activate(move |app| {
        build_window(
            app,
            &cfg,
            palette,
            state_file_path.clone(),
            state_for_activate.clone(),
        );
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

/// Spawn the tokio runtime + IPC loop on a dedicated thread.
fn spawn_ipc_worker(
    state: Arc<SharedState>,
    socket_path: PathBuf,
    reconnect_secs: f32,
    log_every: u32,
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
                if let Ok(mut r) = state.ring.lock() {
                    r.push(frame);
                }
                if let Ok(mut p) = state.peak.lock() {
                    p.update(frame.peak_dbfs, dt_per_frame);
                }
                if let Ok(mut s) = state.last_seq.lock() {
                    *s = s.wrapping_add(1);
                }
                if let Ok(mut t) = state.last_frame_at.lock() {
                    *t = Instant::now();
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

/// Build the GTK window, attach layer-shell config, mount the DrawingArea,
/// and start the redraw tick.
fn build_window(
    app: &Application,
    cfg: &OsdConfig,
    palette: Palette,
    state_file_path: Option<PathBuf>,
    state: Arc<SharedState>,
) {
    if matches!(cfg.style, OsdStyle::CompactPill) {
        install_transparent_css();
    }

    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(cfg.width_px as i32)
        .default_height(cfg.height_px as i32)
        .resizable(false)
        .decorated(false)
        .build();
    if matches!(cfg.style, OsdStyle::CompactPill) {
        window.add_css_class("voxtype-osd-window");
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

    if centered {
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
        // Corner positions: legacy anchor + uniform pixel margin behavior.
        let (anchor_top, anchor_bottom, anchor_left, anchor_right) = match cfg.position {
            OsdPosition::BottomLeft => (false, true, true, false),
            OsdPosition::BottomRight => (false, true, false, true),
            OsdPosition::TopLeft => (true, false, true, false),
            OsdPosition::TopRight => (true, false, false, true),
            // Centered branch is handled above; unreachable here.
            OsdPosition::BottomCenter | OsdPosition::TopCenter => unreachable!(),
        };
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
    if matches!(cfg.style, OsdStyle::CompactPill) {
        drawing_area.add_css_class("voxtype-osd-canvas");
    }
    drawing_area.set_content_width(cfg.width_px as i32);
    drawing_area.set_content_height(cfg.height_px as i32);
    let visual_state = Rc::new(VisualState::new());
    let visual_for_draw = visual_state.clone();
    let state_for_draw = state.clone();
    let draw_style = cfg.style;
    let gain = cfg.waveform_gain as f64;
    drawing_area.set_draw_func(move |_area, cr, w, h| match draw_style {
        OsdStyle::Waveform => draw_waveform_osd(cr, w, h, &palette, &state_for_draw, gain),
        OsdStyle::CompactPill => draw_compact_pill(
            cr,
            w,
            h,
            &palette,
            visual_for_draw.energy.get(),
            visual_for_draw.reveal.get(),
            visual_for_draw.processing_phase.get(),
            visual_for_draw.success_progress.get(),
            visual_for_draw.mode.get(),
        ),
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
    let visual_for_tick = visual_state.clone();
    let tick_style = cfg.style;
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
        if matches!(tick_style, OsdStyle::Waveform) {
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
                let cur_peak = redraw_state
                    .ring
                    .lock()
                    .ok()
                    .and_then(|r| r.latest())
                    .map(|f| f.peak_dbfs)
                    .unwrap_or(-120.0);
                if cur_peak <= p.held_dbfs {
                    p.update(cur_peak, dt);
                }
            }

            if cur_seq != last_drawn_seq.get() {
                redraw_area.queue_draw();
                last_drawn_seq.set(cur_seq);
            }
            return glib::ControlFlow::Continue;
        }

        let runtime_state = read_state_file(state_file_path.as_deref());
        let processing = runtime_state
            .as_deref()
            .is_some_and(|state| state == "transcribing");
        let outputting = runtime_state
            .as_deref()
            .is_some_and(|state| state == "outputting");
        let recording_state = runtime_state
            .as_deref()
            .is_some_and(|state| matches!(state, "recording" | "streaming"));
        let was_processing = visual_for_tick.was_processing.get();
        let outputting_started = outputting && !visual_for_tick.was_outputting.get();
        let now = Instant::now();
        let live_recording = recording_state || (!idle && !processing && !outputting);
        let completed_after_processing = was_processing
            && !processing
            && !live_recording
            && runtime_state
                .as_deref()
                .is_some_and(|state| state == "idle");
        let success_trigger = outputting_started || completed_after_processing;
        visual_for_tick.was_processing.set(processing);
        visual_for_tick.was_outputting.set(outputting);
        if processing || live_recording {
            visual_for_tick.success_started_at.replace(None);
            visual_for_tick.success_progress.set(0.0);
        } else if success_trigger && visual_for_tick.success_started_at.borrow().is_none() {
            visual_for_tick.success_started_at.replace(Some(now));
        }
        let success_progress = visual_for_tick
            .success_started_at
            .borrow()
            .map(|started_at| {
                (now.duration_since(started_at).as_millis() as f64 / SUCCESS_DRAW_MS as f64)
                    .clamp(0.0, 1.0)
            })
            .unwrap_or(0.0);
        let success_visible =
            visual_for_tick
                .success_started_at
                .borrow()
                .is_some_and(|started_at| {
                    now.duration_since(started_at).as_millis() <= SUCCESS_HOLD_MS
                });
        let success_latched = visual_for_tick.success_started_at.borrow().is_some();
        visual_for_tick.success_progress.set(success_progress);
        let active = live_recording || processing || success_visible;
        visual_for_tick.mode.set(if processing {
            OsdMode::Processing
        } else if success_latched && !live_recording {
            OsdMode::Success
        } else {
            OsdMode::Recording
        });

        if !visible.get() && active {
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

        let target_reveal = if active { 1.0 } else { 0.0 };
        let reveal_step = if target_reveal > visual_for_tick.reveal.get() {
            REVEAL_IN
        } else {
            REVEAL_OUT
        };
        let reveal = ease_toward(visual_for_tick.reveal.get(), target_reveal, reveal_step);
        visual_for_tick.reveal.set(reveal);

        let raw_energy = if idle {
            0.0
        } else {
            current_energy(&redraw_state, gain)
        };
        let gated = gated_energy(raw_energy, &visual_for_tick);
        let energy_step = if gated > visual_for_tick.energy.get() {
            ENERGY_ATTACK
        } else {
            ENERGY_RELEASE
        };
        let energy = ease_toward(visual_for_tick.energy.get(), gated, energy_step);
        visual_for_tick.energy.set(energy);
        if processing {
            let phase = (visual_for_tick.processing_phase.get() + 0.018) % 2.0;
            visual_for_tick.processing_phase.set(phase);
        }

        if !active && reveal <= 0.01 && energy <= 0.01 {
            if visible.get() {
                tracing::info!("hiding (idle for {:.2}s)", last_at.elapsed().as_secs_f32());
                redraw_window.set_visible(false);
                visible.set(false);
            }
            visual_for_tick.success_started_at.replace(None);
            visual_for_tick.success_progress.set(0.0);
            return glib::ControlFlow::Continue;
        }

        redraw_area.queue_draw();
        glib::ControlFlow::Continue
    });

    // Map the layer-shell surface once. The redraw timer will hide it
    // immediately on its first tick (no frames yet → idle), and toggle
    // visibility from there. Mapping once at startup keeps Hyprland's
    // layer-shell state machine happy across hide/show cycles.
    window.present();
}

fn install_transparent_css() {
    let Some(display) = gtk4::gdk::Display::default() else {
        return;
    };
    let provider = CssProvider::new();
    provider.load_from_data(
        "
        .voxtype-osd-window,
        .voxtype-osd-window.background,
        .voxtype-osd-window > *,
        .voxtype-osd-canvas {
            background: transparent;
            background-color: transparent;
            box-shadow: none;
            border: none;
        }
        ",
    );
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn read_state_file(path: Option<&std::path::Path>) -> Option<String> {
    let path = path?;
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
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
fn draw_waveform_osd(
    cr: &Context,
    width: i32,
    height: i32,
    palette: &Palette,
    state: &Arc<SharedState>,
    gain: f64,
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
        palette.background.a as f64,
    );
    cr.set_operator(cairo::Operator::Source);
    cr.paint().ok();
    cr.set_operator(cairo::Operator::Over);

    // Layout: waveform area on the left (~92% width), gap (1%), then peak
    // meter on the right (~7% width).
    let meter_width = (w * 0.07).max(8.0);
    let gap = (w * 0.01).max(2.0);
    let wave_width = (w - meter_width - gap).max(0.0);

    draw_waveform(cr, 0.0, 0.0, wave_width, h, palette, state, gain);
    draw_peak_meter(cr, wave_width + gap, 0.0, meter_width, h, palette, state);
}

fn draw_waveform(
    cr: &Context,
    x: f64,
    y: f64,
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

    let mid = y + h * 0.5;
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
        let px = x + i as f64 + 0.5;
        let py = mid - sample_to_pixels(col.max, half, gain);
        if i == 0 {
            cr.move_to(px, py);
        } else {
            cr.line_to(px, py);
        }
    }
    // Bottom edge, right-to-left.
    for (i, col) in cols.iter().enumerate().rev() {
        let px = x + i as f64 + 0.5;
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
    cr.move_to(x, mid);
    cr.line_to(x + w, mid);
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

/// Render a compact status pill into the given Cairo context.
fn draw_compact_pill(
    cr: &Context,
    width: i32,
    height: i32,
    palette: &Palette,
    energy: f64,
    reveal: f64,
    processing_phase: f64,
    success_progress: f64,
    mode: OsdMode,
) {
    let w = width as f64;
    let h = height as f64;
    if w <= 0.0 || h <= 0.0 {
        return;
    }

    // Clear to transparent so the window shape is defined by the pill, not
    // the rectangular layer-shell surface.
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    cr.set_operator(cairo::Operator::Source);
    cr.paint().ok();
    cr.set_operator(cairo::Operator::Over);

    let reveal = ease_out_cubic(reveal.clamp(0.0, 1.0));
    let fall = (1.0 - reveal).powf(0.72);
    let opacity = reveal;
    let slide_y = fall * h * 0.90;

    let pad = 5.0;
    let panel_x = pad;
    let panel_y = pad + slide_y;
    let panel_w = (w - pad * 2.0).max(1.0);
    let panel_h = ((h - pad * 2.0) * 0.82).max(1.0);
    let radius = panel_h * 0.5;

    if opacity <= 0.001 {
        return;
    }

    draw_panel(
        cr, panel_x, panel_y, panel_w, panel_h, radius, palette, opacity,
    );

    let glyph_inset = (panel_h * 0.42).min(panel_w * 0.18);
    let glyph_x = panel_x + glyph_inset;
    let glyph_w = panel_w - glyph_inset * 2.0;
    match mode {
        OsdMode::Recording => {
            draw_reactive_glyph(
                cr, glyph_x, panel_y, glyph_w, panel_h, palette, energy, opacity,
            );
        }
        OsdMode::Processing => {
            draw_processing_bar(
                cr,
                glyph_x,
                panel_y,
                glyph_w,
                panel_h,
                palette,
                processing_phase,
                opacity,
            );
        }
        OsdMode::Success => {
            draw_success_mark(
                cr,
                glyph_x,
                panel_y,
                glyph_w,
                panel_h,
                palette,
                success_progress,
                opacity,
            );
        }
    }
}

fn draw_panel(
    cr: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    palette: &Palette,
    opacity: f64,
) {
    // Soft multi-pass shadow.
    for (spread, alpha) in [(2.0, 0.08), (5.0, 0.040), (8.0, 0.024)] {
        rounded_rect(
            cr,
            x - spread,
            y - spread * 0.55,
            w + spread * 2.0,
            h + spread * 1.2,
            radius + spread,
        );
        cr.set_source_rgba(0.0, 0.0, 0.0, alpha * opacity);
        cr.fill().ok();
    }

    rounded_rect(cr, x, y, w, h, radius);
    let gradient = LinearGradient::new(x, y, x + w, y + h);
    gradient.add_color_stop_rgba(
        0.0,
        (palette.background.r as f64 * 1.35).min(1.0),
        (palette.background.g as f64 * 1.35).min(1.0),
        (palette.background.b as f64 * 1.35).min(1.0),
        0.82 * opacity,
    );
    gradient.add_color_stop_rgba(
        1.0,
        palette.background.r as f64 * 0.62,
        palette.background.g as f64 * 0.62,
        palette.background.b as f64 * 0.62,
        0.94 * opacity,
    );
    cr.set_source(&gradient).ok();
    cr.fill().ok();

    rounded_rect(cr, x + 0.5, y + 0.5, w - 1.0, h - 1.0, radius - 0.5);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.14 * opacity);
    cr.set_line_width(1.0);
    cr.stroke().ok();
}

fn draw_reactive_glyph(
    cr: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    palette: &Palette,
    energy: f64,
    opacity: f64,
) {
    if w < 1.0 {
        return;
    }

    let mid = y + h * 0.5;
    let bars = REACTIVE_BARS.min(w.floor() as usize);
    let center = (bars / 2) as f64;
    let bar_w = (w * 0.044).clamp(2.35, 3.25);
    let bar_gap = if bars > 1 {
        ((w - bar_w * bars as f64) / (bars as f64 - 1.0)).clamp(1.5, 6.0)
    } else {
        0.0
    };
    let used_w = bars as f64 * bar_w + (bars as f64 - 1.0) * bar_gap;
    let start_x = x + ((w - used_w) * 0.5).max(0.0);
    let flat_h = 2.4;
    let max_h = h * 0.70;

    rounded_rect(cr, start_x - 5.0, mid - 0.75, used_w + 10.0, 1.5, 0.75);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.055 * opacity);
    cr.fill().ok();

    let lifted_energy = energy.clamp(0.0, 1.0).powf(0.72);
    for i in 0..bars {
        let distance = ((i as f64 - center).abs() / center.max(1.0)).clamp(0.0, 1.0);
        let bell = (-distance * distance * 6.6).exp();
        let shoulder = (1.0 - distance).powf(2.4);
        let profile = (0.12 + bell * 0.78 + shoulder * 0.10).min(1.0);
        let active_h = max_h * ease_out_cubic(lifted_energy) * profile;
        let bar_h = (flat_h + active_h).min(h - 12.0).max(flat_h);
        let px = start_x + i as f64 * (bar_w + bar_gap);
        let py = mid - bar_h * 0.5;

        let center_weight = 1.0 - distance * 0.28;
        let alpha = (0.56 + lifted_energy * 0.34) * center_weight * opacity;
        let white_mix = 0.24 + lifted_energy * 0.18 * center_weight;

        rounded_rect(cr, px, py, bar_w, bar_h, bar_w * 0.5);
        cr.set_source_rgba(
            mix_channel(palette.accent.r as f64, 1.0, white_mix),
            mix_channel(palette.accent.g as f64, 1.0, white_mix),
            mix_channel(palette.accent.b as f64, 1.0, white_mix),
            alpha.clamp(0.18, 0.90),
        );
        cr.fill().ok();
    }
}

fn draw_processing_bar(
    cr: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    palette: &Palette,
    phase: f64,
    opacity: f64,
) {
    if w < 1.0 {
        return;
    }

    let track_h = (h * 0.16).clamp(4.0, 7.0);
    let track_y = y + h * 0.5 - track_h * 0.5;
    let radius = track_h * 0.5;

    rounded_rect(cr, x, track_y, w, track_h, radius);
    cr.set_source_rgba(
        palette.accent.r as f64,
        palette.accent.g as f64,
        palette.accent.b as f64,
        0.18 * opacity,
    );
    cr.fill().ok();

    let sweep_w = (w * 0.42).max(track_h * 2.2);
    let travel = w + sweep_w;
    let phase = phase.rem_euclid(2.0);
    let ping_pong = if phase <= 1.0 { phase } else { 2.0 - phase };
    let sweep_x = x - sweep_w + travel * ease_in_out_sine(ping_pong);
    let visible_x = sweep_x.max(x);
    let visible_w = (sweep_x + sweep_w).min(x + w) - visible_x;
    if visible_w <= 0.0 {
        return;
    }

    rounded_rect(cr, visible_x, track_y, visible_w, track_h, radius);
    let gradient = LinearGradient::new(visible_x, track_y, visible_x + visible_w, track_y);
    gradient.add_color_stop_rgba(
        0.0,
        palette.accent.r as f64,
        palette.accent.g as f64,
        palette.accent.b as f64,
        0.20 * opacity,
    );
    gradient.add_color_stop_rgba(
        0.5,
        mix_channel(palette.accent.r as f64, 1.0, 0.28),
        mix_channel(palette.accent.g as f64, 1.0, 0.28),
        mix_channel(palette.accent.b as f64, 1.0, 0.28),
        0.92 * opacity,
    );
    gradient.add_color_stop_rgba(
        1.0,
        palette.accent.r as f64,
        palette.accent.g as f64,
        palette.accent.b as f64,
        0.20 * opacity,
    );
    cr.set_source(&gradient).ok();
    cr.fill().ok();
}

fn draw_success_mark(
    cr: &Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    palette: &Palette,
    progress: f64,
    opacity: f64,
) {
    if w < 1.0 {
        return;
    }

    let reveal = ease_out_cubic(progress);
    let center_x = x + w * 0.5;
    let center_y = y + h * 0.5;
    let radius = (h * 0.24).clamp(8.0, 13.0);

    cr.arc(
        center_x,
        center_y,
        radius * (0.84 + reveal * 0.16),
        0.0,
        std::f64::consts::PI * 2.0,
    );
    cr.set_source_rgba(
        palette.accent.r as f64,
        palette.accent.g as f64,
        palette.accent.b as f64,
        0.16 * opacity * reveal,
    );
    cr.fill().ok();

    cr.arc(center_x, center_y, radius, 0.0, std::f64::consts::PI * 2.0);
    cr.set_source_rgba(
        mix_channel(palette.accent.r as f64, 1.0, 0.36),
        mix_channel(palette.accent.g as f64, 1.0, 0.36),
        mix_channel(palette.accent.b as f64, 1.0, 0.36),
        0.92 * opacity,
    );
    cr.set_line_width(1.35);
    cr.stroke().ok();

    let draw = ease_in_out_sine(progress);
    let p1 = (center_x - radius * 0.42, center_y - radius * 0.02);
    let p2 = (center_x - radius * 0.12, center_y + radius * 0.30);
    let p3 = (center_x + radius * 0.48, center_y - radius * 0.34);
    let first_len = 0.42;

    cr.set_source_rgba(
        mix_channel(palette.accent.r as f64, 1.0, 0.62),
        mix_channel(palette.accent.g as f64, 1.0, 0.62),
        mix_channel(palette.accent.b as f64, 1.0, 0.62),
        0.98 * opacity,
    );
    cr.set_line_width(2.1);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.move_to(p1.0, p1.1);

    if draw <= first_len {
        let t = draw / first_len;
        cr.line_to(mix_channel(p1.0, p2.0, t), mix_channel(p1.1, p2.1, t));
    } else {
        cr.line_to(p2.0, p2.1);
        let t = (draw - first_len) / (1.0 - first_len);
        cr.line_to(mix_channel(p2.0, p3.0, t), mix_channel(p2.1, p3.1, t));
    }
    cr.stroke().ok();
}

fn current_energy(state: &Arc<SharedState>, gain: f64) -> f64 {
    let (amplitude_energy, latest_peak) = match state.ring.lock() {
        Ok(r) => {
            let mut weighted = 0.0;
            let mut total_weight = 0.0;
            let latest_peak = r.latest().map(|f| f.peak_dbfs).unwrap_or(f32::NEG_INFINITY);
            for (idx, frame) in r
                .iter()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .take(10)
                .enumerate()
            {
                let weight = 1.0 / (idx as f64 + 1.0);
                let amp = frame.max.abs().max(frame.min.abs()) as f64;
                weighted += (amp * gain * 0.58).clamp(0.0, 1.0) * weight;
                total_weight += weight;
            }
            let amplitude = if total_weight > 0.0 {
                weighted / total_weight
            } else {
                0.0
            };
            (amplitude, latest_peak)
        }
        Err(_) => (0.0, f32::NEG_INFINITY),
    };
    let peak_energy = peak_meter_fraction(latest_peak, METER_FLOOR_DBFS) as f64;
    amplitude_energy.max(peak_energy * 0.52).clamp(0.0, 1.0)
}

fn gated_energy(raw: f64, visual: &VisualState) -> f64 {
    let open = visual.gate_open.get();
    if open {
        if raw < NOISE_GATE_EXIT {
            visual.gate_open.set(false);
            0.0
        } else {
            remap_energy(raw)
        }
    } else if raw >= NOISE_GATE_ENTER {
        visual.gate_open.set(true);
        remap_energy(raw)
    } else {
        0.0
    }
}

fn remap_energy(raw: f64) -> f64 {
    ((raw - NOISE_GATE_EXIT) / (1.0 - NOISE_GATE_EXIT)).clamp(0.0, 1.0)
}

fn ease_toward(current: f64, target: f64, amount: f64) -> f64 {
    current + (target - current) * amount.clamp(0.0, 1.0)
}

fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn ease_in_out_sine(t: f64) -> f64 {
    0.5 - 0.5 * (std::f64::consts::PI * t.clamp(0.0, 1.0)).cos()
}

fn mix_channel(a: f64, b: f64, t: f64) -> f64 {
    a * (1.0 - t) + b * t
}

fn rounded_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        std::f64::consts::PI * 1.5,
    );
    cr.close_path();
}
