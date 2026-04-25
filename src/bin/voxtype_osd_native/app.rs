//! Wayland + wgpu + egui-wgpu glue for `voxtype-osd-native`.
//!
//! The whole rendering stack is collapsed into one file because the borrow
//! relationships between SCTK state, the wgpu device/queue, the surface
//! configuration, and the egui-wgpu renderer are awkward to split without
//! introducing references with non-trivial lifetimes. Each piece is small,
//! and keeping them together makes the lifecycle (`create_surface_if_needed`
//! / `tear_down_surface`) easy to follow.

use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context as _};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{self, EventLoop},
        calloop_wayland_source::WaylandSource,
        client::{
            globals::registry_queue_init,
            protocol::{wl_output, wl_surface::WlSurface},
            Connection, Proxy, QueueHandle,
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
};

use voxtype::audio::levels::AudioFrame;
use voxtype::osd::config::{OsdConfig, OsdPosition};
use voxtype::osd::ipc::FrameRing;
use voxtype::osd::visual::{
    peak_meter_fraction, project_envelope, EnvelopeColumn, MeterZone, Palette, PeakHold,
};

/// State shared between the IPC thread and the render thread.
#[derive(Clone)]
pub struct SharedState {
    pub ring: Arc<Mutex<FrameRing>>,
    pub peak_hold: Arc<Mutex<PeakHold>>,
    /// Wall-clock timestamp of the most recent frame. Used to drive idle
    /// teardown when no frames have arrived for a while.
    pub last_frame_at: Arc<Mutex<Option<Instant>>>,
    pub palette: Palette,
    pub config: OsdConfig,
}

/// How long to keep the surface alive after the last frame arrived, before
/// destroying it. The daemon stops emitting between recordings; this value
/// controls how quickly the OSD disappears after that.
const IDLE_TEARDOWN_SECS: f32 = 5.0;
/// Target render rate. 60 Hz is enough for a smooth scrolling waveform; we
/// can't render faster than the underlying frame rate (100 Hz IPC) gains us.
const REDRAW_INTERVAL_MS: u64 = 16;

/// Outer state owned by the calloop event loop. Implements the SCTK delegate
/// traits via `delegate_*` macros.
pub struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    layer_shell: LayerShell,

    qh: QueueHandle<App>,
    conn: Connection,

    shared: SharedState,
    surface: Option<RenderSurface>,
}

/// All state tied to the live layer-shell surface. Dropped (via
/// `Option::take`) when we tear down for idle.
struct RenderSurface {
    layer: LayerSurface,
    wl_surface: WlSurface,

    /// Last accepted size from the compositor's configure. We use this to
    /// configure the wgpu surface.
    width: u32,
    height: u32,
    /// Whether we've received the first configure (and thus may render).
    configured: bool,

    // wgpu plumbing.
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_format: wgpu::TextureFormat,

    // egui plumbing.
    egui_ctx: egui::Context,
    egui_renderer: egui_wgpu::Renderer,
}

/// Run the event loop. Returns when the user closes the surface or the
/// loop exits via signal.
pub fn run(
    shared: SharedState,
    frame_ping_source: calloop::ping::PingSource,
) -> anyhow::Result<()> {
    let conn =
        Connection::connect_to_env().context("connect to Wayland; is WAYLAND_DISPLAY set?")?;
    let (globals, event_queue) = registry_queue_init::<App>(&conn).context("init registry")?;
    let qh = event_queue.handle();

    let mut event_loop: EventLoop<'static, App> =
        EventLoop::try_new().context("create calloop event loop")?;
    let loop_handle = event_loop.handle();

    let compositor_state =
        CompositorState::bind(&globals, &qh).context("compositor protocol unavailable")?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).context("wlr-layer-shell protocol unavailable")?;
    let output_state = OutputState::new(&globals, &qh);
    let registry_state = RegistryState::new(&globals);

    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .map_err(|e| anyhow!("insert WaylandSource: {}", e))?;

    let mut app = App {
        registry_state,
        output_state,
        compositor_state,
        layer_shell,
        qh: qh.clone(),
        conn: conn.clone(),
        shared,
        surface: None,
    };

    // Wake on each incoming audio frame: create the surface if needed,
    // request a redraw.
    loop_handle
        .insert_source(frame_ping_source, move |_, _, app: &mut App| {
            app.on_frame_ping();
        })
        .map_err(|e| anyhow!("insert ping source: {}", e))?;

    // Periodic redraw timer + idle teardown. Re-arms each fire.
    let timer = calloop::timer::Timer::from_duration(Duration::from_millis(REDRAW_INTERVAL_MS));
    loop_handle
        .insert_source(timer, |_deadline, _, app: &mut App| {
            app.tick();
            calloop::timer::TimeoutAction::ToDuration(Duration::from_millis(REDRAW_INTERVAL_MS))
        })
        .map_err(|e| anyhow!("insert redraw timer: {}", e))?;

    tracing::info!("entering event loop");
    loop {
        if let Err(e) = event_loop.dispatch(Some(Duration::from_secs(1)), &mut app) {
            tracing::error!("event loop dispatch failed: {}", e);
            break;
        }
    }

    drop(app);
    drop(conn);
    Ok(())
}

impl App {
    fn on_frame_ping(&mut self) {
        if self.surface.is_none() {
            if let Err(e) = self.create_surface() {
                tracing::warn!("Failed to create OSD surface: {:#}", e);
            }
        }
    }

    fn tick(&mut self) {
        let last_frame = self.shared.last_frame_at.lock().ok().and_then(|g| *g);
        let idle = match last_frame {
            Some(t) => t.elapsed().as_secs_f32() >= IDLE_TEARDOWN_SECS,
            None => true,
        };

        if idle && self.surface.is_some() {
            tracing::info!("Idle for {}s, tearing down surface", IDLE_TEARDOWN_SECS);
            self.tear_down_surface();
            return;
        }

        if self.surface.is_some() && !idle {
            if let Err(e) = self.render_frame() {
                tracing::warn!("render failed: {:#}", e);
            }
        }
    }

    fn create_surface(&mut self) -> anyhow::Result<()> {
        tracing::info!("Creating OSD layer surface");

        let wl_surface = self.compositor_state.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            wl_surface.clone(),
            Layer::Overlay,
            Some("voxtype-osd"),
            None,
        );

        let cfg = &self.shared.config;
        let (anchor, margin_top, margin_bottom, margin_left, margin_right) =
            position_to_anchor_and_margins(cfg.position, cfg.margin_px as i32);
        layer.set_anchor(anchor);
        layer.set_margin(margin_top, margin_right, margin_bottom, margin_left);
        layer.set_size(cfg.width_px, cfg.height_px);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(0);

        // Empty input region — clicks pass through. SCTK's Region helper
        // owns the wl_region and destroys it on drop. The wl_region must
        // outlive the commit that activates it; we let it drop after.
        let region = Region::new(&self.compositor_state)
            .map_err(|e| anyhow!("create input region: {}", e))?;
        wl_surface.set_input_region(Some(region.wl_region()));

        layer.commit();
        drop(region);

        // wgpu instance + surface.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });

        // Raw handles. With wayland-client's `system` feature + wayland-backend
        // `client_system`, ObjectId/Backend expose libwayland pointers.
        let display_ptr = NonNull::new(self.conn.backend().display_ptr() as *mut std::ffi::c_void)
            .ok_or_else(|| anyhow!("null wl_display ptr"))?;
        let surface_ptr = NonNull::new(wl_surface.id().as_ptr() as *mut std::ffi::c_void)
            .ok_or_else(|| anyhow!("null wl_surface ptr"))?;

        let raw_display = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display_ptr));
        let raw_window = RawWindowHandle::Wayland(WaylandWindowHandle::new(surface_ptr));

        // SAFETY: the `wl_display` and `wl_surface` outlive the wgpu surface
        // because `RenderSurface` keeps them alive (Connection is held in
        // `App`; wl_surface is held in RenderSurface).
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(raw_display),
                raw_window_handle: raw_window,
            })
        }
        .context("create wgpu surface")?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .context("request wgpu adapter")?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("voxtype-osd-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .context("request wgpu device")?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8UnormSrgb))
            .or_else(|| surface_caps.formats.first().copied())
            .ok_or_else(|| anyhow!("no surface formats available"))?;

        let egui_ctx = egui::Context::default();
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            surface_format,
            egui_wgpu::RendererOptions {
                msaa_samples: 1,
                depth_stencil_format: None,
                dithering: false,
                predictable_texture_filtering: false,
            },
        );

        self.surface = Some(RenderSurface {
            layer,
            wl_surface,
            width: cfg.width_px,
            height: cfg.height_px,
            configured: false,
            _instance: instance,
            surface,
            device,
            queue,
            surface_format,
            egui_ctx,
            egui_renderer,
        });

        Ok(())
    }

    fn tear_down_surface(&mut self) {
        if let Some(rs) = self.surface.take() {
            let RenderSurface {
                layer,
                wl_surface,
                surface,
                device,
                queue,
                egui_renderer,
                _instance,
                ..
            } = rs;
            // Drop wgpu state first, then the wl_surface. LayerSurface drops
            // the role on drop; we then explicitly destroy the wl_surface.
            drop(egui_renderer);
            drop(queue);
            drop(device);
            drop(surface);
            drop(_instance);
            drop(layer);
            wl_surface.destroy();
        }
    }

    fn render_frame(&mut self) -> anyhow::Result<()> {
        let rs = match self.surface.as_mut() {
            Some(s) if s.configured => s,
            _ => return Ok(()),
        };

        let cst = rs.surface.get_current_texture();
        let surface_texture: wgpu::SurfaceTexture = match cst {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                reconfigure_surface(rs);
                return Ok(());
            }
            other => {
                tracing::debug!("acquire frame skipped: {:?}", other);
                return Ok(());
            }
        };

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(rs.width as f32, rs.height as f32),
            )),
            ..Default::default()
        };

        let palette = self.shared.palette;
        let cfg = &self.shared.config;
        let waveform_window_secs = cfg.waveform_window_secs;
        let meter_w = ((rs.width as f32) * 0.05).max(8.0);
        let waveform_w = (rs.width as f32) - meter_w - 4.0;
        let n_columns = waveform_w.max(32.0) as usize;

        let envelope_cols = {
            let ring = self.shared.ring.lock().expect("ring poisoned");
            let frames_in_window =
                (waveform_window_secs * voxtype::audio::levels::FRAME_HZ as f32) as usize;
            let mut buf: Vec<AudioFrame> = ring.iter().collect();
            if buf.len() > frames_in_window {
                let skip = buf.len() - frames_in_window;
                buf = buf.split_off(skip);
            }
            project_envelope(&buf, n_columns)
        };

        let (peak_dbfs, held_dbfs) = {
            let ring = self.shared.ring.lock().expect("ring poisoned");
            let p = ring.latest().map(|f| f.peak_dbfs).unwrap_or(-120.0);
            let h = self
                .shared
                .peak_hold
                .lock()
                .map(|x| x.held_dbfs)
                .unwrap_or(-120.0);
            (p, h)
        };

        let width_px = rs.width;
        let height_px = rs.height;
        let full_output = rs.egui_ctx.run_ui(raw_input, |ui| {
            draw_ui(
                ui,
                width_px,
                height_px,
                &palette,
                &envelope_cols,
                peak_dbfs,
                held_dbfs,
            );
        });

        let primitives = rs
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [rs.width, rs.height],
            pixels_per_point: full_output.pixels_per_point,
        };

        for (id, image_delta) in &full_output.textures_delta.set {
            rs.egui_renderer
                .update_texture(&rs.device, &rs.queue, *id, image_delta);
        }

        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("voxtype-osd-encoder"),
            });

        rs.egui_renderer.update_buffers(
            &rs.device,
            &rs.queue,
            &mut encoder,
            &primitives,
            &screen_descriptor,
        );

        {
            let bg = palette.background;
            let mut rpass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("voxtype-osd-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: bg.r as f64,
                                g: bg.g as f64,
                                b: bg.b as f64,
                                a: bg.a as f64,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();

            rs.egui_renderer
                .render(&mut rpass, &primitives, &screen_descriptor);
        }

        for id in &full_output.textures_delta.free {
            rs.egui_renderer.free_texture(id);
        }

        rs.queue.submit(Some(encoder.finish()));
        rs.wl_surface.frame(&self.qh, rs.wl_surface.clone());
        surface_texture.present();
        Ok(())
    }
}

fn reconfigure_surface(rs: &mut RenderSurface) {
    let surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: rs.surface_format,
        width: rs.width.max(1),
        height: rs.height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    rs.surface.configure(&rs.device, &surface_config);
}

fn position_to_anchor_and_margins(pos: OsdPosition, margin: i32) -> (Anchor, i32, i32, i32, i32) {
    // (anchor, top, bottom, left, right)
    match pos {
        OsdPosition::BottomCenter => (Anchor::BOTTOM, 0, margin, 0, 0),
        OsdPosition::TopCenter => (Anchor::TOP, margin, 0, 0, 0),
        OsdPosition::BottomLeft => (Anchor::BOTTOM | Anchor::LEFT, 0, margin, margin, 0),
        OsdPosition::BottomRight => (Anchor::BOTTOM | Anchor::RIGHT, 0, margin, 0, margin),
        OsdPosition::TopLeft => (Anchor::TOP | Anchor::LEFT, margin, 0, margin, 0),
        OsdPosition::TopRight => (Anchor::TOP | Anchor::RIGHT, margin, 0, 0, margin),
    }
}

/// Render the egui UI: scrolling waveform on the left, segmented vertical
/// peak meter on the right.
fn draw_ui(
    ui: &mut egui::Ui,
    width: u32,
    height: u32,
    palette: &Palette,
    envelope: &[EnvelopeColumn],
    peak_dbfs: f32,
    held_dbfs: f32,
) {
    use egui::{Pos2, Rect};
    let painter = ui.painter().clone();
    let w = width as f32;
    let h = height as f32;
    let meter_w = (w * 0.05).max(8.0);
    let waveform_w = w - meter_w - 4.0;
    let waveform_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(waveform_w, h));
    let meter_rect = Rect::from_min_size(Pos2::new(w - meter_w, 0.0), egui::vec2(meter_w, h));

    draw_waveform(&painter, waveform_rect, palette, envelope);
    draw_meter(&painter, meter_rect, palette, peak_dbfs, held_dbfs);
}

fn draw_waveform(
    painter: &egui::Painter,
    rect: egui::Rect,
    palette: &Palette,
    envelope: &[EnvelopeColumn],
) {
    use egui::{pos2, Shape};
    if envelope.is_empty() {
        return;
    }
    let n = envelope.len();
    let col_w = rect.width() / n as f32;
    let mid_y = rect.center().y;
    let half_h = rect.height() * 0.45;

    let mut top_pts = Vec::with_capacity(n);
    let mut bot_pts = Vec::with_capacity(n);
    for (i, col) in envelope.iter().enumerate() {
        let x = rect.left() + (i as f32 + 0.5) * col_w;
        // min/max are in -1.0..=1.0; map to pixel y. y grows downward.
        let top = mid_y - col.max.clamp(-1.0, 1.0) * half_h;
        let bot = mid_y - col.min.clamp(-1.0, 1.0) * half_h;
        top_pts.push(pos2(x, top));
        bot_pts.push(pos2(x, bot));
    }

    // Build a closed polygon: top points left-to-right, bottom right-to-left.
    let mut polygon = top_pts;
    for p in bot_pts.iter().rev() {
        polygon.push(*p);
    }

    let fill = color_to_egui(palette.accent);
    painter.add(Shape::convex_polygon(polygon, fill, egui::Stroke::NONE));

    // Centerline tick for visual reference at low levels.
    let line_color = color_to_egui(palette.foreground.with_alpha(0.25));
    painter.line_segment(
        [pos2(rect.left(), mid_y), pos2(rect.right(), mid_y)],
        egui::Stroke::new(1.0, line_color),
    );
}

fn draw_meter(
    painter: &egui::Painter,
    rect: egui::Rect,
    palette: &Palette,
    peak_dbfs: f32,
    held_dbfs: f32,
) {
    use egui::{pos2, Rect};
    const SEGMENTS: usize = 10;
    const FLOOR_DBFS: f32 = -60.0;

    let segment_h = rect.height() / SEGMENTS as f32;
    let segment_gap = (segment_h * 0.15).clamp(1.0, 3.0);
    let inner_w = rect.width() - 4.0;
    let lit_fraction = peak_meter_fraction(peak_dbfs, FLOOR_DBFS);
    let lit_segments = (lit_fraction * SEGMENTS as f32).round() as usize;

    for i in 0..SEGMENTS {
        // Segment 0 is the BOTTOM of the bar (low dB == bottom).
        let y_top = rect.bottom() - (i as f32 + 1.0) * segment_h + segment_gap * 0.5;
        let y_bot = rect.bottom() - i as f32 * segment_h - segment_gap * 0.5;
        let seg_rect = Rect::from_min_max(
            pos2(rect.left() + 2.0, y_top),
            pos2(rect.left() + 2.0 + inner_w, y_bot),
        );

        let segment_peak_dbfs = FLOOR_DBFS * (1.0 - i as f32 / SEGMENTS as f32);
        let zone = MeterZone::from_dbfs(segment_peak_dbfs);
        let lit = i < lit_segments;
        let base = zone.color(palette);
        let color = if lit {
            color_to_egui(base)
        } else {
            color_to_egui(base.with_alpha(0.18))
        };
        painter.rect_filled(seg_rect, 1.0, color);
    }

    // Held-peak tick, drawn as a thin foreground bar.
    let held_fraction = peak_meter_fraction(held_dbfs, FLOOR_DBFS);
    if held_fraction > 0.0 {
        let y = rect.bottom() - held_fraction * rect.height();
        let tick_rect = Rect::from_min_max(
            pos2(rect.left() + 2.0, y - 1.0),
            pos2(rect.left() + 2.0 + inner_w, y + 1.0),
        );
        painter.rect_filled(tick_rect, 0.0, color_to_egui(palette.foreground));
    }
}

fn color_to_egui(c: voxtype::osd::visual::Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        (c.r.clamp(0.0, 1.0) * 255.0) as u8,
        (c.g.clamp(0.0, 1.0) * 255.0) as u8,
        (c.b.clamp(0.0, 1.0) * 255.0) as u8,
        (c.a.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

// ---------------------------------------------------------------------------
// SCTK delegate trait impls
// ---------------------------------------------------------------------------

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(rs) = self.surface.as_ref() {
            if rs.layer.wl_surface().id() == layer.wl_surface().id() {
                tracing::info!("Compositor closed the layer surface");
                self.tear_down_surface();
            }
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let rs = match self.surface.as_mut() {
            Some(s) => s,
            None => return,
        };
        if rs.layer.wl_surface().id() != layer.wl_surface().id() {
            return;
        }
        let (mut w, mut h) = configure.new_size;
        if w == 0 {
            w = self.shared.config.width_px;
        }
        if h == 0 {
            h = self.shared.config.height_px;
        }
        rs.width = w;
        rs.height = h;
        rs.configured = true;
        reconfigure_surface(rs);
        if let Err(e) = self.render_frame() {
            tracing::warn!("initial render after configure failed: {:#}", e);
        }
    }
}

delegate_compositor!(App);
delegate_output!(App);
delegate_layer!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_registry!(App);
