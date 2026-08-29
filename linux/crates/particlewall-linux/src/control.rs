//! Control plane shared by the Unix socket CLI and the system tray.
//! Commands are applied on the GTK main thread; internal callers (tray
//! menu items) send without a reply stream.

use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

#[cfg(feature = "web")]
use crate::web::tray::ParticleWallTray;
use crate::web::{library, DEFAULT_FPS_CAP};
use webkit6::prelude::*;
use webkit6::WebView;

pub fn socket_path() -> std::path::PathBuf {
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::path::Path::new(&runtime).join("particlewall.sock")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Pause,
    Resume,
    TogglePause,
    Fps(u32),
    Apply(String),
    /// Merge partial color settings into the current appearance.
    SetColors(library::ColorSettings),
    SaveProfile(String),
    DeleteProfile(String),
    ApplyProfile(String),
    /// Open (or raise) the GTK settings window.
    OpenSettings,
    Status,
    Quit,
}

impl Command {
    fn parse(line: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        match v.get("cmd")?.as_str()? {
            "pause" => Some(Self::Pause),
            "resume" => Some(Self::Resume),
            "toggle-pause" | "toggle" => Some(Self::TogglePause),
            "fps" => Some(Self::Fps(v.get("value")?.as_u64()? as u32)),
            "apply" => Some(Self::Apply(v.get("value")?.as_str()?.into())),
            "set-colors" => Some(Self::SetColors(
                serde_json::from_value(v.get("colors").cloned()?).ok()?,
            )),
            "profile-save" => Some(Self::SaveProfile(v.get("name")?.as_str()?.into())),
            "profile-delete" => Some(Self::DeleteProfile(v.get("name")?.as_str()?.into())),
            "profile-apply" => Some(Self::ApplyProfile(v.get("name")?.as_str()?.into())),
            "open-settings" => Some(Self::OpenSettings),
            "status" => Some(Self::Status),
            "quit" => Some(Self::Quit),
            _ => None,
        }
    }
}

/// Mirrors playback state for threads outside the GTK main thread
/// (the tray service reads these when refreshing its icon).
/// `paused` is the USER pause (CLI/tray); `system_paused` is set by the
/// power monitor (lock/sleep/fullscreen, FR-PWR-05). Playback is active
/// only when both are clear.
#[derive(Clone)]
pub struct PlaybackFlags {
    pub paused: Arc<AtomicBool>,
    pub system_paused: Arc<AtomicBool>,
    pub fps_cap: Arc<AtomicU32>,
}

impl Default for PlaybackFlags {
    fn default() -> Self {
        Self {
            paused: Arc::new(AtomicBool::new(false)),
            system_paused: Arc::new(AtomicBool::new(false)),
            fps_cap: Arc::new(AtomicU32::new(DEFAULT_FPS_CAP)),
        }
    }
}

impl PlaybackFlags {
    /// Effective playback pause: user OR system policy.
    pub fn effective_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed) || self.system_paused.load(Ordering::Relaxed)
    }
}

pub struct DaemonState {
    pub webviews: Vec<(String, WebView)>,
    pub flags: PlaybackFlags,
    pub wallpaper: String,
    /// Current appearance (persisted in config.json).
    pub colors: library::ColorSettings,
    /// Saved color profiles (persisted in config.json).
    pub profiles: Vec<library::Profile>,
    /// Shared with the tray so its menu can mark the active wallpaper.
    pub current: Option<Arc<std::sync::Mutex<String>>>,
    /// Shared with the tray: saved profile names for its submenu.
    pub profiles_ui: Option<Arc<std::sync::Mutex<Vec<String>>>>,
    /// Shared with the tray + settings window: the live appearance.
    pub appearance_ui: Option<Arc<std::sync::Mutex<library::ColorSettings>>>,
    #[allow(clippy::type_complexity)]
    pub tray: Option<ksni::blocking::Handle<ParticleWallTray>>,
    /// Layer windows per output; renderers attach children to these.
    pub windows: Vec<(String, gtk4::ApplicationWindow)>,
    /// Renderer switch hook (web <-> GPU), set by the host at startup.
    #[cfg(feature = "gpu")]
    pub switch: Option<std::rc::Rc<dyn Fn(&str) -> String>>,
    /// Appearance hook for non-web renderers (GPU), set by the host.
    #[cfg(feature = "gpu")]
    pub on_colors: Option<std::rc::Rc<dyn Fn(&library::ColorSettings)>>,
    /// Live output-count hook (GPU-aware), set by the host.
    #[cfg(feature = "gpu")]
    pub output_count: Option<std::rc::Rc<dyn Fn() -> usize>>,
    /// Opens (or raises) the settings window with the given initial values;
    /// set by the host on the GTK main loop. Values are passed as arguments
    /// because the applier invokes it under an active state borrow.
    pub open_settings: Option<std::rc::Rc<dyn Fn(f64, f64)>>,
}

/// Wire ranges mirroring the macOS control descriptors: "Tamaño" 0.5-4 and
/// "Intensidad de puntos" 0.25-10.
pub const PARTICLE_SIZE_RANGE: (f64, f64) = (0.5, 4.0);
pub const BRIGHTNESS_RANGE: (f64, f64) = (0.25, 10.0);

fn appearance_js_parts(colors: &library::ColorSettings) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(b) = colors.background {
        parts.push(format!("backgroundColor:{b}"));
    }
    if let Some(p) = colors.particle {
        parts.push(format!("particleColor:{p}"));
    }
    if let Some(s) = colors.size {
        let s = s.clamp(PARTICLE_SIZE_RANGE.0, PARTICLE_SIZE_RANGE.1);
        parts.push(format!("particleSize:{s}"));
    }
    if let Some(w) = colors.brightness {
        let w = w.clamp(BRIGHTNESS_RANGE.0, BRIGHTNESS_RANGE.1);
        parts.push(format!("brightness:{w}"));
    }
    parts
}

impl DaemonState {
    /// JS snippet applying the current appearance through the shared
    /// wallpaper contract. None when nothing is set.
    pub fn colors_js(&self) -> Option<String> {
        let parts = appearance_js_parts(&self.colors);
        if parts.is_empty() {
            return None;
        }
        Some(format!(
            "window.__pwApplySettings && window.__pwApplySettings({{{}}});",
            parts.join(",")
        ))
    }

    fn push_colors(&mut self) {
        self.sync_appearance_ui();
        #[cfg(feature = "gpu")]
        if let Some(hook) = &self.on_colors {
            hook(&self.colors);
        }
        let parts = appearance_js_parts(&self.colors);
        if parts.is_empty() {
            return;
        }
        let js = format!(
            "window.__pwApplySettings && window.__pwApplySettings({{{}}});",
            parts.join(",")
        );
        for (_, wv) in &self.webviews {
            wv.evaluate_javascript(&js, None, None, None::<&gtk4::gio::Cancellable>, |_| {});
        }
    }

    /// Mirrors the current appearance into the shared view used by the tray
    /// menu and the settings window.
    fn sync_appearance_ui(&self) {
        if let Some(ui) = &self.appearance_ui {
            *ui.lock().unwrap() = self.colors.clone();
        }
    }

    fn status_reply(&self, outputs: usize) -> String {
        let body = serde_json::json!({
            "paused": self.flags.effective_paused(),
            "systemPaused": self.flags.system_paused.load(Ordering::Relaxed),
            "fpsCap": self.flags.fps_cap.load(Ordering::Relaxed),
            "outputs": outputs,
            "wallpaper": self.wallpaper,
            "colors": self.colors,
            "profiles": self.profiles,
        });
        format!("{body}\n")
    }

    fn persist(&self) {
        library::save_config(&library::Config {
            wallpaper: Some(self.wallpaper.clone()),
            colors: self.colors.clone(),
            profiles: self.profiles.clone(),
        });
    }

    fn set_paused(&mut self, paused: bool) -> String {
        self.flags.paused.store(paused, Ordering::Relaxed);
        let js = format!("window.__pwPaused = {paused}");
        for (_, wv) in &self.webviews {
            // Fire-and-forget; errors are non-fatal (e.g. mid-load).
            wv.evaluate_javascript(&js, None, None, None::<&gtk4::gio::Cancellable>, |_| {});
        }
        if paused {
            "ok\n".into()
        } else {
            "ok\n".into()
        }
    }

    fn set_fps(&mut self, fps: u32) {
        self.flags.fps_cap.store(fps, Ordering::Relaxed);
        let js = format!("window.__pwFPSCap = {fps}");
        for (_, wv) in &self.webviews {
            wv.evaluate_javascript(&js, None, None, None::<&gtk4::gio::Cancellable>, |_| {});
        }
    }

    /// Applies a command; returns the text reply for socket clients.
    fn apply(&mut self, cmd: &Command) -> String {
        match cmd {
            Command::Pause => self.set_paused(true),
            Command::Resume => self.set_paused(false),
            Command::TogglePause => {
                let p = !self.flags.paused.load(Ordering::Relaxed);
                self.set_paused(p)
            }
            Command::Fps(fps) => {
                self.set_fps(*fps);
                "ok\n".into()
            }
            // Routed to apply_wallpaper by the applier loop: the renderer
            // switch hook re-borrows the shared state, so it can never run
            // under this borrow_mut.
            Command::Apply(_) => "{\"error\":\"internal: apply misrouted\"}\n".into(),
            Command::SetColors(new_colors) => {
                let before = self.colors.clone();
                if new_colors.background.is_some() {
                    self.colors.background = new_colors.background;
                }
                if new_colors.particle.is_some() {
                    self.colors.particle = new_colors.particle;
                }
                if new_colors.size.is_some() {
                    self.colors.size = new_colors.size;
                }
                if new_colors.brightness.is_some() {
                    self.colors.brightness = new_colors.brightness;
                }
                // Slider dragging floods this command: skip no-op updates so
                // config.json is not rewritten for every tick.
                if self.colors != before {
                    self.push_colors();
                    self.persist();
                }
                format!("{{\"colors\":{}}}\n", serde_json::to_string(&self.colors).unwrap())
            }
            Command::SaveProfile(name) => {
                let (Some(bg), Some(particle)) = (self.colors.background, self.colors.particle) else {
                    return "{\"error\":\"set both background and particle colors first\"}\n".into();
                };
                let profile = library::Profile { name: name.clone(), background: bg, particle };
                self.profiles.retain(|p| p.name != *name);
                self.profiles.insert(0, profile);
                {
                    let mut list = self.profiles_ui.as_ref().expect("profiles_ui set").lock().unwrap();
                    list.retain(|n| *n != *name);
                    list.insert(0, name.clone());
                }
                self.persist();
                format!("{{\"saved\":\"{name}\"}}\n")
            }
            Command::DeleteProfile(name) => {
                let before = self.profiles.len();
                self.profiles.retain(|p| p.name != *name);
                if let Some(ui) = &self.profiles_ui { ui.lock().unwrap().retain(|n| *n != *name); }
                self.persist();
                format!("{{\"deleted\":{}}}\n", before != self.profiles.len())
            }
            Command::ApplyProfile(name) => {
                match self.profiles.iter().find(|p| p.name == *name).cloned() {
                    Some(profile) => {
                        self.colors.background = Some(profile.background);
                        self.colors.particle = Some(profile.particle);
                        self.push_colors();
                        format!("{{\"profile-applied\":\"{name}\"}}\n")
                    }
                    None => format!("{{\"error\":\"unknown profile '{name}'\"}}\n"),
                }
            }
            Command::OpenSettings => match &self.open_settings {
                Some(open) => {
                    let size = self
                        .colors
                        .size
                        .unwrap_or(1.6)
                        .clamp(PARTICLE_SIZE_RANGE.0, PARTICLE_SIZE_RANGE.1);
                    let brightness = self
                        .colors
                        .brightness
                        .unwrap_or(1.5)
                        .clamp(BRIGHTNESS_RANGE.0, BRIGHTNESS_RANGE.1);
                    open(size, brightness);
                    "ok\n".into()
                }
                None => "{\"error\":\"settings window unavailable\"}\n".into(),
            },
            Command::Status => {
                #[cfg(feature = "gpu")]
                let outputs = self
                    .output_count
                    .as_ref()
                    .map(|f| f())
                    .unwrap_or(self.webviews.len());
                #[cfg(not(feature = "gpu"))]
                let outputs = self.webviews.len();
                self.status_reply(outputs)
            }
            Command::Quit => String::new(),
        }
    }
}

pub type SharedState = Rc<RefCell<DaemonState>>;
pub type CmdTx = async_channel::Sender<(Option<UnixStream>, Command)>;

/// Web-only fallback when no renderer switch hook is installed (gpu feature
/// disabled): load the wallpaper into the existing web children.
fn web_fallback(state: &SharedState, wp: &library::Wallpaper) -> String {
    let uri = format!("file://{}", wp.index.display());
    let st = state.borrow_mut();
    for (_, wv) in &st.webviews {
        wv.load_uri(&uri);
    }
    format!("{{\"applied\":\"{}\"}}\n", wp.name)
}

/// Switches the active wallpaper. Runs on the GTK main thread with NO active
/// borrow of `state`: the renderer switch hook borrows it again while
/// rebuilding the window children (spawn_web_child).
fn apply_wallpaper(state: &SharedState, id: &str) -> String {
    let Some(wp) = library::find(id) else {
        return format!("{{\"error\":\"unknown wallpaper '{id}'\"}}\n");
    };

    #[cfg(feature = "gpu")]
    let reply = {
        // Clone the hook out; the temporary borrow ends with this statement.
        let hook = state.borrow().switch.clone();
        match hook {
            Some(sw) => sw(&wp.id),
            None => web_fallback(state, &wp),
        }
    };
    #[cfg(not(feature = "gpu"))]
    let reply = web_fallback(state, &wp);

    let mut st = state.borrow_mut();
    st.wallpaper = wp.id.clone();
    if let Some(current) = &st.current {
        *current.lock().unwrap() = wp.id.clone();
    }
    st.persist();
    if let Some(handle) = &st.tray {
        handle.update(|_| {});
    }
    reply
}

/// Starts the applier task on the GTK main thread plus the socket listener
/// thread. The caller keeps a clone of `tx` for internal command sources
/// (e.g. the system tray menu).
pub fn start(
    state: SharedState,
    app: gtk4::Application,
    tx: CmdTx,
    rx: async_channel::Receiver<(Option<UnixStream>, Command)>,
) -> std::io::Result<()> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    println!("pw: control socket at {}", path.display());

    // Tray menu items push internal commands through this sender.
    glib::spawn_future_local(async move {
        while let Ok((stream, cmd)) = rx.recv().await {
            if cmd == Command::Quit {
                let _ = std::fs::remove_file(socket_path());
                app.quit();
                break;
            }
            // Apply swaps renderers through the switch hook, which re-borrows
            // the shared state internally; it must run with no active borrow.
            let reply = match &cmd {
                Command::Apply(id) => apply_wallpaper(&state, id),
                _ => {
                    let mut st = state.borrow_mut();
                    let reply = st.apply(&cmd);
                    // Refresh tray icon/title after any playback change.
                    if let Some(handle) = &st.tray {
                        handle.update(|_| {});
                    }
                    reply
                }
            };
            if let Some(mut stream) = stream {
                let _ = stream.write_all(reply.as_bytes());
            }
        }
    });

    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(match stream.try_clone() {
                Ok(s) => s,
                Err(_) => continue,
            });
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line.is_empty() {
                continue;
            }
            let Some(cmd) = Command::parse(&line) else {
                let _ =
                    writeln!(stream.try_clone().unwrap(), "{{\"error\":\"bad command\"}}");
                continue;
            };
            if tx.send_blocking((Some(stream), cmd)).is_err() {
                break; // applier gone (daemon quitting)
            }
        }
        drop(tx);
    });

    Ok(())
}
