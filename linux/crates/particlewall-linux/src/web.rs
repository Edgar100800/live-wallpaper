//! M0A/M1: wallpaper hosts for Hyprland/Wayland.
//!
//! Two renderer paths behind one control plane:
//! - Web (feature "webkit"): WebKitGTK loading HTML and ASCII-video
//!   wallpapers.
//! - GPU (feature "gpu"): wgpu/Vulkan presenting the shared particle-v1 WGSL
//!   modules directly into dedicated layer surfaces.
//!
//! Without "webkit" the daemon, settings window, tray and CLI still run;
//! only GPU wallpapers are listed and applied.

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, glib};
#[cfg(feature = "webkit")]
use gtk4::gdk as gtk_gdk;
use ksni::blocking::TrayMethods;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;
#[cfg(feature = "webkit")]
use webkit6::prelude::*;
#[cfg(feature = "webkit")]
use webkit6::{
    NavigationPolicyDecision, PolicyDecisionType, UserContentInjectedFrames,
    Settings, UserScript, UserScriptInjectionTime, WebView,
};

#[path = "layer.rs"]
pub(crate) mod layer;
#[path = "control.rs"]
pub(crate) mod control;
#[path = "tray.rs"]
pub(crate) mod tray;
#[path = "library.rs"]
pub(crate) mod library;
#[path = "reprocess_ui.rs"]
pub(crate) mod reprocess_ui;
#[path = "import_ui.rs"]
pub(crate) mod import_ui;
#[path = "importer.rs"]
pub(crate) mod importer;
#[path = "omarchy_palette.rs"]
pub(crate) mod omarchy_palette;
#[cfg(feature = "gpu")]
#[path = "wayland.rs"]
pub(crate) mod wayland;

use control::{CmdTx, PlaybackFlags};
use tray::ParticleWallTray;

pub(crate) const DEFAULT_FPS_CAP: u32 = 30;

/// Bundled wallpaper id -> shared GPU model index (particle-v1 engine).
/// Mirrors LibraryManager.bundledWallpaperSpecs on macOS.
#[cfg(feature = "gpu")]
fn gpu_model_for(wallpaper_id: &str) -> Option<u32> {
    Some(match wallpaper_id {
        "DefaultWallpaper" => 0,
        "TwinVortexWallpaper" => 1,
        "OrbitalBloomWallpaper" => 2,
        "HexagonalRosetteWallpaper" => 3,
        "NoiseRainWallpaper" => 4,
        "PrimeSpiralWallpaper" => 5,
        "TorusOrbitWallpaper" => 6,
        "ChromaticRingsWallpaper" => 7,
        "SphereTorusWallpaper" => 8,
        "JellyfishPointsWallpaper" => 9,
        "NebulaWallpaper" => 10,
        "TorusKnotWallpaper" => 11,
        _ => return None,
    })
}

/// How the GPU runtime renders `wallpaper_id`, if it can: a particle-v1
/// model or a native ascii-video-v1 clip. Everything else needs WebKit.
#[cfg(feature = "gpu")]
fn native_wallpaper(wallpaper_id: &str) -> Option<gpu::Native> {
    if let Some(model) = gpu_model_for(wallpaper_id) {
        return Some(gpu::Native::Particles(model));
    }
    library::find(wallpaper_id)
        .and_then(|wp| library::ascii_clip_dir(&wp))
        .map(gpu::Native::Ascii)
}

#[cfg(feature = "webkit")]
/// Repo-relative location of the shared JS contract scripts.
fn shared_script(name: &str) -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../shared/scripts/");
    std::fs::read_to_string(format!("{path}{name}"))
        .unwrap_or_else(|e| panic!("missing shared script {name}: {e}"))
}

#[cfg(feature = "webkit")]
/// Shared JS contract injection. Order matters: dpr-clamp, then the rAF gate,
/// then pause state and FPS cap (raf-patch initializes __pwFPSCap = 0).
fn user_scripts(paused: bool, fps_cap: u32) -> Vec<String> {
    let dpr = shared_script("dpr-clamp.js").replace("__PW_CAP__", "2.0");
    let raf = shared_script("raf-patch.js");
    let harden = shared_script("harden.js");
    let playback = format!("window.__pwPaused = {paused}; window.__pwFPSCap = {fps_cap};");

    let palette = omarchy_palette::script(omarchy_palette::read().as_ref());
    vec![dpr, raf, playback, palette, harden]
}

#[cfg(feature = "webkit")]
fn attach_scripts(webview: &WebView, paused: bool, fps_cap: u32, ascii: &library::ASCIISettings) {
    let all = UserContentInjectedFrames::AllFrames;
    let start = UserScriptInjectionTime::Start;
    let scripts = user_scripts(paused, fps_cap);
    let ucm = webview
        .user_content_manager()
        .expect("WebView always has a UserContentManager");

    ucm.add_script(&UserScript::new(&ascii.script(), all, start, &[], &[]));

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

#[cfg(feature = "webkit")]
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
                ) || particlewall_contracts::paths::is_within_root(&library::user_wallpapers_dir(), std::path::Path::new(u.path().as_str())),
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
    use particlewall_render::ascii::{AsciiPalette, AsciiPlayer, AsciiStyle};
    use particlewall_render::gpu::{GpuContext, ParticleRenderer, VertexPath};
    use particlewall_render::Uniforms;

    /// A wallpaper the GPU runtime renders without WebKit.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Native {
        /// Shared particle-v1 model index.
        Particles(u32),
        /// ascii-video-v1 clip directory.
        Ascii(std::path::PathBuf),
    }

    pub enum Scene {
        Particles(ParticleRenderer),
        Ascii(AsciiPlayer),
    }

    impl Scene {
        pub fn new(ctx: &Rc<GpuContext>, format: particlewall_render::wgpu::TextureFormat, native: &Native) -> Result<Self, String> {
            Ok(match native {
                Native::Particles(model) => {
                    Scene::Particles(ParticleRenderer::new(ctx, format, *model, VertexPath::Indexed4))
                }
                Native::Ascii(dir) => Scene::Ascii(AsciiPlayer::new(ctx, format, dir)?),
            })
        }
    }

    pub struct GpuOutput {
        pub id: String,
        pub presenter: GpuPresenter,
        pub scene: Scene,
        pub size_px: (f32, f32),
        /// Native/integer-scale buffer ratio (GpuLayerSurface::point_scale).
        pub point_scale: f32,
        /// When the frame callback for the last present was requested;
        /// None once answered (or when pacing by timer).
        pub frame_requested: Option<Instant>,
        pub last_draw: Option<Instant>,
        /// Something visible changed: draw once even while paused.
        pub dirty: bool,
        /// Keepalive for the dedicated Wayland surface feeding `presenter`.
        /// Declared after the presenter so raw pointers are dropped first.
        pub session: crate::web::wayland::GpuLayerSurface,
    }

    /// How long a frame callback may stay unanswered before the output is
    /// drawn anyway. Compositors may withhold callbacks for surfaces they
    /// are not showing; this keeps ~1 fps there instead of stalling.
    const FRAME_CALLBACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

    impl GpuOutput {
        /// Points this output at `native` without touching its swapchain:
        /// particle models switch in place, an ASCII clip keeps playing when
        /// it is already shown, anything else gets a new scene.
        pub fn show(&mut self, native: &Native) -> Result<(), String> {
            match (native, &mut self.scene) {
                (Native::Particles(model), Scene::Particles(renderer)) => renderer.set_model(*model),
                (Native::Ascii(dir), Scene::Ascii(player)) if player.dir() == dir => {}
                _ => {
                    let ctx = self.presenter.context().clone();
                    self.scene = Scene::new(&ctx, self.presenter.format(), native)?;
                }
            }
            Ok(())
        }
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
        /// One wgpu device + pipelines shared by every output's presenter.
        pub ctx: Option<Rc<GpuContext>>,
        pub wallpaper: String,
        pub animation_time: f32,
        pub last_tick: Option<Instant>,
        pub background: [f64; 4],
        pub particle: [f32; 3],
        pub brightness: f32,
        pub particle_size: f32,
        pub speed: f32,
        pub loop_running: bool,
        /// Omarchy theme palette, refreshed by the palette watcher.
        pub palette: Option<AsciiPalette>,
        /// ASCII presentation used for the last frame.
        pub ascii_style: Option<AsciiStyle>,
        /// Playback clock for ASCII clips (wall time, not speed-scaled),
        /// shared so every output shows the same clip frame.
        pub clip_time: f64,
        /// Deadline of the pending one-shot wake-up for an idle ASCII output.
        pub kick_at: Option<Instant>,
    }

    impl GpuRuntime {
        pub fn apply_config(&mut self, colors: &library::ColorSettings) {
            let colors = colors.resolved();
            if let Some(bg) = colors.background {
                let c = packed_to_rgba01(bg);
                self.background = [c[0] as f64, c[1] as f64, c[2] as f64, 1.0];
            }
            if let Some(p) = colors.particle {
                self.particle = packed_to_rgba01(p)[..3].try_into().unwrap();
            }
            if let Some(s) = colors.size {
                self.particle_size =
                    (s as f32).clamp(0.25, 8.0);
            }
            if let Some(w) = colors.brightness {
                self.brightness = (w as f32).clamp(0.25, 10.0);
            }
            self.mark_dirty();
        }

        /// Repaints every output once, even while paused.
        pub fn mark_dirty(&mut self) {
            for out in &mut self.outputs {
                out.dirty = true;
            }
        }

        fn uniforms_for(&self, out: &GpuOutput, renderer: &ParticleRenderer) -> Uniforms {
            let aspect = out.size_px.0 / out.size_px.1.max(1.0);
            let mut u = Uniforms::defaults(aspect, [out.size_px.0, out.size_px.1]);
            u.time = self.animation_time;
            // Swift: max(1.25, min(3, scale * 1.25)) * particleSize, scale = 1 here.
            // point_scale keeps the on-screen size of the former integer-scale
            // buffers now that fractional outputs render at native size.
            u.point_size = 1.25f32.clamp(1.25, 3.0) * self.particle_size * out.point_scale;
            u.appearance = [
                self.particle[0],
                self.particle[1],
                self.particle[2],
                self.brightness,
            ];
            let model = renderer.model();
            u.model = [
                model as f32,
                renderer.latest_flow_slot(),
                12.0,
                Uniforms::model_vertex_count(model) as f32,
            ];
            u
        }

        /// Draws every output that is due. While paused nothing advances and
        /// the last frame stays on screen, except that appearance, palette or
        /// wallpaper changes repaint it once. ASCII outputs only present when
        /// the clip frame changes.
        ///
        /// `sync` paces by frame callbacks (FPS cap 0): an output draws again
        /// only after the compositor answered its last present, i.e. at its
        /// monitor's refresh rate. Returns how long until an idle ASCII
        /// output needs its next clip frame, for the caller to wake up then.
        pub fn tick(
            &mut self,
            paused: bool,
            ascii: &library::ASCIISettings,
            sync: bool,
        ) -> Option<std::time::Duration> {
            let style = AsciiStyle {
                palette: (ascii.color_mode == library::ASCIIColorMode::Omarchy)
                    .then_some(self.palette)
                    .flatten(),
                clean_background: ascii.clean_background,
                separation: ascii.separation as f32,
            };
            if self.ascii_style != Some(style) {
                self.ascii_style = Some(style);
                self.mark_dirty();
            }
            let now = Instant::now();
            let elapsed = self
                .last_tick
                .map(|l| (now - l).as_secs_f32().min(0.1))
                .unwrap_or(0.0);
            self.last_tick = Some(now);
            if !paused {
                self.animation_time += elapsed * self.speed;
                self.clip_time += f64::from(elapsed);
            }
            let (background, clip_time) = (self.background, self.clip_time);
            let mut wake: Option<std::time::Duration> = None;
            for i in 0..self.outputs.len() {
                let out = &self.outputs[i];
                let waiting = out
                    .frame_requested
                    .is_some_and(|requested| now - requested < FRAME_CALLBACK_TIMEOUT);
                if sync && waiting {
                    continue;
                }
                let due = match &out.scene {
                    Scene::Particles(_) => !paused || out.dirty,
                    Scene::Ascii(player) => {
                        let due = out.dirty || player.changes_at(clip_time);
                        if !due && !paused {
                            let next = player.until_next_frame(clip_time);
                            wake = Some(wake.map_or(next, |w| w.min(next)));
                        }
                        due
                    }
                };
                if !due {
                    continue;
                }
                let uniforms = match &out.scene {
                    Scene::Particles(renderer) => Some(self.uniforms_for(out, renderer)),
                    Scene::Ascii(_) => None,
                };
                let delta = match (paused, out.last_draw) {
                    (false, Some(last)) => (now - last).as_secs_f32().min(0.1),
                    _ => 0.0,
                };
                let scaled = delta * self.speed;
                let out = &mut self.outputs[i];
                out.dirty = false;
                out.last_draw = Some(now);
                out.frame_requested = sync.then(|| {
                    out.session.request_frame();
                    now
                });
                let GpuOutput { presenter, scene, .. } = out;
                let size = presenter.size();
                presenter.present(|encoder, view| match scene {
                    Scene::Particles(renderer) => {
                        renderer.encode(encoder, view, uniforms.as_ref().unwrap(), background, scaled)
                    }
                    Scene::Ascii(player) => {
                        player.seek(clip_time);
                        player.encode(encoder, view, size, &style);
                    }
                });
            }
            wake
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
    let native = native_wallpaper(wallpaper_id);
    #[cfg(feature = "gpu")]
    let gpu_capable = native.is_some();

    #[cfg(not(feature = "gpu"))]
    let gpu_capable = false;

    #[cfg(not(feature = "webkit"))]
    if !gpu_capable {
        return format!("{{\"error\":\"'{wallpaper_id}' needs a build with the webkit feature\"}}\n");
    }

    let windows: Vec<(String, ApplicationWindow)> = state.borrow().windows.clone();

    if gpu_capable {
        clear_web_children(state, &windows);

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
                    watch_frame_callbacks(state, gpu_rt, &w);
                    Some(w)
                }
                Err(e) => {
                    eprintln!("pw: GPU Wayland connection failed: {e}");
                    None
                }
            },
        };

        let native = native.expect("gpu_capable");
        let mut outputs: Vec<gpu::GpuOutput> = Vec::new();
        let mut gpu_err: Option<String> = None;
        if let Some(wl) = wl {
            for name in wl.output_names() {
                // Presenters are permanent per-output resources (the NVIDIA
                // Wayland WSI cannot host a second swapchain on the same
                // connection). Reuse an active presenter first, then a
                // parked one, pointing its scene at the new wallpaper; only
                // build brand-new surfaces when none exists for this output.
                let active = {
                    let mut rt = gpu_rt.borrow_mut();
                    rt.outputs.iter().position(|p| p.id == name).map(|idx| (rt.outputs.remove(idx), "reconfigured"))
                };
                let reused = active.or_else(|| {
                    let mut rt = gpu_rt.borrow_mut();
                    rt.parked.iter().position(|p| p.id == name).map(|idx| (rt.parked.remove(idx), "resumed"))
                });
                if let Some((mut out, how)) = reused {
                    if let Err(e) = out.show(&native) {
                        eprintln!("pw: {wallpaper_id} unavailable on {name} ({e})");
                        gpu_rt.borrow_mut().parked.push(out);
                        gpu_err = Some(e);
                        break;
                    }
                    println!("pw: GPU renderer {how} on {name}");
                    outputs.push(out);
                    continue;
                }
                let built = wl.create_surface(&name).and_then(|session| {
                    let size = session.size_px;
                    let mut shared_ctx = gpu_rt.borrow().ctx.clone();
                    let presenter =
                        gpu::GpuPresenter::new(&mut shared_ctx, session.handles(), size)
                            .map_err(|e| e.to_string());
                    gpu_rt.borrow_mut().ctx = shared_ctx;
                    let presenter = presenter?;
                    let scene = gpu::Scene::new(presenter.context(), presenter.format(), &native)?;
                    Ok((session, presenter, scene))
                });
                match built {
                    Ok((session, presenter, scene)) => {
                        println!(
                            "pw: GPU renderer on {name} ({}x{} px)",
                            session.size_px.0, session.size_px.1
                        );
                        outputs.push(gpu::GpuOutput {
                            id: name.clone(),
                            presenter,
                            scene,
                            size_px: (session.size_px.0 as f32, session.size_px.1 as f32),
                            point_scale: session.point_scale,
                            frame_requested: None,
                            last_draw: None,
                            dirty: true,
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
            // surfaces on the same Bottom layer.
            for (_, w) in &windows {
                w.set_visible(false);
            }
            let mut rt = gpu_rt.borrow_mut();
            rt.outputs = outputs;
            if rt.wallpaper != wallpaper_id {
                rt.clip_time = 0.0;
            }
            rt.wallpaper = wallpaper_id.to_string();
            rt.last_tick = None;
            rt.mark_dirty();
            // Reused presenters may still wait on a callback from before.
            for out in &mut rt.outputs {
                out.frame_requested = None;
            }
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
            #[cfg(feature = "webkit")]
            {
                eprintln!("pw: GPU renderer unavailable; falling back to web");
                for (id, window) in &windows {
                    window.set_visible(true);
                    spawn_web_child(state, window, id, wallpaper_id);
                }
                format!("{{\"renderer\":\"web\",\"wallpaper\":\"{wallpaper_id}\"}}\n")
            }
            #[cfg(not(feature = "webkit"))]
            {
                eprintln!("pw: GPU renderer unavailable and no web fallback in this build");
                format!("{{\"error\":\"GPU renderer unavailable: {}\"}}\n", gpu_err.unwrap_or_default())
            }
        }
    } else {
        #[cfg(not(feature = "webkit"))]
        unreachable!("non-GPU wallpapers return early without webkit");
        #[cfg(feature = "webkit")]
        {
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
}

fn clear_web_children(
    state: &Rc<RefCell<control::DaemonState>>,
    windows: &[(String, ApplicationWindow)],
) {
    #[allow(unused_variables)]
    let webviews = std::mem::take(&mut state.borrow_mut().webviews);
    #[cfg(feature = "webkit")]
    for (_, webview) in &webviews {
        webview.stop_loading();
    }
    for (_, window) in windows {
        window.set_child(None::<&gtk4::Widget>);
    }
}

#[cfg(feature = "webkit")]
fn spawn_web_child(
    state: &Rc<RefCell<control::DaemonState>>,
    window: &ApplicationWindow,
    id: &str,
    wallpaper_id: &str,
) {
    let Some(wp) = library::find(wallpaper_id) else { return };
    let resources_root = library::resources_dir();

    // Reuse the document host for this output instead of retaining one per apply.
    let existing = state.borrow().webviews.iter()
        .find(|(output, _)| output == id)
        .map(|(_, webview)| webview.clone());
    if let Some(webview) = existing {
        webview.user_content_manager().unwrap().remove_all_scripts();
        attach_scripts(
            &webview,
            state.borrow().flags.effective_paused(),
            state.borrow().flags.fps_cap.load(std::sync::atomic::Ordering::Relaxed),
            &state.borrow().ascii_settings(wallpaper_id),
        );
        webview.load_uri(&gtk4::gio::File::for_path(&wp.index).uri());
        return;
    }

    let settings = Settings::builder()
        .allow_file_access_from_file_urls(true)
        .enable_write_console_messages_to_stdout(true)
        .build();
    let webview = WebView::builder().settings(&settings).build();
    attach_scripts(
        &webview,
        state.borrow().flags.effective_paused(),
        state.borrow().flags.fps_cap.load(std::sync::atomic::Ordering::Relaxed),
        &state.borrow().ascii_settings(wallpaper_id),
    );
    webview.set_background_color(&gtk_gdk::RGBA::BLACK);

    // Re-apply the saved appearance once each document finishes loading.
    {
        let st = Rc::downgrade(state);
        webview.connect_load_changed(move |wv, event| {
            if event == webkit6::LoadEvent::Finished {
                let Some(st) = st.upgrade() else { return };
                let js = st.borrow().colors_js();
                if let Some(js) = js {
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
    webview.load_uri(&gtk4::gio::File::for_path(&wp.index).uri());

    window.set_child(Some(&webview));
    state.borrow_mut().webviews.push((id.to_string(), webview));
}

/// Draws whatever is due. With the FPS cap at 0 rendering follows each
/// monitor's frame callbacks; an idle ASCII output gets a one-shot wake-up at
/// its next clip frame.
#[cfg(feature = "gpu")]
fn run_tick(state: &Rc<RefCell<control::DaemonState>>, gpu_rt: &Rc<RefCell<gpu::GpuRuntime>>) {
    let (paused, ascii, sync) = {
        let st = state.borrow();
        (
            st.flags.effective_paused(),
            st.ascii_settings(&st.wallpaper),
            st.flags.fps_cap.load(std::sync::atomic::Ordering::Relaxed) == 0,
        )
    };
    let wake = gpu_rt.borrow_mut().tick(paused, &ascii, sync);
    if let (true, Some(delay)) = (sync, wake) {
        wake_after(state, gpu_rt, delay);
    }
    log_render_rate(gpu_rt);
}

/// Schedules one `run_tick` after `delay`, keeping only the soonest pending
/// wake-up.
#[cfg(feature = "gpu")]
fn wake_after(
    state: &Rc<RefCell<control::DaemonState>>,
    gpu_rt: &Rc<RefCell<gpu::GpuRuntime>>,
    delay: std::time::Duration,
) {
    // glib timers have millisecond resolution; never wake before the frame.
    let delay = delay + std::time::Duration::from_millis(1);
    let at = Instant::now() + delay;
    {
        let mut rt = gpu_rt.borrow_mut();
        if rt.kick_at.is_some_and(|pending| pending <= at) {
            return;
        }
        rt.kick_at = Some(at);
    }
    let (st, rt) = (Rc::downgrade(state), Rc::downgrade(gpu_rt));
    glib::timeout_add_local_once(delay, move || {
        let (Some(st), Some(rt)) = (st.upgrade(), rt.upgrade()) else { return };
        // A sooner wake-up replaced this one.
        if rt.borrow().kick_at != Some(at) {
            return;
        }
        rt.borrow_mut().kick_at = None;
        run_tick(&st, &rt);
    });
}

/// Feeds frame callbacks from the dedicated Wayland connection into the
/// render loop: each answered callback lets its output draw again.
#[cfg(feature = "gpu")]
fn watch_frame_callbacks(
    state: &Rc<RefCell<control::DaemonState>>,
    gpu_rt: &Rc<RefCell<gpu::GpuRuntime>>,
    wl: &Rc<wayland::WaylandGpu>,
) {
    let (st, rt, wl_weak) = (Rc::downgrade(state), Rc::downgrade(gpu_rt), Rc::downgrade(wl));
    let condition = glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR;
    glib_unix::unix_fd_add_local(wl.poll_fd(), condition, move |_, condition| {
        let (Some(st), Some(rt), Some(wl)) = (st.upgrade(), rt.upgrade(), wl_weak.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        if let Err(e) = wl.dispatch() {
            eprintln!("pw: GPU Wayland connection lost: {e}");
            return glib::ControlFlow::Break;
        }
        if condition.intersects(glib::IOCondition::HUP | glib::IOCondition::ERR) {
            return glib::ControlFlow::Break;
        }
        let mut answered = false;
        for out in rt.borrow_mut().outputs.iter_mut() {
            if out.session.take_frame_done() {
                out.frame_requested = None;
                answered = true;
            }
        }
        if answered {
            run_tick(&st, &rt);
        }
        glib::ControlFlow::Continue
    });
}

/// `PW_LOG_FPS=1`: prints the presented frames per second every 5 s.
#[cfg(feature = "gpu")]
fn log_render_rate(gpu_rt: &Rc<RefCell<gpu::GpuRuntime>>) {
    thread_local! {
        static ENABLED: bool = std::env::var_os("PW_LOG_FPS").is_some();
        static WINDOW: RefCell<(Option<Instant>, Vec<(String, Option<Instant>, u32)>)> =
            const { RefCell::new((None, Vec::new())) };
    }
    if !ENABLED.with(|e| *e) {
        return;
    }
    WINDOW.with(|window| {
        let mut window = window.borrow_mut();
        let now = Instant::now();
        let start = *window.0.get_or_insert(now);
        for out in &gpu_rt.borrow().outputs {
            let entry = match window.1.iter().position(|(id, _, _)| *id == out.id) {
                Some(i) => &mut window.1[i],
                None => {
                    window.1.push((out.id.clone(), None, 0));
                    window.1.last_mut().unwrap()
                }
            };
            if out.last_draw.is_some() && out.last_draw != entry.1 {
                entry.1 = out.last_draw;
                entry.2 += 1;
            }
        }
        let span = (now - start).as_secs_f64();
        if span >= 5.0 {
            for (id, _, frames) in &mut window.1 {
                println!("pw: {id} {:.1} frames/s", f64::from(*frames) / span);
                *frames = 0;
            }
            window.0 = Some(now);
        }
    });
}

#[cfg(feature = "gpu")]
fn start_gpu_loop(
    state: Rc<RefCell<control::DaemonState>>,
    gpu_rt: Rc<RefCell<gpu::GpuRuntime>>,
) {
    fn schedule(state: Rc<RefCell<control::DaemonState>>, gpu_rt: Rc<RefCell<gpu::GpuRuntime>>) {
        let (fps, paused) = {
            let st = state.borrow();
            (st.flags.fps_cap.load(std::sync::atomic::Ordering::Relaxed), st.flags.effective_paused())
        };
        // A configured cap drives drawing directly. With cap 0 frame
        // callbacks drive it and this timer only supervises at 4 Hz: it
        // starts the callback chain, picks up resume and repaints, and
        // recovers unanswered callbacks. While paused nothing is drawn, so
        // it also only polls at 4 Hz.
        let rate = if paused || fps == 0 { 4.0 } else { fps as f64 };
        let interval = std::time::Duration::from_secs_f64(1.0 / rate);
        glib::timeout_add_local_once(interval, move || {
            run_tick(&state, &gpu_rt);
            if gpu_rt.borrow().outputs.is_empty() {
                gpu_rt.borrow_mut().loop_running = false;
            } else {
                schedule(state, gpu_rt);
            }
        });
    }
    schedule(state, gpu_rt);
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
        flags.fps_cap.store(cfg.fps_cap.min(240), std::sync::atomic::Ordering::Relaxed);
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
        let appearance_ui =
            Arc::new(std::sync::Mutex::new(cfg.colors.clone()));

        let state = Rc::new(RefCell::new(control::DaemonState {
            webviews: Vec::new(),
            flags: flags.clone(),
            wallpaper: initial_wp.id.clone(),
            colors: cfg.colors.clone(),
            profiles: cfg.profiles.clone(),
            ascii: cfg.ascii.clone(),
            current: Some(current_id.clone()),
            profiles_ui: Some(profiles_ui.clone()),
            appearance_ui: Some(appearance_ui.clone()),
            tray: None,
            windows: Vec::new(),
            switch: None,
            on_colors: None,
            output_count: None,
            open_settings: None,
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
            brightness: cfg.colors.brightness.unwrap_or(1.5) as f32,
            speed: 1.0,
            palette: omarchy_palette::read().map(|p| p.to_ascii()),
            ..Default::default()
        }));

        // Renderer switch hook used by the Apply command (main thread).
        {
            let st = Rc::downgrade(&state);
            let rt = gpu_rt.clone();
            state.borrow_mut().switch = Some(Rc::new(move |wallpaper_id: &str| {
                let Some(st) = st.upgrade() else {
                    return "{\"error\":\"daemon state unavailable\"}\n".into();
                };
                switch_renderer(&st, &rt, wallpaper_id)
            }));
        }

        // Appearance hook: colors/size/brightness also reach the GPU runtime.
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
            let rt = gpu_rt.clone();
            let output_count = layer::monitors().len();
            state.borrow_mut().output_count = Some(Rc::new(move || {
                let rt = rt.borrow();
                if rt.outputs.is_empty() { output_count } else { rt.outputs.len() }
            }));
        }

        // Command bus shared by the CLI socket and the tray menu.
        let (tx, rx): (CmdTx, _) = async_channel::unbounded();

        // Settings window hook (GTK main loop). Values are passed as
        // arguments: the applier runs it under an active state borrow.
        {
            let tx_settings = tx.clone();
            state.borrow_mut().open_settings = Some(Rc::new(move |snapshot| {
                crate::settings::open(snapshot, tx_settings.clone());
            }));
        }

        // Power monitor (logind/UPower/Hyprland): pushes the deterministic
        // policy into the shared flags and syncs the web renderer on change.
        #[cfg(feature = "power")]
        {
            let (ptx, prx) = async_channel::unbounded::<()>();
            crate::power::PowerMonitor::start(flags.system_paused.clone(), ptx);
            let st = state.clone();
            glib::spawn_future_local(async move {
                while prx.recv().await.is_ok() {
                    let st = st.borrow();
                    st.eval_js(&format!("window.__pwPaused = {}", st.flags.effective_paused()));
                }
            });
        }

        // System tray in the top bar (volume/bluetooth area).
        let tray_result = ParticleWallTray {
            flags: flags.clone(),
            tx: tx.clone(),
            current: current_id.clone(),
            profiles: profiles_ui.clone(),
            appearance: appearance_ui.clone(),
        }
        .spawn();
        match tray_result {
            Ok(handle) => state.borrow_mut().tray = Some(handle),
            Err(e) => {
                eprintln!("pw: tray unavailable, retrying: {e}");
                let retry_state = state.clone();
                let retry_tx = tx.clone();
                glib::timeout_add_seconds_local(2, move || {
                    let result = ParticleWallTray {
                        flags: flags.clone(),
                        tx: retry_tx.clone(),
                        current: current_id.clone(),
                        profiles: profiles_ui.clone(),
                        appearance: appearance_ui.clone(),
                    }
                    .spawn();
                    match result {
                        Ok(handle) => {
                            retry_state.borrow_mut().tray = Some(handle);
                            println!("pw: tray registered after retry");
                            glib::ControlFlow::Break
                        }
                        Err(_) => glib::ControlFlow::Continue,
                    }
                });
            }
        }

        // Web host windows for every output; children come from
        // switch_renderer. They are mapped only when a web wallpaper needs
        // them: realizing a GTK window starts GTK's own Vulkan device (about
        // 70 MiB of VRAM plus driver threads), which GPU wallpapers never use.
        for monitor in layer::monitors() {
            let id = layer::monitor_identity(&monitor);
            let geometry = monitor.geometry();
            println!("pw: output {id} {}x{}", geometry.width(), geometry.height());

            let window = ApplicationWindow::builder().application(&app).build();
            layer::setup_layer_window(&window);
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
        // Check the small palette file for atomic theme replacements. No reload,
        // no retained WebView references, and no work for unchanged colors.
        {
            let st = Rc::downgrade(&state);
            #[cfg(feature = "gpu")]
            let rt = Rc::downgrade(&gpu_rt);
            let mut previous = omarchy_palette::read();
            glib::timeout_add_seconds_local(2, move || {
                let Some(st) = st.upgrade() else { return glib::ControlFlow::Break };
                let next = omarchy_palette::read();
                // A theme switch can briefly remove the file; keep the last palette.
                if next.is_some() && next != previous {
                    st.borrow().eval_js(&omarchy_palette::script(next.as_ref()));
                    // Native ASCII wallpapers repaint on the next tick, paused or not.
                    #[cfg(feature = "gpu")]
                    if let Some(rt) = rt.upgrade() {
                        rt.borrow_mut().palette = next.as_ref().map(|p| p.to_ascii());
                    }
                    previous = next;
                }
                glib::ControlFlow::Continue
            });
        }
        if let Err(e) = control::start(state.clone(), app, tx, rx) {
            eprintln!("pw: control socket unavailable: {e}");
        }
    });

    let _ = app.run();
}

#[cfg(all(test, feature = "webkit"))]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn web_reapply_reuses_host_and_gpu_teardown_releases_it() {
        gtk4::init().unwrap();
        let app = Application::builder()
            .application_id("com.particlewall.memory-tests")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        // Unmapped test window: never replaces or covers the user's wallpaper.
        let window = ApplicationWindow::builder().application(&app).build();
        gtk4::prelude::WidgetExt::realize(&window);
        let windows = vec![("test-output".to_string(), window.clone())];
        let state = Rc::new(RefCell::new(control::DaemonState {
            webviews: Vec::new(),
            flags: PlaybackFlags::default(),
            wallpaper: "DefaultWallpaper".into(),
            colors: Default::default(),
            profiles: Vec::new(),
            ascii: Default::default(),
            current: None,
            profiles_ui: None,
            appearance_ui: None,
            tray: None,
            windows: windows.clone(),
            switch: None,
            on_colors: None,
            output_count: None,
            open_settings: None,
        }));
        state.borrow().flags.paused.store(true, std::sync::atomic::Ordering::Relaxed);
        spawn_web_child(&state, &window, "test-output", "DefaultWallpaper");
        let original = state.borrow().webviews[0].1.downgrade();
        for i in 0..30 {
            let wallpaper = if i % 2 == 0 { "DefaultWallpaper" } else { "PrimeSpiralWallpaper" };
            spawn_web_child(&state, &window, "test-output", wallpaper);
            assert_eq!(state.borrow().webviews.len(), 1);
            assert_eq!(state.borrow().webviews[0].1, original.upgrade().unwrap());
        }
        // The load callback must not own the daemon state.
        assert_eq!(Rc::strong_count(&state), 1);
        clear_web_children(&state, &windows);
        assert!(state.borrow().webviews.is_empty());
        assert!(window.child().is_none());
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while original.upgrade().is_some() && Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(original.upgrade().is_none(), "old WebView still retained");
        spawn_web_child(&state, &window, "test-output", "DefaultWallpaper");
        assert_eq!(state.borrow().webviews.len(), 1);
        clear_web_children(&state, &windows);
        window.destroy();
    }
}
