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
        .default_width(440)
        .default_height(260)
        .resizable(false)
        .build();

    let root = gtk4::Box::new(Orientation::Vertical, 20);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    window.set_child(Some(&root));

    // ---- particle size ----------------------------------------------------
    let size_group = gtk4::Box::new(Orientation::Vertical, 6);
    let size_row = gtk4::Box::new(Orientation::Horizontal, 8);
    let size_label = Label::new(Some("Tamaño de puntos"));
    size_label.set_hexpand(true);
    size_label.set_halign(Align::Start);
    let size_value = Label::new(Some(&format!("{size:.2}")));
    size_value.set_width_chars(5);
    size_value.set_halign(Align::End);
    let size_scale = Scale::with_range(Orientation::Horizontal, 0.25, 8.0, 0.05);
    size_scale.set_value(size);
    size_scale.set_hexpand(true);
    size_scale.set_draw_value(false);
    size_scale.add_mark(0.25, gtk4::PositionType::Bottom, Some("0.25"));
    size_scale.add_mark(1.0, gtk4::PositionType::Bottom, Some("1"));
    size_scale.add_mark(1.6, gtk4::PositionType::Bottom, Some("1.6"));
    size_scale.add_mark(2.5, gtk4::PositionType::Bottom, Some("2.5"));
    size_scale.add_mark(4.0, gtk4::PositionType::Bottom, Some("4"));
    size_scale.add_mark(6.0, gtk4::PositionType::Bottom, Some("6"));
    size_scale.add_mark(8.0, gtk4::PositionType::Bottom, Some("8"));
    size_row.append(&size_label);
    size_row.append(&size_value);
    size_group.append(&size_row);
    size_group.append(&size_scale);
    root.append(&size_group);

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
    let bright_group = gtk4::Box::new(Orientation::Vertical, 6);
    let bright_row = gtk4::Box::new(Orientation::Horizontal, 8);
    let bright_label = Label::new(Some("Intensidad"));
    bright_label.set_hexpand(true);
    bright_label.set_halign(Align::Start);
    let bright_value = Label::new(Some(&format!("{brightness:.2}")));
    bright_value.set_width_chars(5);
    bright_value.set_halign(Align::End);
    let bright_scale = Scale::with_range(Orientation::Horizontal, 0.25, 10.0, 0.05);
    bright_scale.set_value(brightness);
    bright_scale.set_hexpand(true);
    bright_scale.set_draw_value(false);
    bright_scale.add_mark(0.25, gtk4::PositionType::Bottom, Some("0.25"));
    bright_scale.add_mark(1.5, gtk4::PositionType::Bottom, Some("1.5"));
    bright_scale.add_mark(3.0, gtk4::PositionType::Bottom, Some("3"));
    bright_scale.add_mark(6.0, gtk4::PositionType::Bottom, Some("6"));
    bright_scale.add_mark(10.0, gtk4::PositionType::Bottom, Some("10"));
    bright_row.append(&bright_label);
    bright_row.append(&bright_value);
    bright_group.append(&bright_row);
    bright_group.append(&bright_scale);
    root.append(&bright_group);

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
