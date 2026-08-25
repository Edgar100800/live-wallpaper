//! M0A/M0B spike: proves a BACKGROUND layer surface renders behind all
//! windows on Hyprland without webkit. Builds with --no-default-features.
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Label, glib};

#[path = "../layer.rs"]
mod layer;

fn main() -> glib::ExitCode {
    let app = Application::builder()
        .application_id("com.particlewall.spike")
        .build();

    app.connect_activate(|app| {
        for monitor in layer::monitors() {
            let id = layer::monitor_identity(&monitor);
            let geometry = monitor.geometry();
            println!(
                "spike: output {id} {}x{} at {},{}",
                geometry.width(),
                geometry.height(),
                geometry.x(),
                geometry.y()
            );

            let window = ApplicationWindow::builder()
                .application(app)
                .css_classes(["pw-bg"])
                .build();
            layer::setup_layer_window(&window);

            let label = Label::new(Some(&format!("ParticleWall layer OK — {id}")));
            window.set_child(Some(&label));

            // Live clock proves frames are being composited.
            let id2 = id.clone();
            let label2 = label.clone();
            glib::timeout_add_seconds_local(1, move || {
                let time = glib::DateTime::now_local()
                    .and_then(|d| d.format("%H:%M:%S"))
                    .unwrap_or_default();
                label2.set_text(&format!("ParticleWall layer OK — {id2} — {time}"));
                glib::ControlFlow::Continue
            });

        let css = gtk4::CssProvider::new();
        css.load_from_string("window.pw-bg { background: #0b0e14; color: #7ee0c0; font-size: 28px; }");
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().expect("display"),
                &css,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );

            window.present();
            println!("spike: presented on {id}");
        }
    });

    app.run()
}
