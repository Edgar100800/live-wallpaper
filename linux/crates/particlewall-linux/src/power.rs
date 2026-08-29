//! Power and session monitoring (FR-PWR-01..05).
//!
//! Mirrors PowerManager.swift policy inputs onto Linux:
//!   - logind PrepareForSleep / Lock / Unlock  -> system unavailable
//!   - Hyprland fullscreen (IPC)               -> power save (wallpaper hidden)
//!   - UPower DisplayDevice                    -> battery detection
//!
//! The resolved playback policy (particlewall-contracts::playback) is pushed
//! into the shared PlaybackFlags; changes are announced on a channel so the
//! GTK main loop can sync the web renderer.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct PowerState {
    screen_locked: bool,
    screens_asleep: bool,
    fullscreen: bool,
    on_battery: bool,
}

/// Shared power view for diagnostics (CLI status).
#[derive(Debug, Clone, Copy, Default)]
pub struct PowerSnapshot {
    pub screen_locked: bool,
    pub screens_asleep: bool,
    pub fullscreen: bool,
    pub on_battery: bool,
    pub deep_sleep: bool,
}

pub struct PowerMonitor {
    state: Arc<Mutex<PowerState>>,
    deep_sleep: Arc<std::sync::atomic::AtomicBool>,
}

impl PowerMonitor {
    /// Spawns the monitor threads. Failures degrade individually (e.g. no
    /// logind on a non-systemd box only disables lock/sleep detection).
    /// `system_paused` receives the resolved deep-sleep policy (FR-PWR-05).
    pub fn start(
        system_paused: Arc<AtomicBool>,
        on_change: async_channel::Sender<()>,
    ) -> Self {
        let state = Arc::new(Mutex::new(PowerState::default()));
        let deep_sleep = Arc::new(AtomicBool::new(false));

        let recompute = {
            let state = state.clone();
            let deep_sleep = deep_sleep.clone();
            let system_paused = system_paused.clone();
            let on_change = on_change.clone();
            move || {
                let deep = {
                    let s = state.lock().unwrap();
                    let inputs = particlewall_contracts::playback::PolicyInputs {
                        user_paused: false,
                        power_save: s.fullscreen,
                        system_unavailable: s.screen_locked || s.screens_asleep,
                        // Detection only for now: pausing on battery needs a
                        // user-facing setting (macOS default is off too).
                        battery_pause: false,
                    };
                    particlewall_contracts::playback::resolve(inputs).deep_sleep
                };
                let changed = deep_sleep.swap(deep, Ordering::Relaxed) != deep;
                system_paused.store(deep, Ordering::Relaxed);
                if changed {
                    println!(
                        "pw: power policy {}",
                        if deep { "deep sleep (system)" } else { "active" }
                    );
                    let _ = on_change.try_send(());
                }
            }
        };

        // FR-PWR-01/02: logind sleep + lock signals.
        {
            let state = state.clone();
            let recompute = recompute.clone();
            std::thread::Builder::new()
                .name("pw-logind".into())
                .spawn(move || {
                    if let Err(e) = run_logind(&state, recompute) {
                        eprintln!("pw: logind monitor unavailable: {e}");
                    }
                })
                .ok();
        }

        // FR-PWR-03: UPower battery polling (display device). Desktops have
        // no battery: give up after a few failures instead of logging
        // forever.
        {
            let state = state.clone();
            let recompute = recompute.clone();
            std::thread::Builder::new()
                .name("pw-upower".into())
                .spawn(move || {
                    for attempt in 0..3 {
                        match battery_state() {
                            Ok(state_value) => {
                                // 2 = discharging, 3 = empty (UPower spec).
                                let on_battery = state_value == 2 || state_value == 3;
                                if state.lock().unwrap().on_battery != on_battery {
                                    state.lock().unwrap().on_battery = on_battery;
                                    recompute();
                                }
                                loop {
                                    std::thread::sleep(Duration::from_secs(60));
                                    if let Ok(state_value) = battery_state() {
                                        let on_battery = state_value == 2 || state_value == 3;
                                        if state.lock().unwrap().on_battery != on_battery {
                                            state.lock().unwrap().on_battery = on_battery;
                                            recompute();
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "pw: UPower unavailable (battery detection off): {e}"
                                );
                                std::thread::sleep(Duration::from_secs(5 * (attempt + 1)));
                            }
                        }
                    }
                })
                .ok();
        }

        // FR-PWR-04: Hyprland fullscreen via the event socket.
        {
            let state = state.clone();
            let recompute = recompute.clone();
            std::thread::Builder::new()
                .name("pw-hyprland".into())
                .spawn(move || {
                    if let Err(e) = run_hyprland(&state, recompute) {
                        eprintln!("pw: Hyprland IPC unavailable: {e}");
                    }
                })
                .ok();
        }

        Self { state, deep_sleep }
    }

    /// Current power view for diagnostics.
    pub fn snapshot(&self) -> PowerSnapshot {
        let s = self.state.lock().unwrap();
        PowerSnapshot {
            screen_locked: s.screen_locked,
            screens_asleep: s.screens_asleep,
            fullscreen: s.fullscreen,
            on_battery: s.on_battery,
            deep_sleep: self.deep_sleep.load(Ordering::Relaxed),
        }
    }
}

/// Watches logind PrepareForSleep, Lock and Unlock signals forever. Each
/// signal gets its own connection + thread (proxies are connection-bound).
fn run_logind(
    state: &Arc<Mutex<PowerState>>,
    recompute: impl Fn() + Send + Sync + Clone + 'static,
) -> Result<(), String> {
    for (signal, setter) in [("PrepareForSleep", SignalKind::Sleep)] {
        let state = state.clone();
        let recompute = recompute.clone();
        std::thread::Builder::new()
            .name(format!("pw-logind-{signal}"))
            .spawn(move || {
                if let Err(e) = wait_signal(signal, setter, &state, &recompute) {
                    eprintln!("pw: logind {signal} monitor unavailable: {e}");
                }
            })
            .ok();
    }
    // Lock/Unlock live on the SESSION object, not the Manager.
    {
        let state = state.clone();
        let recompute = recompute.clone();
        std::thread::Builder::new()
            .name("pw-logind-lock".to_string())
            .spawn(move || {
                if let Err(e) = wait_session_lock(&state, &recompute) {
                    eprintln!("pw: logind lock monitor unavailable: {e}");
                }
            })
            .ok();
    }
    Ok(())
}

enum SignalKind {
    Sleep,
}

fn wait_signal(
    signal: &str,
    kind: SignalKind,
    state: &Arc<Mutex<PowerState>>,
    recompute: &(impl Fn() + Send + Sync + Clone + 'static),
) -> Result<(), String> {
    let conn = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
    let proxy = zbus::blocking::Proxy::new_owned(
        conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .map_err(|e| e.to_string())?;
    let iter = proxy.receive_signal(signal).map_err(|e| e.to_string())?;
    for msg in iter {
        match kind {
            SignalKind::Sleep => {
                let start = msg.body().deserialize::<bool>().unwrap_or(false);
                state.lock().unwrap().screens_asleep = start;
            }
        }
        recompute();
    }
    Ok(())
}

/// Subscribes Lock/Unlock on THIS process's logind session.
fn wait_session_lock(
    state: &Arc<Mutex<PowerState>>,
    recompute: &(impl Fn() + Send + Sync + Clone + 'static),
) -> Result<(), String> {
    let conn = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
    let manager = zbus::blocking::Proxy::new_owned(
        conn.clone(),
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .map_err(|e| e.to_string())?;
    // systemd --user services do not belong to any session, so resolve the
    // graphical (seat0) session instead of GetSessionByPID.
    let sessions: Vec<(
        String,
        u32,
        String,
        String,
        zbus::zvariant::OwnedObjectPath,
    )> = manager
        .call_method("ListSessions", &())
        .map_err(|e| e.to_string())?
        .body()
        .deserialize()
        .map_err(|e| e.to_string())?;
    let session = sessions
        .iter()
        .find(|(_, _, _, seat, _)| seat == "seat0")
        .ok_or("no graphical session on seat0")?;
    eprintln!("pw: watching logind session {}", session.0);
    let session_proxy = zbus::blocking::Proxy::new_owned(
        conn,
        "org.freedesktop.login1",
        session.4.clone(),
        "org.freedesktop.login1.Session",
    )
    .map_err(|e| e.to_string())?;
    let lock_iter = session_proxy.receive_signal("Lock").map_err(|e| e.to_string())?;
    let state_lock = state.clone();
    let recompute_lock = recompute.clone();
    std::thread::Builder::new()
        .name("pw-logind-unlock".to_string())
        .spawn(move || {
            if let Ok(iter) = session_proxy.receive_signal("Unlock") {
                for _ in iter {
                    state_lock.lock().unwrap().screen_locked = false;
                    recompute_lock();
                }
            }
        })
        .ok();
    for _ in lock_iter {
        state.lock().unwrap().screen_locked = true;
        recompute();
    }
    Ok(())
}

/// UPower DisplayDevice State property (0 unknown, 1 charging, 2 discharging,
/// 3 empty, 4 fully charged).
fn battery_state() -> Result<u32, String> {
    let conn = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
    let proxy = zbus::blocking::Proxy::new_owned(
        conn,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower/DisplayDevice",
        "org.freedesktop.UPower.Device",
    )
    .map_err(|e| e.to_string())?;
    proxy.get_property("State").map_err(|e| e.to_string())
}

/// Watches the Hyprland event socket and tracks whether the active window is
/// fullscreen; any (re)check-worthy event triggers an activewindow query.
fn run_hyprland(
    state: &Arc<Mutex<PowerState>>,
    recompute: impl Fn() + Send + Sync + 'static,
) -> Result<(), String> {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .map_err(|_| "HYPRLAND_INSTANCE_SIGNATURE not set".to_string())?;
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/0".into());
    let events_path = format!("{runtime}/hypr/{signature}/.socket2.sock");
    let cmd_path = format!("{runtime}/hypr/{signature}/.socket.sock");

    // Keep reconnecting; Hyprland restarts or socket recreation must not kill
    // the monitor.
    loop {
        let mut stream = UnixStream::connect(&events_path).map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        eprintln!("pw: Hyprland event socket connected");
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break, // compositor closed the socket
                Ok(n) => {
                    let events = String::from_utf8_lossy(&buf[..n]);
                    let relevant = events.lines().any(|line| {
                        line.starts_with("fullscreen>>")
                            || line.starts_with("openwindow>>")
                            || line.starts_with("closewindow>>")
                            || line.starts_with("activewindow>>")
                    });
                    if !relevant {
                        continue;
                    }
                    if let Some(fs) = query_fullscreen(&cmd_path) {
                        if state.lock().unwrap().fullscreen != fs {
                            state.lock().unwrap().fullscreen = fs;
                            recompute();
                        }
                    }
                }
                // Timeout (EAGAIN): no events for 30s, keep waiting.
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => {
                    eprintln!("pw: Hyprland events read error: {e}");
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Queries `j/activewindow` and returns whether it is fullscreen.
fn query_fullscreen(cmd_path: &str) -> Option<bool> {
    let mut cmd = UnixStream::connect(cmd_path).ok()?;
    // NOTE: Hyprland's command socket rejects requests with a trailing
    // newline ("unknown request"); send the bare request.
    cmd.write_all(b"j/activewindow").ok()?;
    let mut reply = String::new();
    cmd.read_to_string(&mut reply).ok()?;
    let value: serde_json::Value = serde_json::from_str(reply.trim()).ok()?;
    // fullscreen: 0 none, 1 real, 2 fake (maximized pseudo-fullscreen).
    Some(value.get("fullscreen")?.as_i64()? != 0)
}
