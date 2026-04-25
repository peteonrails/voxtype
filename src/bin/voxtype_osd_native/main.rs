//! `voxtype-osd-native` — native (SCTK + wgpu + egui-wgpu) on-screen
//! mic visualizer for the Voxtype daemon.
//!
//! Architecture:
//!
//! - The IPC reader runs on a dedicated thread with a single-threaded Tokio
//!   runtime; it pushes decoded `AudioFrame`s into an `Arc<Mutex<FrameRing>>`,
//!   updates an `Arc<Mutex<PeakHold>>`, and notifies the main thread via a
//!   `calloop::ping::Ping` so the renderer can wake the surface up.
//! - The main thread runs the Wayland event loop (calloop + SCTK), creates
//!   the wlr-layer-shell surface on demand when frames start arriving, and
//!   destroys it after a configurable idle timeout. While the surface is
//!   alive, a calloop timer drives ~60 Hz redraws.
//! - When no daemon is running, the IPC thread sleeps in its reconnect loop
//!   and the main thread sleeps in `EventLoop::run`. Idle CPU is essentially
//!   zero rendering work.
//!
//! The actual GUI smoke test (does it look right) is a human concern; the
//! bar this binary clears is "starts cleanly when the daemon is absent" plus
//! "exits cleanly on SIGTERM".

mod app;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use anyhow::Context as _;
use clap::Parser;

use voxtype::audio::levels::{AudioFrame, FRAME_HZ};
use voxtype::osd::config::OsdConfig;
use voxtype::osd::ipc::{resolve_socket_path, run_ipc_loop, FrameRing, DEFAULT_RING_DEPTH};
use voxtype::osd::theme::ThemeWatcher;
use voxtype::osd::visual::PeakHold;

use crate::app::SharedState;

#[derive(Parser, Debug)]
#[command(
    name = "voxtype-osd-native",
    version,
    about = "Voxtype on-screen mic visualizer (native: SCTK + wgpu + egui-wgpu)"
)]
struct Args {
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

    /// Surface width in pixels (overrides config default).
    #[arg(long, env = "VOXTYPE_OSD_WIDTH")]
    width_px: Option<u32>,

    /// Surface height in pixels (overrides config default).
    #[arg(long, env = "VOXTYPE_OSD_HEIGHT")]
    height_px: Option<u32>,

    /// Background opacity 0.0..=1.0 (overrides config default).
    #[arg(long, env = "VOXTYPE_OSD_OPACITY")]
    opacity: Option<f32>,
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

    let mut osd_config = OsdConfig::default();
    if let Some(w) = args.width_px {
        osd_config.width_px = w;
    }
    if let Some(h) = args.height_px {
        osd_config.height_px = h;
    }
    if let Some(o) = args.opacity {
        osd_config.opacity = o.clamp(0.0, 1.0);
    }

    if !osd_config.enabled {
        tracing::info!("OSD disabled in config; exiting");
        return Ok(());
    }

    tracing::info!(
        "voxtype-osd-native starting; socket={:?}, size={}x{}",
        socket_path,
        osd_config.width_px,
        osd_config.height_px
    );

    let theme = ThemeWatcher::new();
    let palette = theme.palette();

    let shared = SharedState {
        ring: Arc::new(Mutex::new(FrameRing::new(DEFAULT_RING_DEPTH))),
        peak_hold: Arc::new(Mutex::new(PeakHold::new(osd_config.peak_decay_db_per_sec))),
        last_frame_at: Arc::new(Mutex::new(None)),
        palette,
        config: osd_config,
    };

    // Set up the wakeup channel so the IPC thread can ping the main loop on
    // every frame. Calloop's ping is a single fd; the renderer wakes up,
    // creates the surface if needed, and resets the idle timer.
    let (frame_ping, frame_ping_source) =
        calloop::ping::make_ping().context("create calloop ping")?;

    // Spawn the IPC thread.
    let ipc_shared = shared.clone();
    let log_every = args.log_every;
    let reconnect_secs = args.reconnect_secs;
    let frame_ping_for_ipc = frame_ping.clone();
    let _ipc_thread = thread::Builder::new()
        .name("voxtype-osd-ipc".into())
        .spawn(move || {
            ipc_thread_main(
                ipc_shared,
                socket_path,
                reconnect_secs,
                log_every,
                frame_ping_for_ipc,
            );
        })
        .context("spawn IPC thread")?;

    // Run the Wayland + render event loop on the main thread.
    app::run(shared, frame_ping_source)
}

/// Entry point of the IPC thread. Owns a single-threaded Tokio runtime,
/// runs the reconnect-and-read loop, and pings the main thread on every
/// frame.
fn ipc_thread_main(
    shared: SharedState,
    socket_path: PathBuf,
    reconnect_secs: f32,
    log_every: u32,
    frame_ping: calloop::ping::Ping,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("Failed to build IPC runtime: {}", e);
            return;
        }
    };

    let mut total: u64 = 0;
    let mut last_log = Instant::now();
    let dt_per_frame = 1.0 / FRAME_HZ as f32;

    let on_frame = move |frame: AudioFrame| {
        if let Ok(mut r) = shared.ring.lock() {
            r.push(frame);
        }
        if let Ok(mut p) = shared.peak_hold.lock() {
            p.update(frame.peak_dbfs, dt_per_frame);
        }
        if let Ok(mut t) = shared.last_frame_at.lock() {
            *t = Some(Instant::now());
        }
        // Wake the renderer. Pings coalesce; calling 100x/sec is fine.
        frame_ping.ping();

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
                frontend = "native",
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
}
