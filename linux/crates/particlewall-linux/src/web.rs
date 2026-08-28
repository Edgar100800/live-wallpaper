//! M0A/M1: wallpaper hosts for Hyprland/Wayland.
//!
//! Two renderer paths behind one control plane:
//! - Web: WebKitGTK loading the bundled HTML wallpapers (all eight models).
//! - GPU: wgpu/Vulkan presenting the shared particle-v1 WGSL modules
//!   (parametric-waves in M1) directly into the layer surface.

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, gdk as gtk_gdk, glib};
use ksni::blocking::TrayMethods;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;
use webkit6::prelude::*;
use webkit6::{
    NavigationPolicyDecision, PolicyDecisionType, UserContentInjectedFrames,
    UserScript, UserScriptInjectionTime, WebView,
};

#[path = "layer.rs"]
pub(crate) mod layer;
#[path = "control.rs"]
pub(crate) mod control;
#[path = "tray.rs"]
pub(crate) mod tray;
#[path = "library.rs"]
pub(crate) mod library;
#[cfg(feature = "gpu")]
#[path = "wayland.rs"]
pub(crate) mod wayland;

use control::{CmdTx, PlaybackFlags};
use tray::ParticleWallTray;

pub(crate) const DEFAULT_FPS_CAP: u32 = 30;

/// Bundled wallpaper id served by the shared GPU module (M1: one module).
#[cfg(feature = "gpu")]
const GPU_WALLPAPER_ID: &str = "DefaultWallpaper";

/// Repo-relative location of the shared JS contract scripts.
fn shared_script(name: &str) -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../shared/scripts/");
    std::fs::read_to_string(format!("{path}{name}"))
        .unwrap_or_else(|e| panic!("missing shared script {name}: {e}"))
}

/// Shared JS contract injection. Order matters: dpr-clamp, then the rAF gate,
/// then pause state and FPS cap (raf-patch initializes __pwFPSCap = 0).
fn user_scripts(paused: bool, fps_cap: u32) -> Vec<String> {
    let dpr = shared_script("dpr-clamp.js").replace("__PW_CAP__", "2.0");
    let raf = shared_script("raf-patch.js");
    let harden = shared_script("harden.js");
    let playback = format!("window.__pwPaused = {paused}; window.__pwFPSCap = {fps_cap};");

    vec![dpr, raf, playback, harden]
}

fn attach_scripts(webview: &WebView, paused: bool, fps_cap: u32) {
    let all = UserContentInjectedFrames::AllFrames;
    let start = UserScriptInjectionTime::Start;
    let scripts = user_scripts(paused, fps_cap);
    let ucm = webview
        .user_content_manager()
        .expect("WebView always has a UserContentManager");

    // All except harden.js run at document-start.
    for (i, source) in scripts.iter().enumerate() {
        let time = if i + 1 == scripts.len() {
            UserScriptInjectionTime::End
        } else {
            start
        };
        ucm.add_script(&UserScript::new(source, all, time, &[], &[]));
    }
}

/// Local-only policy mirroring WebViewFactory.swift via particlewall-contracts:
/// about/blob/data allowed; file URIs must stay inside the wallpaper root.
fn install_navigation_policy(webview: &WebView, root: &std::path::Path) {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    webview.connect_decide_policy(move |_, decision, decision_type| {
        if decision_type != PolicyDecisionType::NavigationAction {
            return false; // fall through to default handling
        }
        let Some(nav) = decision.clone().downcast::<NavigationPolicyDecision>().ok() else {
            return false;
        };
        let Some(action) = nav.navigation_action() else { return false };
        let Some(request) = action.request() else { return false };
        let Some(uri) = request.uri() else { return false };

        let allowed = glib::Uri::parse(&uri, glib::UriFlags::NONE)
            .map(|u| match u.scheme().as_str() {
                "about" | "blob" | "data" => true,
                "file" => particlewall_contracts::paths::is_within_root(
                    &root,
                    std::path::Path::new(u.path().as_str()),
                ),
                _ => false,
            })
            .unwrap_or(false);

        if allowed {
            false
        } else {
            println!("pw: blocked navigation to {uri}");
            decision.ignore();
            true
        }
    });
}

// ---------------------------------------------------------------------------
// GPU runtime (feature "gpu")
// ---------------------------------------------------------------------------

#[cfg(feature = "gpu")]
mod gpu {
    use super::*;
    pub(crate) use particlewall_render::gpu::GpuPresenter;
    use particlewall_render::Uniforms;

    pub struct GpuOutput {
        pub id: String,
        pub presenter: GpuPresenter,
        pub size_px: (f32, f32),
        /// Keepalive for the dedicated Wayland surface feeding `presenter`.
        /// Declared after the presenter so raw pointers are dropped first.
        #[allow(dead_code)]
        pub session: crate::web::wayland::GpuLayerSurface,
    }

    #[derive(Default)]
    pub struct GpuRuntime {
        pub outputs: Vec<GpuOutput>,
        /// Presenters parked while a web wallpaper is active (frozen frame,
        /// still mapped but covered by the GTK web windows).
        /// alive; destroying/recreating wgpu stacks on NVIDIA Wayland WSI is
        /// crash-prone).
        pub parked: Vec<GpuOutput>,
        /// Dedicated Wayland connection, created once per process.
        pub wl: Option<Rc<wayland::WaylandGpu>>,
        pub wallpaper: String,
        pub animation_time: f32,
        pub last_tick: Option<Instant>,
        pub background: [f64; 4],
        pub particle: [f32; 3],
        pub brightness: f32,
        pub particle_size: f32,
        pub speed: f32,
        pub loop_running: bool,
    }

    impl GpuRuntime {
        pub fn apply_config(&mut self, colors: &library::ColorSettings) {
            if let Some(bg) = colors.background {
                let c = packed_to_rgba01(bg);
                self.background = [c[0] as f64, c[1] as f64, c[2] as f64, 1.0];
            }
            if let Some(p) = colors.particle {
                self.particle = packed_to_rgba01(p)[..3].try_into().unwrap();
            }
            if let Some(s) = colors.size {
                self.particle_size = s as f32;
            }
        }

        fn uniforms_for(&self, out: &GpuOutput, aspect: f32) -> Uniforms {
            let mut u = Uniforms::defaults(aspect, [out.size_px.0, out.size_px.1]);
            u.time = self.animation_time;
            // Swift: max(1.25, min(3, scale * 1.25)) * particleSize, scale = 1 here.
            u.point_size = 1.25f32.clamp(1.25, 3.0) * self.particle_size;
            u.appearance = [
                self.particle[0],
                self.particle[1],
                self.particle[2],
                self.brightness,
            ];
            u.model = [0.0, 0.0, 12.0, Uniforms::VERTEX_COUNT_MODEL0 as f32];
            u
        }

        pub fn tick(&mut self, paused: bool) {
            let now = Instant::now();
            let delta = self
                .last_tick
                .map(|l| (now - l).as_secs_f32().min(0.1))
                .unwrap_or(0.0);
            self.last_tick = Some(now);
            if paused || self.outputs.is_empty() {
                return; // last frame stays on screen
            }
            self.animation_time += delta * self.speed;
            let n = self.outputs.len();
            for i in 0..n {
                let (u, background) = {
                    let out = &self.outputs[i];
                    let aspect = out.size_px.0 / out.size_px.1.max(1.0);
                    (self.uniforms_for(out, aspect), self.background)
                };
                self.outputs[i].presenter.render(&u, background);
            }
        }
    }

    pub fn packed_to_rgba01(packed: u32) -> [f32; 4] {
        [
            ((packed >> 16) & 0xFF) as f32 / 255.0,
            ((packed >> 8) & 0xFF) as f32 / 255.0,
            (packed & 0xFF) as f32 / 255.0,
            1.0,
        ]
    }
}

// ---------------------------------------------------------------------------
// Daemon
// ---------------------------------------------------------------------------

/// Builds (or rebuilds) the renderer children of every window for the given
/// wallpaper. GPU-capable modules go to wgpu; everything else to WebKitGTK.
fn switch_renderer(
    state: &Rc<RefCell<control::DaemonState>>,
    gpu_rt: &Rc<RefCell<gpu::GpuRuntime>>,
    wallpaper_id: &str,
) -> String {
    #[cfg(feature = "gpu")]
    let gpu_capable = wallpaper_id == GPU_WALLPAPER_ID;

    #[cfg(not(feature = "gpu"))]
    let gpu_capable = false;

    let windows: Vec<(String, ApplicationWindow)> = state.borrow().windows.clone();

    if gpu_capable {
        // Tear down web children.
        let webviews = std::mem::take(&mut state.borrow_mut().webviews);
        drop(webviews);

        // Idempotency: already presenting this wallpaper on the GPU.
        {
            let rt = gpu_rt.borrow();
            if !rt.outputs.is_empty() && rt.wallpaper == wallpaper_id {
                return format!("{{\"renderer\":\"gpu\",\"wallpaper\":\"{wallpaper_id}\"}}\n");
            }
        }

        // Dedicated Wayland connection, kept alive for the whole process:
        // tearing down the NVIDIA WSI state behind it is crash-prone.
        let existing_wl = gpu_rt.borrow().wl.clone();
        let wl = match existing_wl {
            Some(wl) => Some(wl),
            None => match wayland::WaylandGpu::connect() {
                Ok(w) => {
                    let w = Rc::new(w);
                    gpu_rt.borrow_mut().wl = Some(w.clone());
                    Some(w)
                }
                Err(e) => {
                    eprintln!("pw: GPU Wayland connection failed: {e}");
                    None
                }
            },
        };

        let mut outputs: Vec<gpu::GpuOutput> = Vec::new();
        let mut gpu_err: Option<String> = None;
        if let Some(wl) = wl {
            for name in wl.output_names() {
                // Reuse a parked presenter when one exists for this output.
                // Parked surfaces stay mapped (frozen frame) and hidden
                // behind the GTK web windows; z-order = map order.
                let parked_idx = gpu_rt.borrow().parked.iter().position(|p| p.id == name);
                if let Some(idx) = parked_idx {
                    let out = gpu_rt.borrow_mut().parked.remove(idx);
                    println!("pw: GPU renderer resumed on {name}");
                    outputs.push(out);
                    continue;
                }
                let built = wl.create_surface(&name).and_then(|session| {
                    let size = session.size_px;
                    gpu::GpuPresenter::new(session.handles(), size)
                        .map(|presenter| (session, presenter))
                        .map_err(|e| e.to_string())
                });
                match built {
                    Ok((session, presenter)) => {
                        println!("pw: GPU renderer on {name}");
                        outputs.push(gpu::GpuOutput {
                            id: name.clone(),
                            size_px: (session.size_px.0 as f32, session.size_px.1 as f32),
                            presenter,
                            session,
                        });
                    }
                    Err(e) => {
                        eprintln!("pw: GPU unavailable on {name} ({e})");
                        gpu_err = Some(e);
                        break;
                    }
                }
            }
        }

        if !outputs.is_empty() && gpu_err.is_none() {
            // Hide the (empty) GTK layer windows so they don't cover the GPU
            // surfaces on the same Background layer.
            for (_, w) in &windows {
                w.set_visible(false);
            }
            let mut rt = gpu_rt.borrow_mut();
            rt.outputs = outputs;
            rt.wallpaper = wallpaper_id.to_string();
            rt.last_tick = None;
            if !rt.loop_running {
                rt.loop_running = true;
                start_gpu_loop(state.clone(), gpu_rt.clone());
            }
            format!("{{\"renderer\":\"gpu\",\"wallpaper\":\"{wallpaper_id}\"}}\n")
        } else {
            // Park whatever got built before falling back to web. Surfaces
            // stay mapped; the freshly mapped GTK web windows cover them.
            for out in outputs {
                gpu_rt.borrow_mut().parked.push(out);
            }
            eprintln!("pw: GPU renderer unavailable; falling back to web");
            for (id, window) in &windows {
                window.set_visible(true);
                spawn_web_child(state, window, id, wallpaper_id);
            }
            format!("{{\"renderer\":\"web\",\"wallpaper\":\"{wallpaper_id}\"}}\n")
        }
    } else {
        // Park (don't destroy) GPU presenters; see GpuRuntime::parked. The
        // surfaces keep their last frozen frame, hidden behind the GTK web
        // windows that get (re)mapped here and therefore stack on top.
        {
            let mut rt = gpu_rt.borrow_mut();
            let mut actives = std::mem::take(&mut rt.outputs);
            rt.parked.append(&mut actives);
            rt.wallpaper.clear();
        }
        for (_, w) in &windows {
            w.set_visible(true);
        }
        for (id, window) in &windows {
            spawn_web_child(state, window, id, wallpaper_id);
        }
        format!("{{\"renderer\":\"web\",\"wallpaper\":\"{wallpaper_id}\"}}\n")
    }
}

fn spawn_web_child(
    state: &Rc<RefCell<control::DaemonState>>,
    window: &ApplicationWindow,
    id: &str,
    wallpaper_id: &str,
) {
    let Some(wp) = library::find(wallpaper_id) else { return };
    let resources_root = library::resources_dir();

    let webview = WebView::builder().build();
    attach_scripts(
        &webview,
        state.borrow().flags.paused.load(std::sync::atomic::Ordering::Relaxed),
        state.borrow().flags.fps_cap.load(std::sync::atomic::Ordering::Relaxed),
    );
    webview.set_background_color(&gtk_gdk::RGBA::BLACK);

    // Re-apply the saved appearance once each document finishes loading.
    {
        let st = state.clone();
        webview.connect_load_changed(move |wv, event| {
            if event == webkit6::LoadEvent::Finished {
                if let Some(js) = st.borrow().colors_js() {
                    wv.evaluate_javascript(
                        &js,
                        None,
                        None,
                        None::<&gtk4::gio::Cancellable>,
                        |_| {},
                    );
                }
            }
        });
    }

    install_navigation_policy(&webview, &resources_root);
    webview.load_uri(&format!("file://{}", wp.index.display()));

    window.set_child(Some(&webview));
    state.borrow_mut().webviews.push((id.to_string(), webview));
}

#[cfg(feature = "gpu")]
fn start_gpu_loop(
    state: Rc<RefCell<control::DaemonState>>,
    gpu_rt: Rc<RefCell<gpu::GpuRuntime>>,
) {
    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        let fps = state.borrow().flags.fps_cap.load(std::sync::atomic::Ordering::Relaxed);
        let paused = state.borrow().flags.paused.load(std::sync::atomic::Ordering::Relaxed);
        // Skip work when paused; re-check interval needs are handled by the
        // 16ms cadence (cheap when idle: no GPU submission).
        let _ = fps;
        gpu_rt.borrow_mut().tick(paused);
        let still_gpu = !gpu_rt.borrow().outputs.is_empty();
        if still_gpu {
            glib::ControlFlow::Continue
        } else {
            let mut rt = gpu_rt.borrow_mut();
            rt.loop_running = false;
            glib::ControlFlow::Break
        }
    });
}

pub fn run() {
    let app = Application::builder()
        .application_id("com.particlewall.daemon")
        .build();

    app.connect_activate(move |app| {
        let app = app.clone();
        let flags = PlaybackFlags::default();

        // Initial state: persisted configuration or sensible defaults.
        let cfg = library::load_config();
        let initial = cfg
            .wallpaper
            .clone()
            .unwrap_or_else(|| "DefaultWallpaper".into());
        let initial_wp = library::find(&initial)
            .or_else(|| library::find("DefaultWallpaper"))
            .expect("bundled wallpapers present");
        let current_id = Arc::new(std::sync::Mutex::new(initial_wp.id.clone()));
        let profiles_ui = Arc::new(std::sync::Mutex::new(
            cfg.profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
        ));

        let state = Rc::new(RefCell::new(control::DaemonState {
            webviews: Vec::new(),
            flags: flags.clone(),
            wallpaper: initial_wp.id.clone(),
            colors: cfg.colors.clone(),
            profiles: cfg.profiles.clone(),
            current: Some(current_id.clone()),
            profiles_ui: Some(profiles_ui.clone()),
            tray: None,
            windows: Vec::new(),
            switch: None,
            on_colors: None,
            output_count: None,
        }));

        let gpu_rt = Rc::new(RefCell::new(gpu::GpuRuntime {
            background: {
                let c = gpu::packed_to_rgba01(cfg.colors.background.unwrap_or(198153));
                [c[0] as f64, c[1] as f64, c[2] as f64, 1.0]
            },
            particle: gpu::packed_to_rgba01(cfg.colors.particle.unwrap_or(15269887))[..3]
                .try_into()
                .unwrap(),
            particle_size: cfg.colors.size.unwrap_or(1.6) as f32,
            brightness: 1.5,
            speed: 1.0,
            ..Default::default()
        }));

        // Renderer switch hook used by the Apply command (main thread).
        {
            let st = state.clone();
            let rt = gpu_rt.clone();
            state.borrow_mut().switch = Some(Rc::new(move |wallpaper_id: &str| {
                switch_renderer(&st, &rt, wallpaper_id)
            }));
        }

        // Appearance hook: colors/size also reach the GPU runtime.
        #[cfg(feature = "gpu")]
        {
            let rt = gpu_rt.clone();
            state.borrow_mut().on_colors = Some(Rc::new(move |colors: &library::ColorSettings| {
                rt.borrow_mut().apply_config(colors);
            }));
        }

        // Status reports the live renderer's outputs (GPU when active,
        // webviews otherwise).
        #[cfg(feature = "gpu")]
        {
            let st = state.clone();
            let rt = gpu_rt.clone();
            state.borrow_mut().output_count = Some(Rc::new(move || {
                if !rt.borrow().outputs.is_empty() {
                    rt.borrow().outputs.len()
                } else {
                    st.borrow().webviews.len()
                }
            }));
        }

        // Command bus shared by the CLI socket and the tray menu.
        let (tx, rx): (CmdTx, _) = async_channel::unbounded();

        // System tray in the top bar (volume/bluetooth area).
        let tray_result = ParticleWallTray {
            flags: flags.clone(),
            tx: tx.clone(),
            current: current_id.clone(),
            profiles: profiles_ui.clone(),
        }
        .spawn();
        match tray_result {
            Ok(handle) => state.borrow_mut().tray = Some(handle),
            Err(e) => eprintln!("pw: tray unavailable: {e}"),
        }

        // Windows for every output; children come from switch_renderer.
        for monitor in layer::monitors() {
            let id = layer::monitor_identity(&monitor);
            let geometry = monitor.geometry();
            println!("pw: output {id} {}x{}", geometry.width(), geometry.height());

            let window = ApplicationWindow::builder().application(&app).build();
            layer::setup_layer_window(&window);
            window.present();
            state.borrow_mut().windows.push((id.clone(), window));
        }

        // Initial renderer once surfaces are mapped.
        {
            let st = state.clone();
            let rt = gpu_rt.clone();
            let wp_id = initial_wp.id.clone();
            glib::idle_add_local_once(move || {
                let reply = switch_renderer(&st, &rt, &wp_id);
                println!("pw: {reply}");
            });
        }

        // CLI socket + applier loop.
        if let Err(e) = control::start(state.clone(), app, tx, rx) {
            eprintln!("pw: control socket unavailable: {e}");
        }
    });

    let _ = app.run();
}
