//! Native import controls; workers never touch GTK.
use super::importer::{self, Cancel, Event, Options, Video};
use gtk4::{glib, prelude::*, Orientation};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub fn controls(imported: Rc<dyn Fn(String)>) -> gtk4::Box {
    let root = gtk4::Box::new(Orientation::Vertical, 10);
    let heading = gtk4::Label::new(Some("Agregar video de YouTube"));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    root.append(&heading);
    let url = gtk4::Entry::builder()
        .placeholder_text("https://www.youtube.com/watch?v=…")
        .hexpand(true)
        .build();
    url.set_tooltip_text(Some("URL de un video de YouTube o youtu.be"));
    root.append(&url);
    let quality = gtk4::ComboBoxText::new();
    for (id, text) in [
        ("480", "480p · Ligero"),
        ("720", "720p · Equilibrado"),
        ("1080", "1080p · Detallado"),
    ] {
        quality.append(Some(id), text);
    }
    quality.set_active_id(Some("720"));
    root.append(&quality);
    let rate = gtk4::ComboBoxText::new();
    rate.append(Some("15"), "15 FPS · Ahorro");
    rate.append(Some("30"), "30 FPS · Fluido");
    rate.set_active_id(Some("30"));
    root.append(&rate);
    let density_label = gtk4::Label::new(Some("Densidad de caracteres"));
    density_label.set_xalign(0.0);
    root.append(&density_label);
    let density = super::reprocess_ui::density_picker();
    root.append(&density);
    let kept = gtk4::Label::new(Some("El original se guarda en esta PC para cambiar la densidad después, sin volver a descargar."));
    kept.set_wrap(true);
    kept.set_xalign(0.0);
    root.append(&kept);
    let interval = gtk4::Box::new(Orientation::Horizontal, 8);
    let start = gtk4::SpinButton::with_range(0.0, 86400.0, 1.0);
    let end = gtk4::SpinButton::with_range(0.0, 86400.0, 1.0);
    for (title, widget) in [("Desde (s)", &start), ("Hasta (s)", &end)] {
        let label = gtk4::Label::new(Some(title));
        interval.append(&label);
        widget.set_hexpand(true);
        interval.append(widget);
    }
    root.append(&interval);
    let info = gtk4::Label::new(Some(
        "Analiza la URL para ver su duración. Hasta 5 minutos por fondo; sin audio.",
    ));
    info.set_wrap(true);
    info.set_xalign(0.0);
    root.append(&info);
    let progress = gtk4::ProgressBar::new();
    progress.set_visible(false);
    root.append(&progress);
    let actions = gtk4::Box::new(Orientation::Horizontal, 8);
    let analyze = gtk4::Button::with_label("Analizar URL");
    let convert = gtk4::Button::with_label("Convertir a ASCII");
    convert.set_sensitive(false);
    convert.add_css_class("suggested-action");
    let cancel = gtk4::Button::with_label("Cancelar importación");
    cancel.set_visible(false);
    actions.append(&analyze);
    actions.append(&convert);
    root.append(&actions);
    root.append(&cancel);
    let video: Rc<RefCell<Option<Video>>> = Rc::new(RefCell::new(None));
    let busy = Rc::new(Cell::new(false));
    let token: Rc<RefCell<Cancel>> = Rc::new(RefCell::new(Arc::new(AtomicBool::new(false))));
    {
        let close_token = token.clone();
        let token = token.clone();
        cancel.connect_clicked(move |button| {
            token.borrow().store(true, Ordering::Relaxed);
            button.set_sensitive(false);
        });
        root.connect_unrealize(move |_| {
            close_token.borrow().store(true, Ordering::Relaxed);
        });
    }
    let options: Rc<dyn Fn() -> Options> = {
        let quality = quality.downgrade();
        let rate = rate.downgrade();
        let density = density.downgrade();
        let start = start.downgrade();
        let end = end.downgrade();
        Rc::new(move || Options {
            height: quality
                .upgrade()
                .and_then(|w| w.active_id())
                .and_then(|v| v.parse().ok())
                .unwrap_or(720),
            fps: rate
                .upgrade()
                .and_then(|w| w.active_id())
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            start: start.upgrade().map(|w| w.value()).unwrap_or(0.0),
            end: end.upgrade().map(|w| w.value()),
            cell_size: density
                .upgrade()
                .and_then(|w| w.active_id())
                .and_then(|v| v.parse().ok())
                .unwrap_or(8),
        })
    };
    let update: Rc<dyn Fn()> = {
        let video = video.clone();
        let options = options.clone();
        let info = info.downgrade();
        let convert = convert.downgrade();
        let busy = busy.clone();
        Rc::new(move || {
            if busy.get() {
                return;
            }
            if let (Some(video), Some(info), Some(convert)) =
                (video.borrow().as_ref(), info.upgrade(), convert.upgrade())
            {
                match importer::estimate(video, &options()) {
                    Ok(bytes) => {
                        info.set_text(&format!(
                            "{}\nDuración: {:.0} s · ASCII estimado: {:.1} MiB",
                            video.title,
                            video.duration,
                            bytes as f64 / 1048576.0
                        ));
                        convert.set_sensitive(bytes <= 512 * 1024 * 1024);
                    }
                    Err(error) => {
                        info.set_text(&error);
                        convert.set_sensitive(false);
                    }
                }
            }
        })
    };
    for spin in [&start, &end] {
        let update = update.clone();
        spin.connect_value_changed(move |_| update());
    }
    for combo in [&quality, &rate, &density] {
        let update = update.clone();
        combo.connect_changed(move |_| update());
    }
    {
        let video = video.clone();
        let convert = convert.downgrade();
        let info = info.downgrade();
        url.connect_changed(move |_| {
            video.borrow_mut().take();
            if let Some(w) = convert.upgrade() {
                w.set_sensitive(false);
            }
            if let Some(w) = info.upgrade() {
                w.set_text("Analiza la nueva URL antes de convertir.");
            }
        });
    }
    for (button, is_import) in [(&analyze, false), (&convert, true)] {
        let busy = busy.clone();
        let token = token.clone();
        let video = video.clone();
        let options = options.clone();
        let imported = imported.clone();
        let update = update.clone();
        let root = root.downgrade();
        let url = url.downgrade();
        let analyze = analyze.downgrade();
        let convert = convert.downgrade();
        let cancel = cancel.downgrade();
        let info = info.downgrade();
        let progress = progress.downgrade();
        let start = start.downgrade();
        let end = end.downgrade();
        let quality = quality.downgrade();
        let rate = rate.downgrade();
        let density = density.downgrade();
        button.connect_clicked(move |_| {
            if busy.replace(true) { return; }
            let (Some(urlw),Some(analyze),Some(convert),Some(cancel),Some(info),Some(progress))=(url.upgrade(),analyze.upgrade(),convert.upgrade(),cancel.upgrade(),info.upgrade(),progress.upgrade()) else {busy.set(false); return;};
            let urltext=urlw.text().to_string(); let selected=video.borrow().clone(); let opts=options();
            let cancellation=Arc::new(AtomicBool::new(false)); *token.borrow_mut()=cancellation.clone();
            analyze.set_sensitive(false);convert.set_sensitive(false);cancel.set_visible(true);cancel.set_sensitive(true);urlw.set_sensitive(false);progress.set_visible(true);progress.set_fraction(0.0);info.set_text("Consultando YouTube…");
            for w in [start.upgrade(),end.upgrade()] .into_iter().flatten(){w.set_sensitive(false);}
            for w in [quality.upgrade(),rate.upgrade(),density.upgrade()].into_iter().flatten(){w.set_sensitive(false);}
            let (tx,rx)=async_channel::unbounded();
            std::thread::spawn(move || {
                let result=if is_import { selected.ok_or_else(||"Analiza primero la URL".to_string()).and_then(|v| importer::import(&urltext,&v,&opts,&cancellation,&|event| {let _=tx.send_blocking(event);})).map(Event::Imported) }
                    else { importer::analyze(&urltext,&cancellation).map(Event::Analyzed) };
                let _=tx.send_blocking(result.unwrap_or_else(Event::Error));
            });
            let root=root.clone(); let video=video.clone(); let busy=busy.clone(); let update=update.clone(); let imported=imported.clone(); let start=start.clone(); let end=end.clone(); let quality=quality.clone(); let rate=rate.clone(); let density=density.clone();
            // Keep only weak references across awaits so closing settings releases widgets.
            let url=urlw.downgrade();let analyze=analyze.downgrade();let convert=convert.downgrade();let cancel=cancel.downgrade();let info=info.downgrade();let progress=progress.downgrade();
            glib::spawn_future_local(async move {
                while let Ok(event)=rx.recv().await {
                    if root.upgrade().is_none(){break;}
                    let (Some(info),Some(progress))=(info.upgrade(),progress.upgrade()) else {break;};
                    let analyzed = matches!(&event, Event::Analyzed(_));
                    match event {
                        Event::Progress(text,fraction)=>{info.set_text(&text);progress.set_fraction(fraction);continue;}
                        Event::Analyzed(v)=>{ if let Some(end)=end.upgrade(){end.set_value(v.duration.min(300.0));} if let Some(start)=start.upgrade(){start.set_value(0.0);} *video.borrow_mut()=Some(v); }
                        Event::Imported(id)=>{progress.set_fraction(1.0);info.set_text("Video agregado a tu biblioteca. Selecciónalo y guarda los cambios para conservarlo como fondo.");imported(id);}
                        Event::Error(error)=>{info.set_text(&error);}
                    }
                    busy.set(false);
                    if let Some(w)=url.upgrade(){w.set_sensitive(true);}if let Some(w)=analyze.upgrade(){w.set_sensitive(true);}if let Some(w)=cancel.upgrade(){w.set_visible(false);}if let Some(w)=convert.upgrade(){w.set_sensitive(video.borrow().is_some());}
                    for w in [start.upgrade(),end.upgrade()].into_iter().flatten(){w.set_sensitive(true);}
                    for w in [quality.upgrade(),rate.upgrade(),density.upgrade()].into_iter().flatten(){w.set_sensitive(true);}
                    if analyzed {update();}
                    break;
                }
            });
        });
    }
    root
}

#[cfg(test)]
pub mod tests {
    use super::*;
    fn widgets(root: &gtk4::Widget) -> Vec<gtk4::Widget> {
        let mut result = vec![root.clone()];
        let mut child = root.first_child();
        while let Some(w) = child {
            result.extend(widgets(&w));
            child = w.next_sibling();
        }
        result
    }
    pub fn invalid_url_returns_to_idle() {
        let panel = controls(Rc::new(|_| panic!("Invalid URL cannot import")));
        let window = gtk4::Window::new();
        window.set_child(Some(&panel));
        window.present();
        let all = widgets(panel.upcast_ref());
        let entry = all
            .iter()
            .find_map(|w| w.clone().downcast::<gtk4::Entry>().ok())
            .unwrap();
        let button = |text: &str| {
            all.iter()
                .filter_map(|w| w.clone().downcast::<gtk4::Button>().ok())
                .find(|w| w.label().as_deref() == Some(text))
                .unwrap()
        };
        let analyze = button("Analizar URL");
        let convert = button("Convertir a ASCII");
        assert!(!convert.is_sensitive());
        entry.set_text("https://example.com/video");
        analyze.emit_clicked();
        assert!(!analyze.is_sensitive());
        assert!(!entry.is_sensitive());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !analyze.is_sensitive() && std::time::Instant::now() < deadline {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(analyze.is_sensitive());
        assert!(entry.is_sensitive());
        assert!(!convert.is_sensitive());
        assert!(all
            .iter()
            .filter_map(|w| w.clone().downcast::<gtk4::Label>().ok())
            .any(|w| w.text().contains("YouTube")));
        if std::env::var("PARTICLEWALL_YOUTUBE_GUI_TEST").as_deref() == Ok("1") {
            entry.set_text("https://www.youtube.com/watch?v=gU4vSEZwiyE");
            analyze.emit_clicked();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            while !analyze.is_sensitive() && std::time::Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(analyze.is_sensitive(), "YouTube worker timed out");
            assert!(
                convert.is_sensitive(),
                "Valid video should enable conversion"
            );
            assert!(all
                .iter()
                .filter_map(|w| w.clone().downcast::<gtk4::Label>().ok())
                .any(|w| w.text().contains("Duración: 21 s")));
        }
        window.close();
    }
}
