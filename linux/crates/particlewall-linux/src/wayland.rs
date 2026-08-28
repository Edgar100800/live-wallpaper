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

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use wayland_client::protocol::{wl_compositor::WlCompositor, wl_output, wl_output::WlOutput, wl_registry, wl_registry::WlRegistry, wl_surface, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1}};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_surface_v1, zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1}};

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
    /// wl_display pointer of the dedicated connection.
    pub wl_display: *mut std::ffi::c_void,
    /// wl_surface (as wl_proxy) pointer backing `surface`.
    pub wl_surface: *mut std::ffi::c_void,
    /// Physical buffer size (logical configure size x buffer scale).
    pub size_px: (u32, u32),
    /// Buffer scale applied via set_buffer_scale (kept for diagnostics).
    #[allow(dead_code)]
    pub scale: i32,
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
        let outputs = std::mem::take(&mut state.outputs);

        Ok(Self {
            inner: Rc::new(Inner {
                conn,
                queue: RefCell::new(queue),
                state: RefCell::new(state),
                qh,
                compositor,
                shell,
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
        let layer_surface: ZwlrLayerSurfaceV1 = inner.shell.get_layer_surface(
            &surface,
            Some(output),
            Layer::Background,
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

        // Wait for the compositor's configure (ack is sent by the handler).
        inner.state.borrow_mut().configure = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
        let configured = loop {
            if std::time::Instant::now() >= deadline {
                return Err(format!("output {name} never sent a configure"));
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
                break cfg;
            }
        };
        let (_serial, logical_w, logical_h) = configured;
        let size_px = (logical_w.max(1) * scale as u32, logical_h.max(1) * scale as u32);

        let wl_display = inner.conn.backend().display_ptr();
        let wl_surface = surface.id().as_ptr();
        if wl_display.is_null() || wl_surface.is_null() {
            return Err(format!("null Wayland handles for {name}"));
        }

        Ok(GpuLayerSurface {
            _gpu: self.inner.clone(),
            surface,
            layer_surface,
            wl_display: wl_display.cast(),
            wl_surface: wl_surface.cast(),
            size_px,
            scale,
        })
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
        self.layer_surface.destroy();
        self.surface.destroy();
        let _ = self._gpu.conn.flush();
    }
}
