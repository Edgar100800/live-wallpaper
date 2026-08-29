//! GTK settings window with live-preview sliders (macOS
//! WallpaperCustomizationView parity): "Tamaño de puntos" (particleSize,
//! 0.5-4) and "Intensidad de puntos" (brightness, 0.25-10). Every change
//! flows through the regular command channel, so it applies instantly to
//! the GPU and web renderers and persists to config.json.
//!
//! Initial values are arguments (never a state borrow): the applier calls
//! this under an active RefCell borrow.

use crate::web::control::{CmdTx, Command};
use crate::web::library::ColorSettings;
use gtk4::prelude::*;
use gtk4::{Align, Button, Label, Orientation, Scale, Window};

/// Opens the settings window, or raises the existing one.
pub fn open(size: f64, brightness: f64, tx: CmdTx) {
    let window = Window::builder()
        .title("ParticleWall — Ajustes")
        .default_width(400)
        .default_height(190)
        .resizable(false)
        .build();

    let root = gtk4::Box::new(Orientation::Vertical, 12);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    window.set_child(Some(&root));

    // ---- particle size ----------------------------------------------------
    let size_row = gtk4::Box::new(Orientation::Horizontal, 8);
    let size_label = Label::new(Some("Tamaño de puntos"));
    size_label.set_width_chars(17);
    size_label.set_halign(Align::Start);
    let size_value = Label::new(Some(&format!("{size:.2}")));
    size_value.set_width_chars(5);
    size_value.set_halign(Align::End);
    let size_scale = Scale::with_range(Orientation::Horizontal, 0.5, 4.0, 0.05);
    size_scale.set_value(size);
    size_scale.set_hexpand(true);
    size_scale.add_mark(1.0, gtk4::PositionType::Bottom, Some("1"));
    size_scale.add_mark(1.6, gtk4::PositionType::Bottom, Some("1.6"));
    size_scale.add_mark(2.5, gtk4::PositionType::Bottom, Some("2.5"));
    size_row.append(&size_label);
    size_row.append(&size_scale);
    size_row.append(&size_value);
    root.append(&size_row);

    {
        let tx = tx.clone();
        let value_label = size_value.clone();
        size_scale.connect_value_changed(move |scale| {
            let v = scale.value();
            value_label.set_text(&format!("{v:.2}"));
            let _ = tx.send_blocking((
                None,
                Command::SetColors(ColorSettings {
                    size: Some(v as f64),
                    ..Default::default()
                }),
            ));
        });
    }

    // ---- brightness ---------------------------------------------------------
    let bright_row = gtk4::Box::new(Orientation::Horizontal, 8);
    let bright_label = Label::new(Some("Intensidad"));
    bright_label.set_width_chars(17);
    bright_label.set_halign(Align::Start);
    let bright_value = Label::new(Some(&format!("{brightness:.2}")));
    bright_value.set_width_chars(5);
    bright_value.set_halign(Align::End);
    let bright_scale = Scale::with_range(Orientation::Horizontal, 0.25, 10.0, 0.05);
    bright_scale.set_value(brightness);
    bright_scale.set_hexpand(true);
    bright_scale.add_mark(1.5, gtk4::PositionType::Bottom, Some("1.5"));
    bright_scale.add_mark(3.0, gtk4::PositionType::Bottom, Some("3"));
    bright_scale.add_mark(6.0, gtk4::PositionType::Bottom, Some("6"));
    bright_row.append(&bright_label);
    bright_row.append(&bright_scale);
    bright_row.append(&bright_value);
    root.append(&bright_row);

    {
        let tx = tx.clone();
        let value_label = bright_value.clone();
        bright_scale.connect_value_changed(move |scale| {
            let v = scale.value();
            value_label.set_text(&format!("{v:.2}"));
            let _ = tx.send_blocking((
                None,
                Command::SetColors(ColorSettings {
                    brightness: Some(v as f64),
                    ..Default::default()
                }),
            ));
        });
    }

    // ---- footer -------------------------------------------------------------
    let footer = gtk4::Box::new(Orientation::Horizontal, 8);
    footer.set_halign(Align::End);
    let hint = Label::new(Some("Se aplica y guarda en vivo"));
    hint.set_hexpand(true);
    hint.set_halign(Align::Start);
    hint.set_opacity(0.6);
    let close = Button::with_label("Cerrar");
    {
        let window = window.clone();
        close.connect_clicked(move |_| window.destroy());
    }
    footer.append(&hint);
    footer.append(&close);
    root.append(&footer);

    window.present();
}
