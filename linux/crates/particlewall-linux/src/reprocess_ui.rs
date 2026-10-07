//! Reprocessing belongs to the selected wallpaper; workers keep the existing variant intact.
use super::importer::{self, Cancel, Event};
use gtk4::{glib, prelude::*, Orientation};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub struct Panel {
    pub widget: gtk4::Box,
    pub update: Rc<dyn Fn(&str)>,
}

pub fn density_picker() -> gtk4::ComboBoxText {
    let picker = gtk4::ComboBoxText::new();
    for (id, text) in [
        ("4", "Muy alta · 4 px"),
        ("6", "Alta · 6 px"),
        ("8", "Media · 8 px"),
        ("12", "Baja · 12 px"),
        ("16", "Muy baja · 16 px"),
    ] {
        picker.append(Some(id), text);
    }
    picker.set_active_id(Some("8"));
    picker.set_tooltip_text(Some(
        "Celdas más pequeñas producen más caracteres y consumen más memoria.",
    ));
    picker
}

pub fn controls(completed: Rc<dyn Fn(String, String)>) -> Panel {
    let root = gtk4::Box::new(Orientation::Vertical, 12);
    let heading = gtk4::Label::new(Some("Densidad de caracteres"));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    root.append(&heading);
    root.append(&gtk4::Separator::new(Orientation::Horizontal));
    let description = gtk4::Label::new(Some("Reprocesa el video para cambiar el detalle del ASCII. Se crea una variante y se conserva el fondo anterior."));
    description.set_wrap(true);
    description.set_xalign(0.0);
    root.append(&description);
    let picker = density_picker();
    root.append(&picker);
    let info = gtk4::Label::new(None);
    info.set_wrap(true);
    info.set_xalign(0.0);
    root.append(&info);
    let action = gtk4::Button::with_label("Reprocesar video");
    action.add_css_class("suggested-action");
    root.append(&action);
    let progress = gtk4::ProgressBar::new();
    progress.set_visible(false);
    root.append(&progress);
    let result = gtk4::Label::new(None);
    result.set_wrap(true);
    result.set_xalign(0.0);
    root.append(&result);
    let cancel = gtk4::Button::with_label("Cancelar reprocesamiento");
    cancel.set_visible(false);
    root.append(&cancel);
    let selected = Rc::new(RefCell::new(String::new()));
    let busy = Rc::new(Cell::new(false));
    let token: Rc<RefCell<Cancel>> = Rc::new(RefCell::new(Arc::new(AtomicBool::new(false))));
    let refresh: Rc<dyn Fn()> = {
        let selected = selected.clone();
        let busy = busy.clone();
        let picker = picker.downgrade();
        let info = info.downgrade();
        let action = action.downgrade();
        Rc::new(move || {
            let (Some(picker), Some(info), Some(action)) =
                (picker.upgrade(), info.upgrade(), action.upgrade())
            else {
                return;
            };
            if busy.get() {
                return;
            }
            match importer::processing(&selected.borrow()) {
                Ok((video, mut options, cached)) => {
                    let current = options.cell_size;
                    options.cell_size =
                        picker.active_id().and_then(|v| v.parse().ok()).unwrap_or(8);
                    match importer::estimate(&video, &options) {
                        Ok(bytes) => {
                            info.set_text(&format!("{}\nActual: {} px · {}p · {} FPS\nASCII estimado: {:.1} MiB\n{}",video.title,current,options.height,options.fps,bytes as f64/1048576.0,
                                if cached { "Original guardado: se procesará sin descargar." } else { "Falta el original: se recuperará de YouTube una vez y se conservará en esta PC." }));
                            action.set_label(if cached {
                                "Reprocesar video"
                            } else {
                                "Recuperar original y reprocesar"
                            });
                            action.set_tooltip_text(Some(if options.cell_size == current {
                                "Elige otra densidad para habilitar el reprocesamiento."
                            } else {
                                "Se conservará el fondo anterior y se creará una variante."
                            }));
                            action.set_sensitive(
                                options.cell_size != current && bytes <= 512 * 1024 * 1024,
                            );
                            picker.set_sensitive(true);
                        }
                        Err(error) => {
                            info.set_text(&error);
                            action.set_sensitive(false);
                        }
                    }
                }
                Err(error) => {
                    info.set_text(&error);
                    action.set_sensitive(false);
                    picker.set_sensitive(false);
                }
            }
        })
    };
    let update: Rc<dyn Fn(&str)> = {
        let selected = selected.clone();
        let token = token.clone();
        let picker = picker.downgrade();
        let result = result.downgrade();
        let refresh = refresh.clone();
        Rc::new(move |id| {
            if *selected.borrow() != id {
                token.borrow().store(true, Ordering::Relaxed);
                *selected.borrow_mut() = id.into();
                if let Some(picker) = picker.upgrade() {
                    let size = importer::processing(id)
                        .map(|(_, options, _)| options.cell_size)
                        .unwrap_or(8);
                    picker.set_active_id(Some(&size.to_string()));
                }
                if let Some(result) = result.upgrade() {
                    result.set_text("");
                }
            }
            refresh();
        })
    };
    {
        let refresh = refresh.clone();
        picker.connect_changed(move |_| refresh());
    }
    {
        let token = token.clone();
        cancel.connect_clicked(move |w| {
            token.borrow().store(true, Ordering::Relaxed);
            w.set_sensitive(false);
        });
    }
    {
        let token = token.clone();
        root.connect_unrealize(move |_| {
            token.borrow().store(true, Ordering::Relaxed);
        });
    }
    {
        let selected = selected.clone();
        let busy = busy.clone();
        let token = token.clone();
        let refresh = refresh.clone();
        let root = root.downgrade();
        let picker = picker.downgrade();
        let progress = progress.downgrade();
        let result = result.downgrade();
        let cancel = cancel.downgrade();
        action.connect_clicked(move |action| {
            if busy.replace(true) {
                return;
            }
            let (Some(picker), Some(progress), Some(result), Some(cancel)) = (
                picker.upgrade(),
                progress.upgrade(),
                result.upgrade(),
                cancel.upgrade(),
            ) else {
                busy.set(false);
                return;
            };
            let old = selected.borrow().clone();
            let size = picker.active_id().and_then(|v| v.parse().ok()).unwrap_or(8);
            let cancellation = Arc::new(AtomicBool::new(false));
            *token.borrow_mut() = cancellation.clone();
            action.set_sensitive(false);
            picker.set_sensitive(false);
            cancel.set_visible(true);
            cancel.set_sensitive(true);
            progress.set_visible(true);
            progress.set_fraction(0.0);
            result.set_text("Preparando reprocesamiento…");
            let (tx, rx) = async_channel::unbounded();
            let original = old.clone();
            std::thread::spawn(move || {
                let converted = importer::reprocess(&original, size, &cancellation, &|event| {
                    let _ = tx.send_blocking(event);
                });
                let _ =
                    tx.send_blocking(converted.map(Event::Imported).unwrap_or_else(Event::Error));
            });
            let selected = selected.clone();
            let busy = busy.clone();
            let refresh = refresh.clone();
            let completed = completed.clone();
            let root = root.clone();
            let progress = progress.downgrade();
            let result = result.downgrade();
            let cancel = cancel.downgrade();
            glib::spawn_future_local(async move {
                while let Ok(event) = rx.recv().await {
                    if root.upgrade().is_none() {
                        break;
                    }
                    let (Some(progress), Some(result)) = (progress.upgrade(), result.upgrade())
                    else {
                        break;
                    };
                    match event {
                        Event::Progress(text, fraction) => {
                            result.set_text(&text);
                            progress.set_fraction(fraction);
                            continue;
                        }
                        Event::Imported(id) => {
                            busy.set(false);
                            progress.set_fraction(1.0);
                            if *selected.borrow() == old {
                                completed(old.clone(), id);
                            }
                            result.set_text(
                                "Variante lista. Guarda los cambios para conservararla como fondo.",
                            );
                        }
                        Event::Error(error) => {
                            busy.set(false);
                            result.set_text(&error);
                        }
                        Event::Analyzed(_) => {
                            continue;
                        }
                    }
                    if let Some(cancel) = cancel.upgrade() {
                        cancel.set_visible(false);
                    }
                    refresh();
                    break;
                }
            });
        });
    }
    Panel {
        widget: root,
        update,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    fn widgets(root: &gtk4::Widget) -> Vec<gtk4::Widget> {
        let mut all = vec![root.clone()];
        let mut child = root.first_child();
        while let Some(w) = child {
            all.extend(widgets(&w));
            child = w.next_sibling();
        }
        all
    }
    pub fn selected_video_density_and_completion() {
        let completed = Rc::new(RefCell::new(None));
        let result = completed.clone();
        let panel = controls(Rc::new(move |old, new| {
            *result.borrow_mut() = Some((old, new));
        }));
        let window = gtk4::Window::new();
        window.set_child(Some(&panel.widget));
        window.present();
        let all = widgets(panel.widget.upcast_ref());
        let picker = all
            .iter()
            .find_map(|w| w.clone().downcast::<gtk4::ComboBoxText>().ok())
            .unwrap();
        let action = all
            .iter()
            .filter_map(|w| w.clone().downcast::<gtk4::Button>().ok())
            .find(|w| w.label().as_deref() == Some("Reprocesar video"))
            .unwrap();
        (panel.update)("DefaultWallpaper");
        assert!(!action.is_sensitive());
        assert!(!picker.is_sensitive());
        if let Ok(id) = std::env::var("PARTICLEWALL_REPROCESS_GUI_TEST_ID") {
            (panel.update)(&id);
            assert!(picker.is_sensitive());
            assert_eq!(picker.active_id().as_deref(), Some("8"));
            assert!(!action.is_sensitive());
            picker.set_active_id(Some("4"));
            assert!(action.is_sensitive());
            action.emit_clicked();
            assert!(!action.is_sensitive());
            assert!(!picker.is_sensitive());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while completed.borrow().is_none() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let (old, new) = completed
                .borrow()
                .clone()
                .expect("Existing variant must complete asynchronously");
            assert_eq!(old, id);
            assert_eq!(new, format!("{id}-c4"));
            assert!(picker.is_sensitive());
        }
        window.close();
    }
}
