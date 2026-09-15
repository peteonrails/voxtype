//! `voxtype-osd-gtk4` — GTK4 + gtk4-layer-shell on-screen mic visualizer
//! for the Voxtype daemon.
//!
//! Renders a click-through, layer-shell-anchored window containing the
//! scrolling waveform plus a segmented peak meter. Audio frames arrive on
//! the daemon's audio Unix socket via [`voxtype::osd::ipc::run_ipc_loop`],
//! decoded into [`AudioFrame`]s by a tokio runtime on a worker thread, and
//! pushed into a shared [`FrameRing`] + [`PeakHold`]. The GTK frame clock
//! redraws the `DrawingArea` while visible, including between audio arrivals.
//!
//! The daemon's state file provides immediate startup feedback while the
//! microphone wakes up. Once audio arrives, visibility follows the frame
//! stream, hiding the window when the socket goes quiet.
//!
//! Run with `RUST_LOG=debug` for verbose logs.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cairo::{Context, RectangleInt, Region};
use clap::Parser;
use gtk4::prelude::*;
use gtk4::{gio, glib};
use gtk4::{Application, ApplicationWindow, DrawingArea};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use voxtype::audio::levels::{AudioFrame, FRAME_HZ};
use voxtype::config::Config as VoxtypeConfig;
use voxtype::osd::config::{OsdConfig, OsdPosition};
use voxtype::osd::ipc::{resolve_socket_path, run_session_ipc_loop, FrameRing};
use voxtype::osd::recording::{sidecar_path, RecordingState};
use voxtype::osd::theme::ThemeWatcher;
use voxtype::osd::visual::{peak_meter_fraction, project_envelope, MeterZone, Palette, PeakHold};

/// Load the OSD settings and daemon state path without parsing unrelated
/// engine settings. Missing or malformed config uses the daemon defaults.
///
/// We deliberately ignore parse errors instead of returning them: the OSD
/// is a side car, and a malformed config shouldn't prevent it from running
/// with sensible defaults — the user will see the daemon complain about
/// the same file separately.
fn load_osd_config_from_file(explicit: Option<&std::path::Path>) -> (OsdConfig, Option<PathBuf>) {
    let defaults = || {
        (
            OsdConfig::default(),
            VoxtypeConfig::default().resolve_state_file(),
        )
    };
    let path = explicit
        .map(std::path::Path::to_path_buf)
        .or_else(VoxtypeConfig::default_path);
    let Some(path) = path else {
        return defaults();
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return defaults(),
    };

    #[derive(serde::Deserialize, Default)]
    struct PartialConfig {
        #[serde(default)]
        osd: Option<OsdConfig>,
        state_file: Option<String>,
    }

    match toml::from_str::<PartialConfig>(&content) {
        Ok(p) => {
            let mut config = VoxtypeConfig::default();
            if let Some(path) = p.state_file {
                config.state_file = Some(path);
            }
            (p.osd.unwrap_or_default(), config.resolve_state_file())
        }
        Err(_) => defaults(),
    }
}

/// Application id for the GTK4 frontend.
const APP_ID: &str = "io.voxtype.OsdGtk4";

/// Visibility polling period. Drawing follows the GTK frame clock.
const RENDER_TICK_MS: u32 = 16;

/// How long we wait without frames before treating the daemon as idle and
/// hiding the surface. Matches the BRIEF's "Idle: surface destroyed" rule.
const IDLE_TIMEOUT_SECS: f32 = 0.15;

/// Bound startup feedback if the daemon exits or the microphone never
/// produces samples. Live audio can still reveal the window afterward.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

fn waiting_for_audio(started: Option<Instant>, last_frame: Instant, now: Instant) -> bool {
    started.is_some_and(|started| {
        last_frame < started && now.saturating_duration_since(started) < STARTUP_TIMEOUT
    })
}

/// Watch transitions instead of reading the state file on every render tick.
/// Keep the monitor alive for the lifetime of the window. Audio-only
/// visibility remains available when state reporting is disabled or missing.
fn watch_recording_state(
    path: Option<&std::path::Path>,
    started: Rc<Cell<Option<Instant>>>,
    shared: Arc<SharedState>,
) -> Vec<gio::FileMonitor> {
    let Some(path) = path else { return Vec::new() };
    let path = path.to_path_buf();
    let paths = [path.clone(), sidecar_path(&path)];
    let refresh = Rc::new(move || {
        let tagged = RecordingState::read(&path);
        let (value, session) = if let Some(tagged) = tagged {
            (tagged.state, Some(tagged.session))
        } else {
            (
                std::fs::read_to_string(&path).unwrap_or_else(|_| "idle".into()),
                None,
            )
        };
        match value.trim() {
            "recording" | "streaming" => {
                let mut current = shared.session.lock().unwrap();
                if started.get().is_none() || *current != session {
                    *current = session;
                    shared.clear_audio();
                    started.set(Some(Instant::now()));
                }
            }
            "" => {} // A legacy state file may be between truncate and write.
            _ => started.set(None),
        }
    });
    let monitors = paths
        .iter()
        .filter_map(|path| {
            let file = gio::File::for_path(path);
            match file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
                Ok(monitor) => {
                    // GIO otherwise coalesces updates over 800 ms.
                    monitor.set_rate_limit(0);
                    let refresh = refresh.clone();
                    monitor.connect_changed(move |_, _, _, _| refresh());
                    Some(monitor)
                }
                Err(e) => {
                    tracing::warn!("Cannot watch daemon state {}: {e}", path.display());
                    None
                }
            }
        })
        .collect();
    // Subscribe before the initial read so a startup racing OSD launch is
    // either read here or delivered as an event.
    refresh();
    monitors
}

/// True while the daemon has marked the in-flight recording as OSD-suppressed
/// (`voxtype record start --no-osd`).
///
/// Frames stop flowing for a suppressed recording too, so the idle timeout
/// alone would hide the surface. This is checked anyway so suppression is
/// immediate rather than waiting out IDLE_TIMEOUT_SECS, and so the behaviour
/// matches the Quickshell frontend exactly.
fn osd_suppressed() -> bool {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    std::path::Path::new(&base)
        .join("voxtype/osd_suppressed")
        .exists()
}

/// Number of segments in the vertical peak meter.
const METER_SEGMENTS: usize = 10;

/// dBFS floor for the peak meter (maps to "empty bar").
const METER_FLOOR_DBFS: f32 = -60.0;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "voxtype-osd-gtk4",
    version,
    about = "Voxtype on-screen mic visualizer (GTK4 + gtk4-layer-shell)"
)]
struct Args {
    /// Path to the voxtype config file. Defaults to
    /// `~/.config/voxtype/config.toml`. Reads `[osd]` and `state_file`.
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
}

/// State shared between the IPC worker and the GTK redraw timer.
struct SharedState {
    session: Mutex<Option<u64>>,
    ring: Mutex<FrameRing>,
    window_frames: f64,
    peak: Mutex<PeakHold>,
    last_seq: Mutex<u64>,
    last_frame_at: Mutex<Instant>,
}

impl SharedState {
    // Caller holds session: clear and frame acceptance must be atomic with
    // respect to each other when GTK observes a new recording identity.
    fn clear_audio(&self) {
        self.ring.lock().unwrap().clear();
        let mut peak = self.peak.lock().unwrap();
        *peak = PeakHold::new(peak.decay_db_per_sec);
        *self.last_frame_at.lock().unwrap() = Instant::now() - Duration::from_secs(3600);
    }

    fn accept_frame(&self, session: Option<u64>, frame: AudioFrame) -> bool {
        let current = self.session.lock().unwrap();
        // Untagged frames keep older daemons usable. Tagged frames must belong
        // to the active recording, regardless of when IPC delivered them.
        if session.is_some() && current.is_some() && session != *current {
            return false;
        }
        self.ring.lock().unwrap().push(frame);
        self.peak
            .lock()
            .unwrap()
            .update(frame.peak_dbfs, 1.0 / FRAME_HZ as f32);
        let mut seq = self.last_seq.lock().unwrap();
        *seq = seq.wrapping_add(1);
        *self.last_frame_at.lock().unwrap() = Instant::now();
        true
    }

    fn new(decay_db_per_sec: f32, window_secs: f32) -> Self {
        let window_frames = window_secs.max(0.01) as f64 * FRAME_HZ as f64;
        Self {
            session: Mutex::new(None),
            ring: Mutex::new(FrameRing::for_window(window_secs)),
            window_frames,
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
    let (mut osd_cfg, state_file) = load_osd_config_from_file(args.config.as_deref());
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
    // peak_decay_db_per_sec has a clap default value, so this always
    // overrides whatever the file said. That's intentional: if the user
    // passes the flag, honor it; if they don't, the clap default kicks in.
    osd_cfg.peak_decay_db_per_sec = args.peak_decay_db_per_sec;

    tracing::info!(
        "voxtype-osd-gtk4 starting; socket={:?} size={}x{} margin={} pos={:?}",
        socket_path,
        osd_cfg.width_px,
        osd_cfg.height_px,
        osd_cfg.margin_px,
        osd_cfg.position,
    );

    let theme = ThemeWatcher::new();
    let palette = theme.palette();

    let state = Arc::new(SharedState::new(
        osd_cfg.peak_decay_db_per_sec,
        osd_cfg.waveform_window_secs,
    ));

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
            state_for_activate.clone(),
            state_file.as_deref(),
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

            let mut total: u64 = 0;
            let mut last_log = Instant::now();

            let on_frame = move |session: Option<u64>, frame: AudioFrame| {
                if !state.accept_frame(session, frame) {
                    return;
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

            rt.block_on(run_session_ipc_loop(socket_path, reconnect_secs, on_frame));
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
    state: Arc<SharedState>,
    state_file: Option<&std::path::Path>,
) {
    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(cfg.width_px as i32)
        .default_height(cfg.height_px as i32)
        .resizable(false)
        .decorated(false)
        .build();

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
    drawing_area.set_content_width(cfg.width_px as i32);
    drawing_area.set_content_height(cfg.height_px as i32);
    let state_for_draw = state.clone();
    let starting = Rc::new(Cell::new(false));
    let starting_for_draw = starting.clone();
    let gain = cfg.waveform_gain as f64;
    drawing_area.set_draw_func(move |_area, cr, w, h| {
        draw(
            cr,
            w,
            h,
            &palette,
            &state_for_draw,
            gain,
            starting_for_draw.get(),
        );
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

    // GTK pauses tick callbacks while the widget is hidden. While visible,
    // redraw even between audio deliveries so fractional scrolling continues.
    drawing_area.add_tick_callback(|area, _clock| {
        area.queue_draw();
        glib::ControlFlow::Continue
    });

    // Visibility must be polled separately: a hidden widget has no frame clock
    // ticks with which to notice the next recording.
    let redraw_state = state.clone();
    let redraw_window = window.clone();
    // Tracks GTK visibility. Starts true because `window.present()` below maps
    // the surface, the first tick's idle check then hides it.
    let visible = Cell::new(true);
    let recording_started = Rc::new(Cell::new(None));
    let state_monitor = watch_recording_state(state_file, recording_started.clone(), state.clone());

    glib::timeout_add_local(Duration::from_millis(RENDER_TICK_MS as u64), move || {
        let _keep_monitor_alive = &state_monitor;
        let cur_seq = redraw_state.last_seq.lock().map(|s| *s).unwrap_or(0);
        let last_at = redraw_state
            .last_frame_at
            .lock()
            .map(|t| *t)
            .unwrap_or_else(|_| Instant::now() - Duration::from_secs(3600));
        let now = Instant::now();
        let is_starting = waiting_for_audio(recording_started.get(), last_at, now);
        starting.set(is_starting);
        let idle = (!is_starting
            && now.saturating_duration_since(last_at).as_secs_f32() > IDLE_TIMEOUT_SECS)
            || osd_suppressed();

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
                "showing (frame seq={}, last_at={:.3}s ago, starting={})",
                cur_seq,
                last_at.elapsed().as_secs_f32(),
                is_starting,
            );
            redraw_window.set_visible(true);
            visible.set(true);
        }

        if !is_starting {
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
        }

        glib::ControlFlow::Continue
    });

    // Map the layer-shell surface once. The redraw timer will hide it
    // immediately on its first tick (no frames yet → idle), and toggle
    // visibility from there. Mapping once at startup keeps Hyprland's
    // layer-shell state machine happy across hide/show cycles.
    window.present();
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
fn draw(
    cr: &Context,
    width: i32,
    height: i32,
    palette: &Palette,
    state: &Arc<SharedState>,
    gain: f64,
    starting: bool,
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

    if starting {
        cr.set_source_rgba(
            palette.foreground.r as f64,
            palette.foreground.g as f64,
            palette.foreground.b as f64,
            palette.foreground.a as f64,
        );
        cr.set_font_size((h * 0.28).clamp(12.0, 18.0));
        let label = "Starting microphone…";
        if let Ok(extents) = cr.text_extents(label) {
            cr.move_to(
                (w - extents.width()) / 2.0 - extents.x_bearing(),
                (h - extents.height()) / 2.0 - extents.y_bearing(),
            );
            cr.show_text(label).ok();
        }
        return;
    }

    // Layout: waveform area on the left (~92% width), gap (1%), then peak
    // meter on the right (~7% width).
    let meter_width = (w * 0.07).max(8.0);
    let gap = (w * 0.01).max(2.0);
    let wave_width = (w - meter_width - gap).max(0.0);

    draw_waveform(cr, wave_width, h, palette, state, gain);
    draw_peak_meter(cr, wave_width + gap, 0.0, meter_width, h, palette, state);
}

#[allow(clippy::too_many_arguments)]
fn draw_waveform(
    cr: &Context,
    w: f64,
    h: f64,
    palette: &Palette,
    state: &Arc<SharedState>,
    gain: f64,
) {
    // The waveform occupies the drawing area's origin.
    let (x, y) = (0.0, 0.0);
    if w < 1.0 {
        return;
    }
    let n_columns = w.floor() as usize;
    if n_columns == 0 {
        return;
    }

    let cols = match state.ring.lock() {
        Ok(r) => {
            let frames: Vec<AudioFrame> = r.iter().collect();
            project_envelope(
                &frames,
                n_columns,
                state.window_frames,
                r.scroll_offset(Instant::now()),
            )
        }
        Err(_) => return,
    };

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
    (sample as f64 * gain).clamp(-1.0, 1.0) * half_height
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_prior_session_frames_cannot_end_startup() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::AsyncWriteExt;
        use voxtype::audio::levels::{session_socket_path, SessionFrame};
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("state");
                let socket = dir.path().join("audio.sock");
                let state = Arc::new(SharedState::new(24.0, 3.0));
                let started = Rc::new(Cell::new(None));
                let _monitors = watch_recording_state(Some(&path), started.clone(), state.clone());
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let listener =
                        tokio::net::UnixListener::bind(session_socket_path(&socket)).unwrap();
                    let seen = Arc::new(AtomicUsize::new(0));
                    let observed = seen.clone();
                    let shared = state.clone();
                    let reader =
                        tokio::spawn(run_session_ipc_loop(socket, 0.01, move |session, frame| {
                            shared.accept_frame(session, frame);
                            observed.fetch_add(1, Ordering::SeqCst);
                        }));
                    let (mut writer, _) = listener.accept().await.unwrap();
                    let frame = AudioFrame {
                        seq: 9,
                        min: -0.2,
                        max: 0.3,
                        peak_dbfs: -8.0,
                    };
                    // Hold the old frame until after a new state transition, just
                    // as an already-queued subscriber write can be delayed.
                    let queued = SessionFrame { session: 1, frame }.to_bytes();
                    RecordingState {
                        state: "recording".into(),
                        session: 2,
                    }
                    .write(&path)
                    .unwrap();
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while started.get().is_none() && Instant::now() < deadline {
                        while context.pending() {
                            context.iteration(false);
                        }
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    assert!(started.get().is_some());
                    writer.write_all(&queued).await.unwrap();
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while seen.load(Ordering::SeqCst) == 0 {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
                    assert!(state.ring.lock().unwrap().is_empty());
                    assert!(waiting_for_audio(
                        started.get(),
                        *state.last_frame_at.lock().unwrap(),
                        Instant::now()
                    ));
                    writer
                        .write_all(&SessionFrame { session: 2, frame }.to_bytes())
                        .await
                        .unwrap();
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while seen.load(Ordering::SeqCst) < 2 {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
                    assert!(!waiting_for_audio(
                        started.get(),
                        *state.last_frame_at.lock().unwrap(),
                        Instant::now()
                    ));
                    assert_eq!(state.ring.lock().unwrap().len(), 1);
                    reader.abort();
                });
            })
            .unwrap();
    }

    #[test]
    fn recording_shows_startup_before_first_audio_including_previous_session_frames() {
        let started = Instant::now();
        let previous_frame = started - Duration::from_millis(20);
        assert!(waiting_for_audio(Some(started), previous_frame, started));
        assert!(waiting_for_audio(
            Some(started),
            previous_frame,
            started + Duration::from_millis(800)
        ));
        assert!(!waiting_for_audio(
            Some(started),
            started + Duration::from_millis(800),
            started + Duration::from_millis(810)
        ));
    }

    #[test]
    fn cancelled_or_failed_startup_does_not_leave_popup_waiting_forever() {
        let started = Instant::now();
        let previous_frame = started - Duration::from_secs(1);
        assert!(!waiting_for_audio(None, previous_frame, started));
        assert!(!waiting_for_audio(
            Some(started),
            previous_frame,
            started + STARTUP_TIMEOUT
        ));
    }

    #[test]
    fn config_honors_custom_and_disabled_state_paths() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        for disabled in ["disabled", "none", "off", "false"] {
            std::fs::write(&config, format!("state_file = {disabled:?}\n")).unwrap();
            assert!(load_osd_config_from_file(Some(&config)).1.is_none());
        }
        let state = dir.path().join("custom-state");
        std::fs::write(
            &config,
            format!("state_file = {:?}\n", state.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(load_osd_config_from_file(Some(&config)).1, Some(state));
        std::fs::write(&config, "[osd]\nwidth_px = 600\n").unwrap();
        let (osd, state) = load_osd_config_from_file(Some(&config));
        assert_eq!(osd.width_px, 600);
        assert_eq!(state, VoxtypeConfig::default().resolve_state_file());
    }

    #[test]
    fn state_watcher_accepts_a_state_file_created_after_the_popup_starts() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("state");
                let started = Rc::new(Cell::new(None));
                let _monitor = watch_recording_state(
                    Some(&path),
                    started.clone(),
                    Arc::new(SharedState::new(24.0, 3.0)),
                );
                std::fs::write(&path, "recording").unwrap();
                let deadline = Instant::now() + Duration::from_secs(2);
                while started.get().is_none() && Instant::now() < deadline {
                    while context.pending() {
                        context.iteration(false);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                assert!(
                    started.get().is_some(),
                    "state created after OSD startup must be observed"
                );
            })
            .unwrap();
    }

    #[test]
    fn state_watcher_handles_start_cancel_and_atomic_replacement_without_audio() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("state");
                std::fs::write(&path, "idle").unwrap();
                let started = Rc::new(Cell::new(None));
                let _monitor = watch_recording_state(
                    Some(&path),
                    started.clone(),
                    Arc::new(SharedState::new(24.0, 3.0)),
                );
                let wait_for = |active: bool| {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while started.get().is_some() != active && Instant::now() < deadline {
                        while context.pending() {
                            context.iteration(false);
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    assert_eq!(started.get().is_some(), active);
                };
                std::fs::write(&path, "recording").unwrap();
                wait_for(true);
                std::fs::write(&path, "idle").unwrap();
                wait_for(false);
                let replacement = dir.path().join("replacement");
                std::fs::write(&replacement, "streaming").unwrap();
                std::fs::rename(&replacement, &path).unwrap();
                wait_for(true);
                std::fs::remove_file(&path).unwrap();
                wait_for(false);
            })
            .unwrap();
    }
}
