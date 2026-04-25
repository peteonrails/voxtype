# mic-osd worktree status

## Commit 1 — daemon-side audio level emitter and IPC

Landed: daemon-side scaffolding for the OSD audio-frame channel.

- New module `src/audio/levels.rs` (497 lines, 7 tests passing).
  - `AudioFrame { seq: u32, min: f32, max: f32, peak_dbfs: f32 }` (16 bytes, native byte order).
  - `LevelHub` binds a Unix socket and runs an accept loop + a broadcast loop.
  - `LevelBucketer` collects samples into 10 ms windows (160 samples at 16 kHz) and
    emits one `AudioFrame` per window. No allocation in the hot path.
  - `spawn_emitter` plumbs an existing `mpsc::Receiver<Vec<f32>>` (the chunk stream
    from `AudioCapture::start()`) through the bucketer into the hub. Task ends when
    the input channel closes (i.e. when the recording capture is dropped/stopped).
  - Fan-out is non-blocking: per-subscriber bounded queue (30 frames). Slow consumers
    are dropped, never back-pressured. When no subscribers are connected, frames are
    discarded with no work beyond a `try_send` and an empty `Vec::retain`.
- `Daemon` now owns an `Option<LevelHub>` plus an active emitter `JoinHandle`.
  - Hub is bound at daemon startup; bind failure is logged, not fatal.
  - `start_recording_capture()` helper centralises the three non-meeting
    `audio::create_capture` + `capture.start()` call sites and (when the hub is
    present) attaches a per-recording emitter task. Meeting `DualCapture` is left
    untouched.
  - Emitter is aborted in `start_transcription_task`; cancel paths rely on the
    capture's `Drop` closing the channel naturally.
  - Socket file is removed on shutdown.

### IPC choice

A new Unix socket at `$XDG_RUNTIME_DIR/voxtype/audio.sock`, separate from the
status socket. Reasoning: 100 Hz binary frames don't belong on the human-readable
status stream, and a separate socket lets subscribers connect/disconnect
independently without parsing status events. Per BRIEF.md, this is the recommended
shape.

### Design questions for Pete

1. The emitter is on by default once the hub binds; opt-out is "don't run the OSD".
   Adding an `[osd] enabled = false` switch is deferred to Commit 6 (config). Idle
   cost is essentially zero (no recording = no frames at all). OK to defer?
2. `to_bytes()` uses native byte order. Same-machine IPC, no portability concern,
   matches the `repr(C)` layout assertion in tests. OK?
3. Cancel paths abort the emitter implicitly via `capture.stop()` closing the
   chunk receiver. I considered adding `stop_level_emitter()` to each cancel site
   but the implicit close is correct and simpler.

## Validation

- `cargo check --offline --lib --bins --tests` clean (only pre-existing warnings).
- `cargo test --offline --lib`: 546 passed, 7 new in `audio::levels::tests`.
- `cargo fmt` applied to changed files.
- Clippy on changed files clean (the workspace has plenty of pre-existing
  clippy lints that aren't ours to fix here).

## Commit 2 — voxtype-osd binary skeleton

Landed: a second `[[bin]]` at `src/bin/voxtype_osd.rs`.

- Connects to the daemon socket, decodes `AudioFrame`s, drops them into a
  300-entry ring buffer (3 s at 100 Hz).
- Logs a `tracing::debug!` line every N frames so end-to-end IPC can be
  verified before any Wayland code lands.
- Reconnects automatically: when the daemon is down the binary sleeps for
  `--reconnect-secs` and tries again. EOF on the socket is handled the same
  way (daemon restart, recording ended cleanly, etc.).
- Three unit tests on the ring buffer pass.
- CLI: `--socket`, `--reconnect-secs`, `--log-every`, plus `VOXTYPE_OSD_SOCKET`
  env var (added the `env` feature to clap).

Smoke check is pending until Pete runs the daemon + OSD side by side. The
binary builds clean and the IPC types are shared via `voxtype::audio::levels`,
so a runtime mismatch is impossible.

## Commit 3 — shared `osd::` module + dual-binary skeleton

Pete decided to ship two frontends so users can pick their deployment
style: `voxtype-osd-native` (SCTK + wgpu + egui-wgpu, single static
binary) and `voxtype-osd-gtk4` (GTK4 + gtk4-layer-shell, smaller binary,
dyn-links GTK4 for systems that already ship it). This commit lands the
shared logic both binaries consume, and replaces the single
`voxtype-osd` skeleton from Commit 2.

- New module tree at `src/osd/`:
  - `ipc.rs` — `FrameRing` and `run_ipc_loop` factored out of the old
    skeleton; takes a per-frame callback so each frontend supplies its
    own state. Six unit tests on the ring buffer (oldest-first iter,
    partial-fill, clear/reset).
  - `visual.rs` — `Color`, `Palette` (with `fallback()`), `MeterZone`,
    `PeakHold` + free-function `update_peak_hold` matching BRIEF.md
    verbatim, `EnvelopeColumn`, `project_envelope` (handles partial-ring
    "fills from right", aggregates min/max when full), and
    `peak_meter_fraction`. Ten unit tests cover the math.
  - `config.rs` — `OsdConfig` and `OsdPosition`, defaults match BRIEF.md
    (`enabled=true`, 600x80, bottom-center, 0.85 opacity, 3s window,
    6 dB/sec decay). Three tests (defaults, kebab-case serde, partial
    TOML deserialise).
  - `theme.rs` — `omarchy_theme_dir()`, `load_palette()` (returns
    `Palette::fallback()` for now), `ThemeWatcher` placeholder. Real
    parsing + `notify`-based watcher land in Commit 5. Two tests.
- Two new feature-gated bin entry points:
  - `src/bin/voxtype_osd_native.rs` (required-features `osd-native`)
  - `src/bin/voxtype_osd_gtk4.rs` (required-features `osd-gtk4`)
  - Both connect via `osd::ipc::run_ipc_loop`, push frames into a
    shared `Arc<Mutex<FrameRing>>`, run a `PeakHold` update per frame,
    and emit a `tracing::debug!` line every `--log-every` frames. The
    `frontend` field in the log line distinguishes them; everything
    else (seq, peak_dbfs, held_dbfs, ring_len, …) is identical so
    Pete can verify shared logic by running them side-by-side.
- `Cargo.toml`: removed the `voxtype-osd` `[[bin]]` entry; added
  `osd-native` and `osd-gtk4` features (empty for now; GUI deps land
  in Commits 4a/4b) and the two `[[bin]]` entries gated on those
  features.
- `src/lib.rs` exposes `pub mod osd`.
- Old `src/bin/voxtype_osd.rs` deleted.

### Validation

- `cargo check --offline --lib`: clean (1 pre-existing warning).
- `cargo check --offline --features osd-native --bin voxtype-osd-native`:
  clean.
- `cargo check --offline --features osd-gtk4 --bin voxtype-osd-gtk4`:
  clean.
- `cargo test --offline --features osd-native,osd-gtk4 --lib`:
  566 passed (was 546; +20 new tests in `osd::*`).
- `cargo clippy --offline --features osd-native,osd-gtk4 --bin
  voxtype-osd-native --bin voxtype-osd-gtk4` clean for files we
  touched (preexisting warnings on unmodified files left alone per
  worktree brief).
- `cargo fmt -- --check` clean for files we touched.

### Notes

- The shared logic is fully runtime-verifiable now: with the daemon
  recording, both binaries pump identical frames through the same
  ring + peak-hold and log identical numerics. Stdout sanity check is
  Pete's call.
- Choice of GUI deps for Commits 4a/4b is deferred. The brief lists
  starting points; verify exact crate names + versions when wiring
  them in. Both feature flags currently have empty `dep:` lists so
  the build works today and grows naturally.

## Commit 4a — native (SCTK + wgpu + egui-wgpu) rendering

Landed: real GUI for `voxtype-osd-native`. The binary now opens a
wlr-layer-shell surface on demand and renders the waveform + peak meter
via egui-wgpu, with the architecture described below.

### Architecture

The binary splits into a main thread that runs the calloop event loop
(Wayland + render timer) and a dedicated IPC thread that owns a
single-threaded Tokio runtime to drive `osd::ipc::run_ipc_loop`. The IPC
thread pushes decoded `AudioFrame`s into the shared `Arc<Mutex<FrameRing>>`,
updates the `Arc<Mutex<PeakHold>>`, and pings the main thread via
`calloop::ping::Ping` after every frame. Pings coalesce, so 100 Hz of
notifications is fine.

Lifecycle:

- Surface is created on the first frame ping (initial connect, or after
  the daemon resumes recording). All wgpu/egui state lives in
  `RenderSurface`, which is `None` while idle.
- Surface is destroyed after `IDLE_TEARDOWN_SECS` (5 s) without a frame.
  This matches the BRIEF: surface destroyed when Idle, not just hidden.
- `LayerShellHandler::configure` accepts the compositor's size, configures
  the wgpu swapchain, and triggers an immediate render so the surface
  becomes visible. Subsequent renders are driven by a 16 ms calloop timer.
- Click-through is set up by attaching an empty `wl_region` as the input
  region (`KeyboardInteractivity::None` in addition).

### Files

- `src/bin/voxtype_osd_native/main.rs` — CLI parsing, IPC thread spawn,
  entry into the Wayland event loop. Replaces the old single-file
  `src/bin/voxtype_osd_native.rs`.
- `src/bin/voxtype_osd_native/app.rs` — all SCTK + wgpu + egui glue.
  Single file because the borrow relationships between the SCTK state,
  the wgpu device/queue, and the egui renderer fight when split.

### Rendering

- Waveform: `osd::visual::project_envelope` with 3 s of frames mapped onto
  ~95 % of the surface width. Filled `Shape::convex_polygon` (mirrored
  min/max columns) in `palette.accent` over `palette.background`.
- Peak meter: 10 vertical segments, color zones from `MeterZone::from_dbfs`,
  segment fill via `peak_meter_fraction(peak_dbfs, -60.0)`. Held-peak tick
  drawn as a thin foreground bar at the held position; held-peak decays
  through `osd::visual::PeakHold` (already updated on the IPC thread).
- Background uses `Palette::fallback()` today (Commit 5 swaps in real
  Omarchy parsing without changing this surface).

### Cargo.toml

`osd-native` now pulls in:

- `smithay-client-toolkit 0.20`, `calloop 0.14`, `calloop-wayland-source 0.4`
- `wayland-client 0.31` (with `system` feature) + `wayland-backend 0.3`
  (with `client_system` feature) so we can hand wgpu the raw libwayland
  pointers
- `wayland-protocols 0.32` and `wayland-protocols-wlr 0.3`
- `wgpu 29` (default-features off; `vulkan + gles + wgsl + std`)
- `egui 0.34` and `egui-wgpu 0.34`
- `raw-window-handle 0.6`, `pollster 0.4`, `bytemuck 1`

The binary path moved from `src/bin/voxtype_osd_native.rs` to
`src/bin/voxtype_osd_native/main.rs` so we can split modules cleanly.

### Validation

- `cargo check --features osd-native --bin voxtype-osd-native` clean.
- `cargo build --features osd-native --bin voxtype-osd-native --release`
  clean.
- `cargo test --features osd-native,osd-gtk4 --lib` 566 passed.
- `cargo clippy --features osd-native --bin voxtype-osd-native` clean on
  the OSD files (preexisting warnings on unmodified files left alone per
  brief).
- `rustfmt` clean on touched files (pre-existing diffs in unrelated
  files left alone per brief).
- Runtime smoke test (does the surface appear, does it look right, idle
  CPU < 0.1 %) is Pete's call; the agent environment can't run a Wayland
  client.

### Notes / things to review

- The `OsdConfig` consumed here is built from defaults plus a few CLI
  overrides (`--width-px`, `--height-px`, `--opacity`). Wiring the full
  `[osd]` config block + env-var layering is Commit 6, as planned.
- `IDLE_TEARDOWN_SECS = 5.0` is a literal in `app.rs`; if Pete wants it
  user-tunable, lift it onto `OsdConfig` in Commit 6.
- The wgpu swapchain uses `CompositeAlphaMode::PreMultiplied` so the
  background alpha (`palette.background.a = 0.85`) actually goes through
  the compositor as transparency.

## Next

Commit 4b: GTK4 + gtk4-layer-shell rendering for `voxtype-osd-gtk4`.
Commits 5/6: Omarchy theme parsing + watcher; `[osd]` config wiring.
