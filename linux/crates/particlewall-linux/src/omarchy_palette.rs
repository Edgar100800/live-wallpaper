//! Read Omarchy's generated palette without modifying desktop configuration.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Palette {
    background: [f32; 3],
    ink: [f32; 3],
    highlight: [f32; 3],
}

fn hex_rgb(value: &str) -> Option<[f32; 3]> {
    let value = value.strip_prefix('#')?;
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let packed = u32::from_str_radix(value, 16).ok()?;
    Some([
        ((packed >> 16) & 255) as f32 / 255.0,
        ((packed >> 8) & 255) as f32 / 255.0,
        (packed & 255) as f32 / 255.0,
    ])
}

fn parse(source: &str) -> Option<Palette> {
    let table: toml::Table = toml::from_str(source).ok()?;
    let color = |key: &str| table.get(key)?.as_str().and_then(hex_rgb);
    Some(Palette {
        background: color("background")?,
        // Prefer the system's cyan for the requested cool ASCII treatment.
        ink: color("cyan").or_else(|| color("accent")).or_else(|| color("foreground"))?,
        highlight: color("bright_cyan").or_else(|| color("foreground"))?,
    })
}

fn paths() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME") else { return Vec::new() };
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from).unwrap_or_else(|| Path::new(&home).join(".local/state"));
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from).unwrap_or_else(|| Path::new(&home).join(".config"));
    vec![
        state.join("omarchy/current/theme/colors.toml"),
        config.join("omarchy/current/theme/colors.toml"),
    ]
}

pub fn read() -> Option<Palette> {
    paths().iter().find_map(|path| parse(&std::fs::read_to_string(path).ok()?))
}

pub fn script(palette: Option<&Palette>) -> String {
    let json = serde_json::to_string(&palette).expect("palette is JSON serializable");
    format!("window.__pwSystemPalette = {json}; window.dispatchEvent(new Event('pw-system-palette'));")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_uses_cyan_and_handles_comments_and_fallbacks() {
        let palette = parse("background = '#0B0C16'\ncyan = '#7cf8f7' # cool\nforeground = '#ddf7ff'\n").unwrap();
        assert_eq!(palette.ink, hex_rgb("#7cf8f7").unwrap());
        assert_eq!(palette.highlight, hex_rgb("#ddf7ff").unwrap());
        let fallback = parse("background = '#000000'\naccent = '#123456'\nforeground = '#ffffff'").unwrap();
        assert_eq!(fallback.ink, hex_rgb("#123456").unwrap());
    }

    #[test]
    fn malformed_or_incomplete_palette_does_not_override_wallpaper() {
        assert!(parse("background = '#000000'").is_none());
        assert!(parse("background = 'bad'\ncyan = '#ffffff'\nforeground = '#ffffff'").is_none());
        assert!(hex_rgb("#é0000").is_none());
        assert!(script(None).contains("= null;"));
    }
}

/// Small, native color samples for the settings view. Values are parsed hex only.
pub fn description() -> String {
    match read() {
        Some(p) => {
            let hex = |rgb: [f32; 3]| format!("#{:02x}{:02x}{:02x}", (rgb[0] * 255.0).round() as u8, (rgb[1] * 255.0).round() as u8, (rgb[2] * 255.0).round() as u8);
            let ink = hex(p.ink); let bg = hex(p.background); let light = hex(p.highlight);
            format!("<span background=\"{bg}\">   </span> {bg}   <span background=\"{ink}\">   </span> {ink}   <span background=\"{light}\">   </span> {light}")
        }
        None => "Omarchy no está disponible; se usarán los colores originales.".into(),
    }
}
