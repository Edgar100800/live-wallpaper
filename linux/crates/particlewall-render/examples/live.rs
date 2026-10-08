//! Live presenter check: maps a Bottom-layer surface on the first output
//! (like the daemon), renders a model through `GpuPresenter` on a timer at
//! the FPS cap for a few seconds, and reports CPU time per frame.
//!
//!   cargo run --release -p particlewall-render --example live -- [model|ascii[:dir]] [seconds] [fps] [native|integer]
//!
//! `ascii` plays the bundled Spider-Man clip (or `dir`) through AsciiPlayer.
//!
//! `native` (default) sizes the buffer from wp_fractional_scale_v1 and maps
//! it with wp_viewporter; `integer` reproduces the old path (integer
//! wl_output scale + set_buffer_scale), which oversizes fractional outputs.
//!
//! CPU time is the sum over the process's threads (/proc/self/task/*/schedstat),
//! so it includes the driver's swapchain and present work.

use std::time::{Duration, Instant};

use particlewall_render::ascii::{AsciiPlayer, AsciiStyle};
use particlewall_render::gpu::{GpuPresenter, ParticleRenderer, VertexPath, WaylandHandles};
use particlewall_render::Uniforms;
use wayland_client::protocol::{wl_compositor, wl_output, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    output: Option<wl_output::WlOutput>,
    fractional_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    /// Preferred scale in 1/120 units (wp_fractional_scale_v1).
    preferred_scale: Option<u32>,
    scale: i32,
    configure: Option<(u32, u32)>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => state.compositor = Some(registry.bind(name, version.min(4), qh, ())),
                "zwlr_layer_shell_v1" => state.shell = Some(registry.bind(name, 1, qh, ())),
                "wp_fractional_scale_manager_v1" => state.fractional_manager = Some(registry.bind(name, 1, qh, ())),
                "wp_viewporter" => state.viewporter = Some(registry.bind(name, 1, qh, ())),
                "wl_output" if state.output.is_none() => {
                    state.output = Some(registry.bind(name, version.min(4), qh, ()))
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(s: &mut Self, _: &wl_output::WlOutput, e: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Scale { factor } = e {
            s.scale = factor;
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        s: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        e: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, width, height } = e {
            layer.ack_configure(serial);
            s.configure = Some((width, height));
        }
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for State {
    fn event(
        s: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        e: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = e {
            s.preferred_scale = Some(scale);
        }
    }
}

wayland_client::delegate_noop!(State: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(State: wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
wayland_client::delegate_noop!(State: wp_viewporter::WpViewporter);
wayland_client::delegate_noop!(State: wp_viewport::WpViewport);
wayland_client::delegate_noop!(State: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(State: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

/// Nanoseconds on CPU summed over all threads of this process.
fn process_cpu_ns() -> u64 {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|t| std::fs::read_to_string(t.ok()?.path().join("schedstat")).ok())
        .filter_map(|s| s.split_whitespace().next()?.parse::<u64>().ok())
        .sum()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let scene = args.next().unwrap_or_else(|| "10".into());
    let ascii_dir = scene.strip_prefix("ascii").map(|rest| {
        rest.strip_prefix(':').map(std::path::PathBuf::from).unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../Sources/ParticleWall/Resources/SpiderManASCIIWallpaper")
        })
    });
    let model: u32 = if ascii_dir.is_some() { 0 } else { scene.parse().unwrap() };
    let seconds: f64 = args.next().map(|a| a.parse().unwrap()).unwrap_or(8.0);
    let fps: f64 = args.next().map(|a| a.parse().unwrap()).unwrap_or(30.0);
    let native = args.next().as_deref() != Some("integer");

    let conn = Connection::connect_to_env().expect("Wayland session");
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    let mut state = State { scale: 1, ..Default::default() };
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut state).unwrap();
    queue.roundtrip(&mut state).unwrap();

    let surface = state.compositor.as_ref().expect("wl_compositor").create_surface(&qh, ());
    let layer = state.shell.as_ref().expect("zwlr_layer_shell_v1").get_layer_surface(
        &surface,
        state.output.as_ref(),
        zwlr_layer_shell_v1::Layer::Bottom,
        "particlewall-live-check".into(),
        &qh,
        (),
    );
    use zwlr_layer_surface_v1::Anchor;
    layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
    let fractional = native
        .then(|| Some((state.fractional_manager.clone()?, state.viewporter.clone()?)))
        .flatten()
        .map(|(manager, viewporter)| {
            (manager.get_fractional_scale(&surface, &qh, ()), viewporter.get_viewport(&surface, &qh, ()))
        });
    if fractional.is_none() {
        surface.set_buffer_scale(state.scale.max(1));
    }
    surface.commit();
    while state.configure.is_none() || (fractional.is_some() && state.preferred_scale.is_none()) {
        queue.blocking_dispatch(&mut state).unwrap();
    }
    let (w, h) = state.configure.unwrap();
    let size = match (&fractional, state.preferred_scale) {
        (Some((_, viewport)), Some(scale)) => {
            viewport.set_destination(w as i32, h as i32);
            let px = |logical: u32| (u64::from(logical) * u64::from(scale)).div_ceil(120) as u32;
            (px(w), px(h))
        }
        _ => (w * state.scale.max(1) as u32, h * state.scale.max(1) as u32),
    };
    println!(
        "live: logical {w}x{h}, integer scale {}, preferred scale {:?}/120",
        state.scale, state.preferred_scale
    );

    let handles = WaylandHandles {
        display: conn.backend().display_ptr().cast(),
        surface: surface.id().as_ptr().cast(),
    };
    let mut shared = None;
    let mut presenter = GpuPresenter::new(&mut shared, handles, size).expect("presenter");
    let ctx = presenter.context().clone();
    let mut particles = ParticleRenderer::new(&ctx, presenter.format(), model, VertexPath::Indexed4);
    let mut ascii = ascii_dir.map(|dir| AsciiPlayer::new(&ctx, presenter.format(), &dir).expect("ASCII clip"));
    println!(
        "live: {}x{} {} at {fps} fps for {seconds} s on {}",
        size.0,
        size.1,
        if ascii.is_some() { "ascii".to_string() } else { format!("model {model}") },
        ctx.adapter_info().name
    );

    let mut u = Uniforms::defaults(size.0 as f32 / size.1 as f32, [size.0 as f32, size.1 as f32]);
    u.point_size = 1.25 * 1.6;
    let interval = Duration::from_secs_f64(1.0 / fps);
    let started = Instant::now();
    let cpu_start = process_cpu_ns();
    let mut frames = 0u32;
    let mut render_ns = 0u128;
    let mut last = Instant::now();
    while started.elapsed().as_secs_f64() < seconds {
        let now = Instant::now();
        let delta = (now - last).as_secs_f32().min(0.1);
        last = now;
        u.time += delta;
        u.model = [model as f32, particles.latest_flow_slot(), 12.0, Uniforms::model_vertex_count(model) as f32];
        let t = Instant::now();
        presenter.present(|encoder, view| match &mut ascii {
            Some(player) => {
                player.advance(delta);
                player.encode(encoder, view, size, &AsciiStyle::default());
            }
            None => particles.encode(encoder, view, &u, [0.02, 0.03, 0.08, 1.0], delta),
        });
        render_ns += t.elapsed().as_nanos();
        frames += 1;
        queue.dispatch_pending(&mut state).unwrap();
        conn.flush().ok();
        std::thread::sleep(interval);
    }
    let wall = started.elapsed().as_secs_f64();
    let cpu = (process_cpu_ns() - cpu_start) as f64 / 1e9;
    println!(
        "live: {frames} frames ({:.1} fps), CPU {:.2}% of one core, {:.0} us CPU/frame, render() call {:.0} us avg",
        frames as f64 / wall,
        cpu / wall * 100.0,
        cpu / frames as f64 * 1e6,
        render_ns as f64 / frames as f64 / 1e3,
    );
    drop((ascii, particles, presenter));
    layer.destroy();
    surface.destroy();
    conn.flush().ok();
}
