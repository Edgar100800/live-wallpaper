//! M0A: WebKitGTK wallpaper host. Compiled only with the `web` feature
//! (requires the system package webkitgtk-6.0).

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, gdk as gtk_gdk, glib};
use ksni::blocking::TrayMethods;
use std::cell::RefCell;
use std::sync::Arc;
use std::rc::Rc;
use webkit6::prelude::*;
use webkit6::{
    NavigationPolicyDecision, PolicyDecisionType, UserContentInjectedFrames,
    UserScript, UserScriptInjectionTime, WebView,
};

#[path = "layer.rs"]
mod layer;
#[path = "control.rs"]
pub(crate) mod control;
#[path = "tray.rs"]
pub(crate) mod tray;
#[path = "library.rs"]
pub(crate) mod library;

use control::{CmdTx, PlaybackFlags};
use tray::ParticleWallTray;

pub(crate) const DEFAULT_FPS_CAP: u32 = 30;

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
        }));

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

        // Security root for local-only navigation: the whole bundled
        // resources tree (per-wallpaper roots return with the importer).
        let resources_root = library::resources_dir();

        for monitor in layer::monitors() {
            let id = layer::monitor_identity(&monitor);
            let geometry = monitor.geometry();
            println!("pw: output {id} {}x{}", geometry.width(), geometry.height());

            let window = ApplicationWindow::builder().application(&app).build();
            layer::setup_layer_window(&window);

            let webview = WebView::builder().build();
            attach_scripts(
                &webview,
                flags.paused.load(std::sync::atomic::Ordering::Relaxed),
                flags.fps_cap.load(std::sync::atomic::Ordering::Relaxed),
            );
            webview.set_background_color(&gtk_gdk::RGBA::BLACK);

            window.set_child(Some(&webview));
            install_navigation_policy(&webview, &resources_root);

            // Re-apply the saved appearance once each document finishes
            // loading (the appearance bridge installs at document-end).
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

            webview.load_uri(&format!("file://{}", initial_wp.index.display()));

            state.borrow_mut().webviews.push((id.clone(), webview));

            window.present();
            println!("pw: presented on {id} — {}", initial_wp.name);
        }

        // CLI socket + applier loop.
        if let Err(e) = control::start(state.clone(), app, tx, rx) {
            eprintln!("pw: control socket unavailable: {e}");
        }
    });

    let _ = app.run();
}
