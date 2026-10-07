//! Shared layer-shell window setup for Hyprland/Wayland.
//!
//! Every wallpaper surface is a BOTTOM layer surface anchored to all four
//! edges with exclusive zone -1 (full output) and no keyboard mode. Bottom
//! (not Background) is deliberate: shells map their own static wallpaper
//! into Background (Omarchy's quickshell included) at unpredictable times
//! after us, and same-layer surfaces stack in map order, so a Background
//! wallpaper gets covered at boot. Bottom always renders above Background
//! and below regular windows, with no race to lose.

use gtk4::prelude::*;
use gtk4::{ApplicationWindow, gdk};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

pub const NAMESPACE: &str = "particlewall";

/// Configures an unmanaged window as a full-output bottom layer surface.
/// Must run before the window is mapped.
pub fn setup_layer_window(window: &ApplicationWindow) {
    window.init_layer_shell();
    window.set_layer(Layer::Bottom);
    window.set_namespace(Some(NAMESPACE));
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
}

/// Stable-ish identity for persistence: connector name when available,
/// otherwise the monitor description/model.
pub fn monitor_identity(monitor: &gdk::Monitor) -> String {
    monitor
        .connector()
        .or_else(|| monitor.description())
        .or_else(|| monitor.model())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Enumerates current outputs.
pub fn monitors() -> Vec<gdk::Monitor> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    let list = display.monitors();
    (0..list.n_items())
        .filter_map(|i| list.item(i).and_downcast::<gdk::Monitor>())
        .collect()
}
