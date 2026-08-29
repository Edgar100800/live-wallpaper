//! System tray (StatusNotifierItem) for the top bar, mirroring the macOS
//! NSStatusItem quick menu: pause/resume, FPS cap and quit.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::control::{CmdTx, Command, PlaybackFlags};
use ksni::menu::{MenuItem, StandardItem, SubMenu};
use ksni::{Tray, ToolTip};

pub struct ParticleWallTray {
    pub flags: PlaybackFlags,
    pub tx: CmdTx,
    /// Active wallpaper id, shared with DaemonState.
    pub current: Arc<std::sync::Mutex<String>>,
    /// Saved profile names, shared with DaemonState.
    pub profiles: Arc<std::sync::Mutex<Vec<String>>>,
}

impl ParticleWallTray {
    fn send(&self, cmd: Command) {
        // Unbounded channel: never blocks.
        let _ = self.tx.send_blocking((None, cmd));
    }

    fn is_paused(&self) -> bool {
        self.flags.effective_paused()
    }
}

impl Tray for ParticleWallTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "particlewall".into()
    }

    fn title(&self) -> String {
        "ParticleWall".into()
    }

    fn icon_name(&self) -> String {
        if self.is_paused() {
            "media-playback-pause".into()
        } else {
            "applications-graphics".into()
        }
    }

    fn tool_tip(&self) -> ToolTip {
        let fps = self.flags.fps_cap.load(Ordering::Relaxed);
        ToolTip {
            icon_name: self.icon_name(),
            icon_pixmap: vec![],
            title: "ParticleWall".into(),
            description: if self.is_paused() {
                "Pausado — click para el menu".into()
            } else if fps == 0 {
                "Activo (sin limite de FPS)".into()
            } else {
                format!("Activo ({fps} FPS)")
            },
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let paused = self.is_paused();
        let fps = self.flags.fps_cap.load(Ordering::Relaxed);
        let current = self.current.lock().unwrap().clone();

        // Wallpaper picker (bundled HTML fallbacks of the eight GPU models).
        let mut wallpapers: Vec<MenuItem<Self>> = crate::web::library::bundled()
            .into_iter()
            .map(|wp| {
                let check = if wp.id == current { "[x] " } else { "[ ] " };
                MenuItem::Standard(StandardItem {
                    label: format!("{check}{}", wp.name),
                    activate: Box::new(move |t: &mut Self| {
                        t.send(Command::Apply(wp.id.clone()));
                    }),
                    ..Default::default()
                })
            })
            .collect();
        wallpapers.push(MenuItem::Separator);

        // Saved color profiles.
        let profiles = self.profiles.lock().unwrap().clone();
        let mut profile_items: Vec<MenuItem<Self>> = if profiles.is_empty() {
            vec![MenuItem::Standard(StandardItem {
                label: "(sin perfiles — crea con --profile-save)".into(),
                enabled: false,
                ..Default::default()
            })]
        } else {
            profiles
                .iter()
                .map(|name| {
                    let n = name.clone();
                    MenuItem::Standard(StandardItem {
                        label: format!("  {name}"),
                        activate: Box::new(move |t: &mut Self| {
                            t.send(Command::ApplyProfile(n.clone()));
                        }),
                        ..Default::default()
                    })
                })
                .collect()
        };
        profile_items.push(MenuItem::Separator);
        profile_items.push(MenuItem::Standard(StandardItem {
            label: "Abrir ajustes (--set-color)".into(),
            enabled: false,
            ..Default::default()
        }));
        wallpapers.push(MenuItem::SubMenu(SubMenu {
            label: "Perfiles de color".into(),
            submenu: profile_items,
            ..Default::default()
        }));
        wallpapers.push(MenuItem::Separator);

        let mut items = vec![MenuItem::SubMenu(SubMenu {
            label: "Fondos".into(),
            submenu: wallpapers,
            ..Default::default()
        })];
        items.push(MenuItem::Separator);
        items.push(MenuItem::Separator);
        items.push(MenuItem::Standard(StandardItem {
            label: if paused { "Reanudar".into() } else { "Pausar".into() },
            icon_name: if paused {
                "media-playback-start".into()
            } else {
                "media-playback-pause".into()
            },
            activate: Box::new(|t: &mut Self| t.send(Command::TogglePause)),
            ..Default::default()
        }));
        items.push(MenuItem::SubMenu(SubMenu {
            label: "Limite de FPS".into(),
            submenu: [
                (0u32, "Sin limite"),
                (15, "15"),
                (30, "30"),
                (60, "60"),
            ]
            .into_iter()
            .map(|(value, label)| {
                let check = if value == fps { "  [x] " } else { "  [ ] " };
                MenuItem::Standard(StandardItem {
                    label: format!("{check}{label}"),
                    activate: Box::new(move |t: &mut Self| {
                        t.send(Command::Fps(value));
                    }),
                    ..Default::default()
                })
            })
            .collect(),
            ..Default::default()
        }));
        items.push(MenuItem::Separator);
        items.push(MenuItem::Standard(StandardItem {
            label: "Salir".into(),
            icon_name: "application-exit".into(),
            activate: Box::new(|t: &mut Self| t.send(Command::Quit)),
            ..Default::default()
        }));
        items
    }
}
