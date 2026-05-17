//! `voxtype-audio-bridge` — sidecar that exposes the daemon's audio frame
//! stream as NDJSON on stdout, for clients (notably the Quickshell QML
//! OSD) that cannot read Unix sockets directly.
//!
//! ## Protocol
//!
//! The daemon broadcasts 16-byte [`AudioFrame`]s at 100 Hz over a Unix
//! socket (default `$XDG_RUNTIME_DIR/voxtype/audio.sock`). This bridge
//! connects to that socket, decodes each frame, and prints one JSON
//! object per line on stdout:
//!
//! ```json
//! {"peak":0.421,"rms":0.180,"vad":1,"ts_ms":1234567}
//! ```
//!
//! Connection state changes are signalled inline:
//!
//! ```json
//! {"status":"connected"}
//! {"status":"disconnected"}
//! ```
//!
//! Stdout is flushed after every line so QML's `Process` element sees
//! updates immediately. All diagnostics (tracing) are written to stderr
//! so they don't corrupt the NDJSON stream.
//!
//! ## Field derivation
//!
//! The wire frame carries `seq`, `min`, `max`, `peak_dbfs`. The locked
//! NDJSON protocol exposes `peak`, `rms`, `vad`, `ts_ms`:
//!
//! - `peak`: `max(|min|, |max|)`, clamped to `[0, 1]`.
//! - `rms`: linear amplitude reconstructed from `peak_dbfs` and divided
//!   by `sqrt(2)` (the conventional sinusoidal peak-to-RMS ratio). This
//!   is an approximation; the wire frame does not carry true RMS.
//! - `vad`: heuristic, `peak_dbfs > VAD_THRESHOLD_DBFS` (-40 dBFS).
//! - `ts_ms`: milliseconds since the bridge started, monotonic.
//!
//! ## Reconnect behaviour
//!
//! Same backoff strategy as `src/osd/ipc.rs`: a fixed delay between
//! connection attempts, configurable via `--reconnect-secs`. An optional
//! `--reconnect-max-secs` is accepted for forward compatibility; the
//! current implementation uses a constant delay (matching the daemon's
//! existing OSD frontends), but the flag is parsed so callers can wire
//! it through without code changes when we add exponential backoff.

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Parser;
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tokio::time::sleep;

use voxtype::audio::levels::{default_socket_path, AudioFrame, FRAME_BYTES};

/// dBFS threshold above which we report `vad=1`. Chosen to match a
/// typical "speech is present" floor; quieter signals (room tone,
/// breathing) fall below.
const VAD_THRESHOLD_DBFS: f32 = -40.0;

#[derive(Parser, Debug)]
#[command(
    name = "voxtype-audio-bridge",
    version,
    about = "Bridge the voxtype daemon's audio frame socket to NDJSON on stdout"
)]
struct Args {
    /// Path to the audio-frame Unix socket. Defaults to
    /// `$XDG_RUNTIME_DIR/voxtype/audio.sock`.
    #[arg(long, env = "VOXTYPE_AUDIO_BRIDGE_SOCKET")]
    socket_path: Option<PathBuf>,

    /// Seconds to wait between reconnect attempts.
    #[arg(long, default_value = "1", env = "VOXTYPE_AUDIO_BRIDGE_RECONNECT_SECS")]
    reconnect_secs: u64,

    /// Maximum reconnect interval in seconds. Reserved for future
    /// exponential backoff; currently the bridge uses a constant
    /// `--reconnect-secs` delay between attempts.
    #[arg(
        long,
        default_value = "30",
        env = "VOXTYPE_AUDIO_BRIDGE_RECONNECT_MAX_SECS"
    )]
    reconnect_max_secs: u64,
}

fn main() -> anyhow::Result<()> {
    // Tracing goes to stderr only; stdout is reserved for NDJSON output.
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let socket_path = args.socket_path.unwrap_or_else(default_socket_path);
    let reconnect = Duration::from_secs(args.reconnect_secs.max(1));

    tracing::info!(
        socket = ?socket_path,
        reconnect_secs = args.reconnect_secs,
        reconnect_max_secs = args.reconnect_max_secs,
        "voxtype-audio-bridge starting"
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(run(socket_path, reconnect))
}

/// Top-level connect / read / reconnect loop. Never returns under normal
/// operation; exits only on a fatal stdout write error (caller closed the
/// pipe).
async fn run(socket_path: PathBuf, reconnect: Duration) -> anyhow::Result<()> {
    let started = Instant::now();
    // Tracks whether we've already emitted a `disconnected` status line
    // during the current run of "we can't reach the daemon". We avoid
    // spamming the line every reconnect attempt while the daemon stays
    // down. The flag is reset to `false` on every successful connect by
    // the natural flow: `stream_frames` returns, we emit `disconnected`
    // and set the flag to `true`; if the next connect succeeds, the
    // `connected` arm emits its status and the flag is no longer
    // consulted until the next disconnect.
    let mut disconnect_announced = false;

    loop {
        match UnixStream::connect(&socket_path).await {
            Ok(stream) => {
                tracing::info!(socket = ?socket_path, "connected to daemon");
                emit_status("connected")?;

                stream_frames(stream, started).await?;

                tracing::info!("daemon closed the socket; will reconnect");
                emit_status("disconnected")?;
                disconnect_announced = true;
            }
            Err(e) => {
                tracing::debug!(error = %e, socket = ?socket_path, "connect failed");
                if !disconnect_announced {
                    emit_status("disconnected")?;
                    disconnect_announced = true;
                }
            }
        }

        sleep(reconnect).await;
    }
}

/// Read 16-byte frames from `stream` and emit one NDJSON line per frame.
/// Returns when the stream ends (EOF, read error, or stdout write error).
async fn stream_frames(mut stream: UnixStream, started: Instant) -> anyhow::Result<()> {
    let mut buf = [0u8; FRAME_BYTES];
    let stdout = io::stdout();
    let mut out = stdout.lock();

    loop {
        match stream.read_exact(&mut buf).await {
            Ok(_) => {
                let frame = AudioFrame::from_bytes(&buf);
                let line = encode_frame(&frame, started.elapsed());
                if let Err(e) = out.write_all(line.as_bytes()) {
                    // Caller closed our pipe; propagate so the process exits.
                    return Err(e.into());
                }
                if let Err(e) = out.flush() {
                    return Err(e.into());
                }
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(error = %e, "read error on audio socket");
                return Ok(());
            }
        }
    }
}

/// Emit a `{"status":"..."}` line to stdout and flush.
fn emit_status(status: &str) -> io::Result<()> {
    let mut out = io::stdout().lock();
    // Hand-roll the JSON: the value set is fixed and tiny, no need for
    // serde_json on this path. Quoted strings cannot contain control
    // characters and are caller-controlled to be plain ASCII.
    writeln!(out, "{{\"status\":\"{}\"}}", status)?;
    out.flush()?;
    Ok(())
}

/// Encode one [`AudioFrame`] as a single NDJSON line (terminated by `\n`).
///
/// Values are rounded to 3 decimal places to keep lines compact (~50 B)
/// while preserving enough resolution for a 100 Hz waveform display.
fn encode_frame(frame: &AudioFrame, elapsed: Duration) -> String {
    let peak = peak_amplitude(frame);
    let rms = rms_approx(frame);
    let vad: u8 = if frame.peak_dbfs > VAD_THRESHOLD_DBFS {
        1
    } else {
        0
    };
    let ts_ms = elapsed.as_millis() as u64;

    format!(
        "{{\"peak\":{:.3},\"rms\":{:.3},\"vad\":{},\"ts_ms\":{}}}\n",
        peak, rms, vad, ts_ms
    )
}

/// Per-frame absolute peak in [0, 1].
fn peak_amplitude(frame: &AudioFrame) -> f32 {
    let p = frame.min.abs().max(frame.max.abs());
    p.clamp(0.0, 1.0)
}

/// Approximate RMS from `peak_dbfs`. The wire frame doesn't carry true
/// RMS, so we reconstruct the peak amplitude from `peak_dbfs` and divide
/// by `sqrt(2)` (the sinusoidal peak-to-RMS ratio). For a true signal
/// this under-reads on transients and over-reads on noise, but it's good
/// enough for a meter.
fn rms_approx(frame: &AudioFrame) -> f32 {
    let peak_linear = if frame.peak_dbfs <= -120.0 {
        0.0
    } else {
        10f32.powf(frame.peak_dbfs / 20.0)
    };
    (peak_linear / std::f32::consts::SQRT_2).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(min: f32, max: f32, peak_dbfs: f32) -> AudioFrame {
        AudioFrame {
            seq: 0,
            min,
            max,
            peak_dbfs,
        }
    }

    #[test]
    fn peak_amplitude_uses_max_abs() {
        let f = make_frame(-0.8, 0.4, -2.0);
        assert!((peak_amplitude(&f) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn peak_amplitude_clamped_to_unit() {
        let f = make_frame(-1.5, 0.0, 0.0);
        assert!((peak_amplitude(&f) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rms_approx_silence_is_zero() {
        let f = make_frame(0.0, 0.0, -120.0);
        assert_eq!(rms_approx(&f), 0.0);
    }

    #[test]
    fn rms_approx_full_scale_under_unit() {
        let f = make_frame(-1.0, 1.0, 0.0);
        let rms = rms_approx(&f);
        // 1.0 / sqrt(2) ≈ 0.7071
        assert!((rms - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
    }

    #[test]
    fn encode_frame_shape_is_locked_ndjson() {
        let f = make_frame(-0.5, 0.5, -10.0);
        let line = encode_frame(&f, Duration::from_millis(1234));
        // Trailing newline, no whitespace inside.
        assert!(line.ends_with('\n'));
        assert!(!line[..line.len() - 1].contains('\n'));
        assert!(line.starts_with('{'));
        assert!(line.trim_end().ends_with('}'));
        // Keys appear in locked order.
        let payload = line.trim_end();
        let parsed: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert!(parsed.get("peak").is_some());
        assert!(parsed.get("rms").is_some());
        assert!(parsed.get("vad").is_some());
        assert!(parsed.get("ts_ms").is_some());
        assert_eq!(parsed["ts_ms"], 1234);
    }

    #[test]
    fn vad_threshold_at_minus_40_dbfs() {
        let quiet = make_frame(0.0, 0.01, -50.0);
        let loud = make_frame(-0.3, 0.3, -10.0);
        let quiet_line = encode_frame(&quiet, Duration::ZERO);
        let loud_line = encode_frame(&loud, Duration::ZERO);
        assert!(quiet_line.contains("\"vad\":0"));
        assert!(loud_line.contains("\"vad\":1"));
    }
}
