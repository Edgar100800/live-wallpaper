//! Unified native settings editor. Preview commands never write configuration.
use crate::web::control::{CmdTx, Command};
use crate::web::library::{self, ASCIIColorMode, Config, Profile};
use gtk4::prelude::*;
use gtk4::{Align, Button, CheckButton, ColorDialog, ColorDialogButton, ComboBoxText, Label, Orientation, Scale, Window};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone)]
pub struct Snapshot {
    pub config: Config,
    pub paused: bool,
    pub system_paused: bool,
    pub outputs: usize,
}

thread_local! {
    static WINDOW: RefCell<gtk4::glib::WeakRef<Window>> = RefCell::new(gtk4::glib::WeakRef::new());
    static EDITOR: RefCell<Option<Rc<Editor>>> = const { RefCell::new(None) };
}

struct Editor {
    draft: RefCell<Config>,
    baseline: RefCell<Config>,
    updating: Cell<bool>,
    pending_save: RefCell<Option<Config>>,
    tx: CmdTx,
    hint: Label,
    save: Button,
}
impl Editor {
    fn preview(&self) {
        if self.updating.get() { return; }
        let dirty = *self.draft.borrow() != *self.baseline.borrow();
        self.save.set_sensitive(dirty);
        self.hint.set_text(if dirty { "Vista previa · Cambios sin guardar" } else { "Sin cambios pendientes" });
        let _ = self.tx.send_blocking((None, Command::Configure(self.draft.borrow().clone(), false)));
    }
    fn id(&self) -> String { self.draft.borrow().wallpaper.clone().unwrap_or_else(|| "DefaultWallpaper".into()) }
}

pub fn configuration_result(reply: &str, saved: bool) {
    EDITOR.with(|slot| {
        if let Some(editor) = slot.borrow().as_ref() {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(reply) {
                if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
                    editor.pending_save.borrow_mut().take();
                    editor.save.set_sensitive(*editor.draft.borrow() != *editor.baseline.borrow());
                    editor.hint.set_text(error);
                    return;
                }
            }
            if saved {
                if let Some(saved) = editor.pending_save.borrow_mut().take() { *editor.baseline.borrow_mut() = saved; }
                editor.save.set_sensitive(*editor.draft.borrow() != *editor.baseline.borrow());
                editor.hint.set_text("Cambios guardados");
            }
        }
    });
}

fn label(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.set_halign(Align::Start);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label
}
fn section(parent: &gtk4::Box, title: &str) -> gtk4::Box {
    let group = gtk4::Box::new(Orientation::Vertical, 10);
    let heading = label(title);
    heading.add_css_class("heading");
    group.append(&heading);
    group.append(&gtk4::Separator::new(Orientation::Horizontal));
    parent.append(&group);
    group
}
fn row(parent: &gtk4::Box, title: &str, widget: &impl IsA<gtk4::Widget>) {
    let row = gtk4::Box::new(Orientation::Horizontal, 16);
    let title = label(title);
    title.set_hexpand(true);
    row.append(&title);
    row.append(widget);
    parent.append(&row);
}
fn slider(parent: &gtk4::Box, title: &str, min: f64, max: f64, step: f64) -> Scale {
    let value = label("");
    value.set_width_chars(5);
    row(parent, title, &value);
    let scale = Scale::with_range(Orientation::Horizontal, min, max, step);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.connect_value_changed(move |scale| value.set_text(&format!("{:.2}", scale.value())));
    parent.append(&scale);
    scale
}
fn packed(color: &gtk4::gdk::RGBA) -> u32 {
    ((color.red() * 255.0).round() as u32) << 16 |
    ((color.green() * 255.0).round() as u32) << 8 |
    (color.blue() * 255.0).round() as u32
}
fn rgba(value: u32) -> gtk4::gdk::RGBA {
    gtk4::gdk::RGBA::new(((value >> 16) & 255) as f32 / 255.0,
        ((value >> 8) & 255) as f32 / 255.0, (value & 255) as f32 / 255.0, 1.0)
}

pub fn open(snapshot: Snapshot, tx: CmdTx) {
    if let Some(window) = WINDOW.with(|slot| slot.borrow().upgrade()) {
        window.present();
        return;
    }
    let window = Window::builder().title("ParticleWall — Configuración")
        .default_width(900).default_height(760).resizable(true).build();
    WINDOW.with(|slot| slot.borrow_mut().set(Some(&window)));
    let root = gtk4::Box::new(Orientation::Vertical, 18);
    root.set_margin_top(24); root.set_margin_bottom(24);
    root.set_margin_start(24); root.set_margin_end(24);
    window.set_child(Some(&root));
    let title = label("Tu fondo, a tu manera");
    title.add_css_class("title-1");
    root.append(&title);
    root.append(&label("Ajusta el escritorio en vivo y guarda cuando estés conforme."));
    let notebook = gtk4::Notebook::new(); notebook.set_vexpand(true); notebook.set_hexpand(true);
    root.append(&notebook);
    let page = |title: &str, horizontal: bool| {
        let scroll = gtk4::ScrolledWindow::builder().vexpand(true).hexpand(true).hscrollbar_policy(gtk4::PolicyType::Never).build();
        let body = gtk4::Box::new(if horizontal { Orientation::Horizontal } else { Orientation::Vertical }, 24);
        body.set_margin_top(20); body.set_margin_bottom(12);
        if horizontal { body.set_homogeneous(true); }
        scroll.set_child(Some(&body)); notebook.append_page(&scroll, Some(&Label::new(Some(title)))); body
    };
    let fondo_page = page("Fondo", false);
    let ascii_page = page("Video ASCII", true);
    let import_slot = page("Importar video", false);
    let particles_page = page("Partículas", true);
    let ascii_left = gtk4::Box::new(Orientation::Vertical, 24); let reprocess_slot = gtk4::Box::new(Orientation::Vertical, 24);
    ascii_page.append(&ascii_left); ascii_page.append(&reprocess_slot);
    let particles_left = gtk4::Box::new(Orientation::Vertical, 24); let particles_right = gtk4::Box::new(Orientation::Vertical, 24);
    particles_page.append(&particles_left); particles_page.append(&particles_right);
    let reprocess_update: Rc<RefCell<Option<Rc<dyn Fn(&str)>>>> = Rc::new(RefCell::new(None));

    let playback = section(&fondo_page, "Fondo y reproducción");
    let wallpaper = ComboBoxText::new();
    for wp in library::all() { wallpaper.append(Some(&wp.id), &wp.name); }
    for cell in wallpaper.cells() {
        cell.set_property("ellipsize", gtk4::pango::EllipsizeMode::End);
        cell.set_property("max-width-chars", 28i32);
    }
    wallpaper.set_hexpand(true);
    playback.append(&wallpaper);
    let status = label("");
    status.add_css_class("dim-label");
    playback.append(&status);
    let pause = CheckButton::with_label("Pausar animación");
    pause.set_active(snapshot.paused);
    playback.append(&pause);
    if snapshot.system_paused { playback.append(&label("Pausa automática del sistema activa.")); }
    let fps = ComboBoxText::new();
    for (id, title) in [("15", "15 FPS · Ahorro"), ("30", "30 FPS · Equilibrado"),
        ("60", "60 FPS · Fluido"), ("0", "Monitor · sincronizado")] { fps.append(Some(id), title); }
    if ![0, 15, 30, 60].contains(&snapshot.config.fps_cap) {
        fps.append(Some(&snapshot.config.fps_cap.to_string()), &format!("{} FPS", snapshot.config.fps_cap));
    }
    row(&playback, "Frecuencia", &fps);

    let ascii_group = section(&ascii_left, "Color y separación del ASCII");
    let mode = ComboBoxText::new();
    mode.append(Some("original"), "Originales del video");
    mode.append(Some("omarchy"), "Paleta de Omarchy");
    row(&ascii_group, "Modo de color", &mode);
    let swatches = label("");
    swatches.add_css_class("dim-label");
    ascii_group.append(&swatches);
    let clean = CheckButton::with_label("Limpiar fondo lejano");
    ascii_group.append(&clean);
    let separation = slider(&ascii_group, "Separación del primer plano", 0.0, 1.0, 0.01);
    let note = label("Más separación elimina más zonas luminosas. Es una aproximación: puede ocultar detalles brillantes del primer plano.");
    note.add_css_class("dim-label");
    ascii_group.append(&note);

    let appearance = section(&particles_left, "Apariencia de partículas");
    let background = ColorDialogButton::new(Some(ColorDialog::builder().title("Color de fondo").with_alpha(false).build()));
    let particle = ColorDialogButton::new(Some(ColorDialog::builder().title("Color de puntos").with_alpha(false).build()));
    row(&appearance, "Fondo", &background);
    row(&appearance, "Puntos", &particle);
    let size = slider(&appearance, "Tamaño de puntos", 0.25, 8.0, 0.05);
    let brightness = slider(&appearance, "Intensidad", 0.25, 10.0, 0.05);
    appearance.append(&label("Estos controles se habilitan para fondos de partículas."));

    let profiles = section(&particles_right, "Perfiles de color");
    let profile = ComboBoxText::new();
    profiles.append(&profile);
    let actions = gtk4::Box::new(Orientation::Horizontal, 8);
    let apply_profile = Button::with_label("Usar perfil");
    let delete_profile = Button::with_label("Eliminar");
    actions.append(&apply_profile); actions.append(&delete_profile);
    profiles.append(&actions);
    let name = gtk4::Entry::builder().placeholder_text("Nombre del nuevo perfil").build();
    name.set_tooltip_text(Some("Guardar los colores actuales como perfil"));
    profiles.append(&name);
    let add_profile = Button::with_label("Crear con estos colores");
    profiles.append(&add_profile);

    let footer = gtk4::Box::new(Orientation::Horizontal, 10);
    let hint = label("Sin cambios pendientes");
    hint.set_hexpand(true);
    let reset = Button::with_label("Restaurar valores");
    let cancel = Button::with_label("Cancelar");
    let save = Button::with_label("Guardar cambios");
    save.add_css_class("suggested-action");
    save.set_sensitive(false);
    footer.append(&hint); footer.append(&reset); footer.append(&cancel); footer.append(&save);
    root.append(&gtk4::Separator::new(Orientation::Horizontal));
    root.append(&footer);
    let editor = Rc::new(Editor { draft: RefCell::new(snapshot.config.clone()), baseline: RefCell::new(snapshot.config),
        updating: Cell::new(false), pending_save: RefCell::new(None), tx, hint, save: save.clone() });
    EDITOR.with(|slot| *slot.borrow_mut() = Some(editor.clone()));

    let refresh: Rc<dyn Fn()> = {
        let editor = Rc::downgrade(&editor);
        let reprocess_update = reprocess_update.clone();
        let wallpaper = wallpaper.downgrade(); let fps = fps.downgrade(); let mode = mode.downgrade();
        let clean = clean.downgrade(); let separation = separation.downgrade(); let profile = profile.downgrade();
        let background = background.downgrade(); let particle = particle.downgrade();
        let size = size.downgrade(); let brightness = brightness.downgrade(); let appearance = appearance.downgrade();
        let ascii_group = ascii_group.downgrade(); let profiles = profiles.downgrade();
        let apply_profile = apply_profile.downgrade(); let delete_profile = delete_profile.downgrade();
        Rc::new(move || {
            let Some(editor) = editor.upgrade() else { return };
            let Some(wallpaper) = wallpaper.upgrade() else { return };
            let Some(fps) = fps.upgrade() else { return };
            let Some(mode) = mode.upgrade() else { return };
            let Some(clean) = clean.upgrade() else { return };
            let Some(separation) = separation.upgrade() else { return };
            let Some(profile) = profile.upgrade() else { return };
            let Some(background) = background.upgrade() else { return };
            let Some(particle) = particle.upgrade() else { return };
            let Some(size) = size.upgrade() else { return };
            let Some(brightness) = brightness.upgrade() else { return };
            let Some(appearance) = appearance.upgrade() else { return };
            let Some(ascii_group) = ascii_group.upgrade() else { return };
            let Some(profiles) = profiles.upgrade() else { return };
            let Some(apply_profile) = apply_profile.upgrade() else { return };
            let Some(delete_profile) = delete_profile.upgrade() else { return };

            editor.updating.set(true);
            let cfg = editor.draft.borrow();
            let id = cfg.wallpaper.as_deref().unwrap_or("DefaultWallpaper");
            let ascii = library::find(id).map(|wp| wp.index.with_file_name("clip.asciivideo").is_file() || wp.index.with_file_name("stream.json").is_file()).unwrap_or(false);
            wallpaper.set_active_id(Some(id));
            wallpaper.set_tooltip_text(library::find(id).as_ref().map(|wp| wp.name.as_str())); fps.set_active_id(Some(&cfg.fps_cap.to_string()));
            let settings = cfg.ascii.get(id).cloned().unwrap_or_default();
            mode.set_active_id(Some(if settings.color_mode == ASCIIColorMode::Original { "original" } else { "omarchy" }));
            clean.set_active(settings.clean_background); separation.set_value(settings.separation);
            separation.set_sensitive(settings.clean_background);
            ascii_group.set_sensitive(ascii); appearance.set_sensitive(!ascii); profiles.set_sensitive(!ascii);
            background.set_rgba(&rgba(cfg.colors.background.unwrap_or(198153)));
            particle.set_rgba(&rgba(cfg.colors.particle.unwrap_or(15269887)));
            size.set_value(cfg.colors.size.unwrap_or(1.6)); brightness.set_value(cfg.colors.brightness.unwrap_or(1.5));
            profile.remove_all();
            for p in &cfg.profiles { profile.append(Some(&p.name), &p.name); }
            profile.set_active(if cfg.profiles.is_empty() { None } else { Some(0) });
            apply_profile.set_sensitive(!cfg.profiles.is_empty()); delete_profile.set_sensitive(!cfg.profiles.is_empty());
            status.set_text(&format!("{} · {} pantalla(s)", if ascii { "Video ASCII" } else { "Partículas" }, snapshot.outputs));
            swatches.set_markup(&crate::web::omarchy_palette::description());
            editor.updating.set(false);
            if let Some(update) = reprocess_update.borrow().as_ref() { update(id); }
        })
    };
    refresh();
    let select_variant: Rc<dyn Fn(Option<String>, String)> = {
        let e = Rc::downgrade(&editor); let picker = wallpaper.downgrade(); let refresh = refresh.clone();
        Rc::new(move |old, id| {
            let (Some(e), Some(picker)) = (e.upgrade(), picker.upgrade()) else { return; };
            e.updating.set(true); picker.remove_all();
            for wp in library::all() { picker.append(Some(&wp.id), &wp.name); }
            { let mut draft = e.draft.borrow_mut();
              let appearance = old.as_ref().and_then(|old| draft.ascii.get(old)).cloned()
                  .unwrap_or(library::ASCIISettings { clean_background: false, ..Default::default() });
              draft.wallpaper = Some(id.clone()); draft.ascii.entry(id).or_insert(appearance); }
            e.updating.set(false); refresh(); e.preview();
        })
    };
    {
        let select = select_variant.clone();
        import_slot.append(&crate::web::import_ui::controls(Rc::new(move |id| select(None, id))));
        let panel = crate::web::reprocess_ui::controls(Rc::new(move |old,id| select_variant(Some(old),id)));
        (panel.update)(&editor.id()); *reprocess_update.borrow_mut() = Some(panel.update);
        reprocess_slot.append(&panel.widget);
    }
    {
        let e = Rc::downgrade(&editor); let refresh = refresh.clone();
        wallpaper.connect_changed(move |widget| {
            let Some(e) = e.upgrade() else { return };
            if e.updating.get() { return; }
            if let Some(id) = widget.active_id() { e.draft.borrow_mut().wallpaper = Some(id.into()); }
            refresh(); e.preview();
        });
    }
    {
        let e = Rc::downgrade(&editor);
        fps.connect_changed(move |widget| {
            let Some(e) = e.upgrade() else { return };
            if e.updating.get() { return; }
            if let Some(id) = widget.active_id() { e.draft.borrow_mut().fps_cap = id.parse().unwrap_or(30); }
            e.preview();
        });
    }
    {
        let e = Rc::downgrade(&editor);
        mode.connect_changed(move |widget| {
            let Some(e) = e.upgrade() else { return };
            if e.updating.get() { return; }
            let id = e.id();
            e.draft.borrow_mut().ascii.entry(id).or_default().color_mode =
                if widget.active_id().as_deref() == Some("original") { ASCIIColorMode::Original } else { ASCIIColorMode::Omarchy };
            e.preview();
        });
    }
    {
        let e = Rc::downgrade(&editor); let separation = separation.clone();
        clean.connect_toggled(move |widget| {
            let Some(e) = e.upgrade() else { return };
            if e.updating.get() { return; }
            let id = e.id();
            e.draft.borrow_mut().ascii.entry(id).or_default().clean_background = widget.is_active();
            separation.set_sensitive(widget.is_active()); e.preview();
        });
    }
    {
        let e = Rc::downgrade(&editor);
        separation.connect_value_changed(move |widget| {
            let Some(e) = e.upgrade() else { return };
            if e.updating.get() { return; }
            let id = e.id(); e.draft.borrow_mut().ascii.entry(id).or_default().separation = widget.value(); e.preview();
        });
    }
    for (widget, is_background) in [(background, true), (particle, false)] {
        let e = Rc::downgrade(&editor);
        widget.connect_rgba_notify(move |widget| {
            let Some(e) = e.upgrade() else { return };
            if e.updating.get() { return; }
            { let mut cfg = e.draft.borrow_mut();
                if is_background { cfg.colors.background = Some(packed(&widget.rgba())); }
                else { cfg.colors.particle = Some(packed(&widget.rgba())); } }
            e.preview();
        });
    }
    for (widget, is_size) in [(size, true), (brightness, false)] {
        let e = Rc::downgrade(&editor);
        widget.connect_value_changed(move |widget| {
            let Some(e) = e.upgrade() else { return };
            if e.updating.get() { return; }
            { let mut cfg = e.draft.borrow_mut();
                if is_size { cfg.colors.size = Some(widget.value()); } else { cfg.colors.brightness = Some(widget.value()); } }
            e.preview();
        });
    }
    {
        let e = Rc::downgrade(&editor); let profile = profile.clone(); let refresh = refresh.clone();
        apply_profile.connect_clicked(move |_| {
            let Some(e) = e.upgrade() else { return };
            let selected = profile.active_id();
            let found = e.draft.borrow().profiles.iter().find(|p| Some(p.name.as_str()) == selected.as_deref()).cloned();
            if let Some(p) = found { let mut cfg = e.draft.borrow_mut(); cfg.colors.background = Some(p.background); cfg.colors.particle = Some(p.particle); }
            refresh(); e.preview();
        });
    }
    {
        let e = Rc::downgrade(&editor); let profile = profile.clone(); let refresh = refresh.clone();
        delete_profile.connect_clicked(move |_| {
            let Some(e) = e.upgrade() else { return };
            let selected = profile.active_id();
            e.draft.borrow_mut().profiles.retain(|p| Some(p.name.as_str()) != selected.as_deref());
            refresh(); e.preview();
        });
    }
    {
        let e = Rc::downgrade(&editor); let refresh = refresh.clone();
        add_profile.connect_clicked(move |_| {
            let Some(e) = e.upgrade() else { return };
            let title = name.text().trim().to_string();
            if title.is_empty() { e.hint.set_text("Escribe un nombre para el perfil."); name.grab_focus(); return; }
            { let mut cfg = e.draft.borrow_mut();
                let p = Profile { name: title.clone(), background: cfg.colors.background.unwrap_or(198153), particle: cfg.colors.particle.unwrap_or(15269887) };
                cfg.profiles.retain(|p| p.name != title); cfg.profiles.insert(0, p); }
            name.set_text(""); refresh(); e.preview();
        });
    }
    {
        let e = Rc::downgrade(&editor); let refresh = refresh.clone();
        reset.connect_clicked(move |_| {
            let Some(e) = e.upgrade() else { return };
            { let mut cfg = e.draft.borrow_mut();
                cfg.colors = library::ColorSettings { background: Some(198153), particle: Some(15269887), size: Some(1.6), brightness: Some(1.5) };
                cfg.fps_cap = 30;
                if let Some(id) = cfg.wallpaper.clone() { cfg.ascii.remove(&id); } }
            refresh(); e.preview();
        });
    }
    {
        let e = Rc::downgrade(&editor);
        save.connect_clicked(move |_| {
            let Some(e) = e.upgrade() else { return };
            let cfg = e.draft.borrow().clone();
            *e.pending_save.borrow_mut() = Some(cfg.clone());
            e.save.set_sensitive(false);
            e.hint.set_text("Guardando cambios…");
            let _ = e.tx.send_blocking((None, Command::Configure(cfg, true)));
        });
    }
    {
        let tx = editor.tx.clone();
        pause.connect_toggled(move |widget| { let _ = tx.send_blocking((None, if widget.is_active() { Command::Pause } else { Command::Resume })); });
    }
    let weak = window.downgrade();
    cancel.connect_clicked(move |_| { if let Some(window) = weak.upgrade() { window.close(); } });
    window.connect_close_request(move |_| {
        if editor.pending_save.borrow().is_some() { return gtk4::glib::Propagation::Stop; }
        if *editor.draft.borrow() != *editor.baseline.borrow() {
            let _ = editor.tx.send_blocking((None, Command::Configure(editor.baseline.borrow().clone(), false)));
        }
        WINDOW.with(|slot| slot.borrow_mut().set(None));
        EDITOR.with(|slot| *slot.borrow_mut() = None);
        gtk4::glib::Propagation::Proceed
    });
    window.present();
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn widgets(root: &gtk4::Widget) -> Vec<gtk4::Widget> {
        let mut result = vec![root.clone()];
        let mut child = root.first_child();
        while let Some(widget) = child {
            result.extend(widgets(&widget));
            child = widget.next_sibling();
        }
        result
    }
    pub fn reprocess_preserves_appearance_and_cancel_restores_selection() {
        let Ok(id) = std::env::var("PARTICLEWALL_REPROCESS_GUI_TEST_ID") else { return; };
        let (tx,rx) = async_channel::unbounded();
        let mut config = Config::default(); config.wallpaper = Some(id.clone());
        let appearance = library::ASCIISettings { color_mode: ASCIIColorMode::Original, clean_background: true, separation: 0.7 };
        config.ascii.insert(id.clone(), appearance.clone());
        open(Snapshot { config: config.clone(), paused: false, system_paused: false, outputs: 1 }, tx);
        let window = WINDOW.with(|slot| slot.borrow().upgrade().unwrap());
        let all = widgets(window.upcast_ref());
        let notebook = all.iter().find_map(|w| w.clone().downcast::<gtk4::Notebook>().ok()).unwrap();
        notebook.set_current_page(Some(1));
        let action = all.iter().filter_map(|w| w.clone().downcast::<Button>().ok()).find(|w| w.label().as_deref() == Some("Reprocesar video")).unwrap();
        let panel = action.parent().unwrap();
        let density = widgets(&panel).into_iter().find_map(|w| w.downcast::<ComboBoxText>().ok()).unwrap();
        density.set_active_id(Some("4")); action.emit_clicked();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let preview = loop {
            if let Ok((_,Command::Configure(config,false))) = rx.try_recv() { break config; }
            assert!(std::time::Instant::now() < deadline, "Reprocessing did not preview the selected variant");
            while gtk4::glib::MainContext::default().pending() { gtk4::glib::MainContext::default().iteration(false); }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let target = format!("{id}-c4"); assert_eq!(preview.wallpaper.as_deref(),Some(target.as_str()));
        assert_eq!(preview.ascii[&target],appearance);
        window.close();
        let (_,Command::Configure(restored,false)) = rx.try_recv().unwrap() else {panic!("Expected previous variant on cancel")};
        assert_eq!(restored,config);
    }
    pub fn settings_preview_save_cancel_and_single_window() {
        let (tx, rx) = async_channel::unbounded();
        let mut cfg = Config::default();
        cfg.wallpaper = Some("SpiderManASCIIWallpaper".into());
        let snapshot = Snapshot { config: cfg, paused: false, system_paused: false, outputs: 1 };
        open(snapshot.clone(), tx.clone());
        let window = WINDOW.with(|slot| slot.borrow().upgrade().unwrap());
        assert!(window.is_resizable());
        assert_eq!(window.default_width(), 900);
        let notebook = widgets(window.upcast_ref()).into_iter().find_map(|w| w.downcast::<gtk4::Notebook>().ok()).unwrap();
        assert_eq!(notebook.n_pages(), 4);
        for (index,title) in ["Fondo","Video ASCII","Importar video","Partículas"].iter().enumerate() {
            assert_eq!(notebook.tab_label_text(&notebook.nth_page(Some(index as u32)).unwrap()).as_deref(),Some(*title));
        }
        open(snapshot, tx);
        assert_eq!(WINDOW.with(|slot| slot.borrow().upgrade().unwrap()), window);
        let all = widgets(window.upcast_ref());
        let mode = all.iter().filter_map(|w| w.clone().downcast::<ComboBoxText>().ok())
            .find(|w| w.active_id().as_deref() == Some("omarchy")).unwrap();
        mode.set_active_id(Some("original"));
        let (_, Command::Configure(original, false)) = rx.try_recv().unwrap() else { panic!("expected preview") };
        assert_eq!(original.ascii["SpiderManASCIIWallpaper"].color_mode, ASCIIColorMode::Original);
        let clean = all.iter().filter_map(|w| w.clone().downcast::<CheckButton>().ok())
            .find(|w| w.label().as_deref() == Some("Limpiar fondo lejano")).unwrap();
        clean.set_active(false);
        let (_, Command::Configure(saved, false)) = rx.try_recv().unwrap() else { panic!("expected preview") };
        assert!(!saved.ascii["SpiderManASCIIWallpaper"].clean_background);
        assert_eq!(saved.ascii["SpiderManASCIIWallpaper"].color_mode, ASCIIColorMode::Original);
        let save = all.iter().filter_map(|w| w.clone().downcast::<Button>().ok())
            .find(|w| w.label().as_deref() == Some("Guardar cambios")).unwrap();
        save.emit_clicked();
        let (_, Command::Configure(persisted, true)) = rx.try_recv().unwrap() else { panic!("expected explicit save") };
        assert_eq!(persisted, saved);
        window.close();
        assert!(WINDOW.with(|slot| slot.borrow().upgrade().is_some()), "pending save must finish before close");
        assert!(rx.try_recv().is_err());
        configuration_result(r#"{"error":"No se pudo guardar"}"#, true);
        assert!(save.is_sensitive(), "failed save must allow retry");
        save.emit_clicked();
        let (_, Command::Configure(retried, true)) = rx.try_recv().unwrap() else { panic!("expected retry") };
        assert_eq!(retried, saved);
        configuration_result("ok", true);
        assert!(!save.is_sensitive());
        clean.set_active(true);
        let _ = rx.try_recv().unwrap();
        let wallpaper = all.iter().filter_map(|w| w.clone().downcast::<ComboBoxText>().ok())
            .find(|w| w.active_id().as_deref() == Some("SpiderManASCIIWallpaper")).unwrap();
        wallpaper.set_active_id(Some("DefaultWallpaper"));
        let (_, Command::Configure(particles, false)) = rx.try_recv().unwrap() else { panic!("expected wallpaper preview") };
        assert_eq!(particles.wallpaper.as_deref(), Some("DefaultWallpaper"));
        let name = all.iter().filter_map(|w| w.clone().downcast::<gtk4::Entry>().ok()).find(|w| w.placeholder_text().as_deref() == Some("Nombre del nuevo perfil")).unwrap();
        name.set_text("Test profile");
        let add = all.iter().filter_map(|w| w.clone().downcast::<Button>().ok())
            .find(|w| w.label().as_deref() == Some("Crear con estos colores")).unwrap();
        assert!(add.is_sensitive());
        add.emit_clicked();
        let (_, Command::Configure(profile, false)) = rx.try_recv().unwrap() else { panic!("expected profile preview") };
        assert_eq!(profile.profiles[0].name, "Test profile");
        let delete = all.iter().filter_map(|w| w.clone().downcast::<Button>().ok())
            .find(|w| w.label().as_deref() == Some("Eliminar")).unwrap();
        delete.emit_clicked();
        let (_, Command::Configure(deleted, false)) = rx.try_recv().unwrap() else { panic!("expected profile deletion preview") };
        assert!(deleted.profiles.is_empty());
        window.close();
        let (_, Command::Configure(restored, false)) = rx.try_recv().unwrap() else { panic!("expected cancel restoration") };
        assert_eq!(restored, saved);
        assert!(WINDOW.with(|slot| slot.borrow().upgrade().is_none()));
        assert!(EDITOR.with(|slot| slot.borrow().is_none()));
    }
}
