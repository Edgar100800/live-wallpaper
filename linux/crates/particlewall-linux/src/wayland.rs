//! GPU-specific Wayland plumbing: a dedicated client connection with
//! zwlr-layer-shell background surfaces, one per output.
//!
//! Why not the GTK layer windows? GTK 4.18+ holds a wp_fifo_v1 object on
//! every surface it owns for frame throttling, and the fifo protocol allows
//! only one fifo per surface. The Vulkan WSI also wants a fifo on the
//! surface it presents to, so any surface shared between GTK and wgpu dies
//! with "Surface already has a fifo" on the first swapchain configure.
//! These surfaces are only ever touched by our GPU presenter, which is
//! exactly the fifo-v1 recommendation: only the component performing
//! wl_surface.attach should use the protocol.
//!
//! Buffers are sized at the output's native resolution: with
//! wp_fractional_scale_v1 + wp_viewporter a 1.25x output renders 2560x1440
//! instead of the integer-scale 4096x2304 the compositor would downsample
//! (2.56x the pixels and swapchain memory).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use wayland_client::protocol::{wl_compositor::WlCompositor, wl_output, wl_output::WlOutput, wl_registry, wl_registry::WlRegistry, wl_surface, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1}};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_surface_v1, zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1}};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter};

use crate::web::layer::NAMESPACE;

/// Per-output info gathered from wl_output events.
#[derive(Default)]
struct OutInfo {
    name: Option<String>,
    scale: i32,
}

/// Event state for the dedicated GPU connection.
#[derive(Default)]
struct WState {
    compositor: Option<WlCompositor>,
    shell: Option<ZwlrLayerShellV1>,
    fractional: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
    outputs: Vec<(WlOutput, Arc<Mutex<OutInfo>>)>,
    /// Latest layer-surface configure (serial, width, height, logical).
    configure: Option<(u32, u32, u32)>,
    closed: bool,
}

impl Dispatch<WlRegistry, ()> for WState {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } => match interface.as_str() {
                "wl_compositor" => {
                    let compositor: WlCompositor =
                        registry.bind(name, version.min(4), qh, ());
                    state.compositor = Some(compositor);
                }
                "zwlr_layer_shell_v1" => {
                    let shell: ZwlrLayerShellV1 = registry.bind(name, version.min(1), qh, ());
                    state.shell = Some(shell);
                }
                "wp_fractional_scale_manager_v1" => {
                    state.fractional = Some(registry.bind(name, 1, qh, ()));
                }
                "wp_viewporter" => {
                    state.viewporter = Some(registry.bind(name, 1, qh, ()));
                }
                "wl_output" if version >= 4 => {
                    let info = Arc::new(Mutex::new(OutInfo::default()));
                    let output: WlOutput = registry.bind(name, 4, qh, info.clone());
                    state.outputs.push((output, info));
                }
                _ => {}
            },
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, Arc<Mutex<OutInfo>>> for WState {
    fn event(
        _state: &mut Self,
        _output: &WlOutput,
        event: wl_output::Event,
        data: &Arc<Mutex<OutInfo>>,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Name { name } => data.lock().unwrap().name = Some(name),
            wl_output::Event::Scale { factor } => data.lock().unwrap().scale = factor,
            _ => {}
        }
    }
}

impl Dispatch<WlCompositor, ()> for WState {
    fn event(
        _state: &mut Self,
        _proxy: &WlCompositor,
        _event: <WlCompositor as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for WState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrLayerShellV1,
        _event: zwlr_layer_shell_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for WState {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                state.configure = Some((serial, width, height));
                layer_surface.ack_configure(serial);
            }
            zwlr_layer_surface_v1::Event::Closed => state.closed = true,
            _ => {}
        }
    }
}

/// Preferred scale in 1/120 units; 0 until the compositor sends one.
impl Dispatch<WpFractionalScaleV1, Arc<AtomicU32>> for WState {
    fn event(
        _state: &mut Self,
        _proxy: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        data: &Arc<AtomicU32>,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            data.store(scale, Ordering::Relaxed);
        }
    }
}

wayland_client::delegate_noop!(WState: WpFractionalScaleManagerV1);
wayland_client::delegate_noop!(WState: WpViewporter);
wayland_client::delegate_noop!(WState: WpViewport);

impl Dispatch<WlSurface, ()> for WState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSurface,
        _event: wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

struct Inner {
    conn: Connection,
    queue: RefCell<wayland_client::EventQueue<WState>>,
    state: RefCell<WState>,
    qh: QueueHandle<WState>,
    compositor: WlCompositor,
    shell: ZwlrLayerShellV1,
    /// Native-resolution sizing; both are needed, else integer buffer scale.
    native_scaling: Option<(WpFractionalScaleManagerV1, WpViewporter)>,
    outputs: Vec<(WlOutput, Arc<Mutex<OutInfo>>)>,
}

/// Owns the dedicated Wayland connection and creates GPU layer surfaces.
pub struct WaylandGpu {
    inner: Rc<Inner>,
}

/// One mapped layer surface for the GPU presenter. Kept alive alongside the
/// presenter; the raw wl_display/wl_surface pointers feed wgpu.
pub struct GpuLayerSurface {
    _gpu: Rc<Inner>,
    surface: WlSurface,
    layer_surface: ZwlrLayerSurfaceV1,
    fractional: Option<WpFractionalScaleV1>,
    viewport: Option<WpViewport>,
    /// wl_display pointer of the dedicated connection.
    pub wl_display: *mut std::ffi::c_void,
    /// wl_surface (as wl_proxy) pointer backing `surface`.
    pub wl_surface: *mut std::ffi::c_void,
    /// Physical buffer size: logical configure size x preferred fractional
    /// scale, or x integer buffer scale without wp_fractional_scale_v1.
    pub size_px: (u32, u32),
    /// Integer wl_output scale (kept for diagnostics).
    #[allow(dead_code)]
    pub scale: i32,
    /// size_px relative to the integer-scale buffer the daemon used before
    /// native sizing (0.625 on a 1.25x output). Point sizes are multiplied
    /// by it so particles keep their on-screen size.
    pub point_scale: f32,
}

impl WaylandGpu {
    /// Connects to the compositor with a private connection and collects
    /// globals + output identities.
    pub fn connect() -> Result<Self, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("wayland connect: {e}"))?;
        let mut queue = conn.new_event_queue::<WState>();
        let qh = queue.handle();
        let mut state = WState::default();
        conn.display().get_registry(&qh, ());
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("wayland roundtrip: {e}"))?;
        // Output name/scale events arrive after the bind triggered by the
        // first roundtrip; give them one more trip to land.
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("wayland roundtrip: {e}"))?;

        let compositor = state
            .compositor
            .take()
            .ok_or("missing wl_compositor global")?;
        let shell = state.shell.take().ok_or("missing zwlr_layer_shell_v1 global")?;
        let native_scaling = state.fractional.take().zip(state.viewporter.take());
        let outputs = std::mem::take(&mut state.outputs);

        Ok(Self {
            inner: Rc::new(Inner {
                conn,
                queue: RefCell::new(queue),
                state: RefCell::new(state),
                qh,
                compositor,
                shell,
                native_scaling,
                outputs,
            }),
        })
    }

    /// Connector names ("HDMI-A-1") of the current outputs.
    pub fn output_names(&self) -> Vec<String> {
        self.inner
            .outputs
            .iter()
            .filter_map(|(_, info)| info.lock().unwrap().name.clone())
            .collect()
    }

    /// Creates and maps a full-output background layer surface for `name`.
    pub fn create_surface(&self, name: &str) -> Result<GpuLayerSurface, String> {
        let inner = &self.inner;
        let (output, out_info) = inner
            .outputs
            .iter()
            .find(|(_, info)| info.lock().unwrap().name.as_deref() == Some(name))
            .ok_or_else(|| format!("unknown output {name}"))?;
        let scale = out_info.lock().unwrap().scale.max(1);

        let surface: WlSurface = inner.compositor.create_surface(&inner.qh, ());
        let preferred = Arc::new(AtomicU32::new(0));
        let native = inner.native_scaling.as_ref().map(|(manager, viewporter)| {
            (
                manager.get_fractional_scale(&surface, &inner.qh, preferred.clone()),
                viewporter.get_viewport(&surface, &inner.qh, ()),
            )
        });
        let (fractional, viewport) = native.unzip();
        let layer_surface =
            map_background_role(inner, &surface, output, if viewport.is_some() { 1 } else { scale });

        let wl_display = inner.conn.backend().display_ptr();
        let wl_surface = surface.id().as_ptr();
        // Own both protocol objects before any fallible operation so Drop also
        // destroys them on null handles, configure errors, or connection loss.
        let mut session = GpuLayerSurface {
            _gpu: self.inner.clone(),
            surface,
            layer_surface,
            fractional,
            viewport,
            wl_display: wl_display.cast(),
            wl_surface: wl_surface.cast(),
            size_px: (1, 1),
            scale,
            point_scale: 1.0,
        };
        if wl_display.is_null() || wl_surface.is_null() {
            return Err(format!("null Wayland handles for {name}"));
        }
        let wants_fraction = session.viewport.is_some();
        let (_serial, logical_w, logical_h) = wait_configure(inner, name, wants_fraction.then_some(&*preferred))?;
        let (logical_w, logical_h) = (logical_w.max(1), logical_h.max(1));
        let integer_px = (logical_w * scale as u32, logical_h * scale as u32);
        let fraction = preferred.load(Ordering::Relaxed);
        match &session.viewport {
            Some(viewport) if fraction > 0 => {
                // Double-buffered: applies with the presenter's first commit.
                viewport.set_destination(logical_w as i32, logical_h as i32);
                let px = |logical: u32| (u64::from(logical) * u64::from(fraction)).div_ceil(120) as u32;
                session.size_px = (px(logical_w), px(logical_h));
            }
            _ => {
                // No preferred scale arrived: keep the integer path.
                if wants_fraction && scale > 1 {
                    session.surface.set_buffer_scale(scale);
                }
                session.size_px = integer_px;
            }
        }
        session.point_scale = session.size_px.0 as f32 / integer_px.0 as f32;
        Ok(session)
    }
}

/// Assigns (or re-assigns) the full-output Bottom-layer role to
/// `surface`: anchor, keyboard policy, buffer scale and commit. The
/// configure wait happens separately in `wait_configure`. The Bottom layer
/// sits above Background (where shells map their own static wallpapers, e.g.
/// Omarchy's quickshell) regardless of map order, so the daemon never races
/// for stacking at boot — the only alternative (unmapping/re-roling the
/// surface feeding a live swapchain) segfaults the NVIDIA Wayland WSI.
fn map_background_role(
    inner: &Rc<Inner>,
    surface: &WlSurface,
    output: &WlOutput,
    scale: i32,
) -> ZwlrLayerSurfaceV1 {
    let layer_surface: ZwlrLayerSurfaceV1 = inner.shell.get_layer_surface(
        surface,
        Some(output),
        Layer::Bottom,
        NAMESPACE.to_string(),
        &inner.qh,
        (),
    );
    layer_surface.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
    layer_surface.set_exclusive_zone(-1);
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
    if scale > 1 {
        surface.set_buffer_scale(scale);
    }
    surface.commit();
    layer_surface
}

/// Rounds the event queue until the layer surface gets its configure
/// (acked by the handler) and, when requested, its preferred fractional
/// scale. Returns the logical (width, height); a missing preferred scale is
/// not an error (the caller falls back to the integer scale).
fn wait_configure(
    inner: &Rc<Inner>,
    name: &str,
    preferred: Option<&AtomicU32>,
) -> Result<(u32, u32, u32), String> {
    inner.state.borrow_mut().configure = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
    let mut configured = None;
    loop {
        let scale_known = preferred.is_none_or(|p| p.load(Ordering::Relaxed) > 0);
        match configured {
            Some(cfg) if scale_known => return Ok(cfg),
            Some(cfg) if std::time::Instant::now() >= deadline => return Ok(cfg),
            None if std::time::Instant::now() >= deadline => {
                return Err(format!("output {name} never sent a configure"));
            }
            _ => {}
        }
        inner
            .queue
            .borrow_mut()
            .roundtrip(&mut inner.state.borrow_mut())
            .map_err(|e| format!("wayland roundtrip: {e}"))?;
        let mut state = inner.state.borrow_mut();
        if state.closed {
            return Err(format!("output {name} closed while configuring"));
        }
        if let Some(cfg) = state.configure.take() {
            configured = Some(cfg);
        }
    }
}

impl GpuLayerSurface {
    /// Raw handle pair for `GpuPresenter::new`.
    pub fn handles(&self) -> particlewall_render::gpu::WaylandHandles {
        particlewall_render::gpu::WaylandHandles {
            display: self.wl_display,
            surface: self.wl_surface,
        }
    }
}

impl Drop for GpuLayerSurface {
    fn drop(&mut self) {
        if let Some(viewport) = &self.viewport {
            viewport.destroy();
        }
        if let Some(fractional) = &self.fractional {
            fractional.destroy();
        }
        self.layer_surface.destroy();
        self.surface.destroy();
        let _ = self._gpu.conn.flush();
    }
}
