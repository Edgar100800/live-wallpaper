//! Built-in wallpaper library: the bundled HTML fallbacks of the eight GPU
//! models (same formulas, rendered via WebKit until the shared GPU modules
//! of phases M1/M2 land on Linux).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Wallpaper {
    /// Folder name, used as stable id.
    pub id: String,
    /// Human name matching the macOS gallery.
    pub name: String,
    pub index: PathBuf,
}

/// Repo layout during development: <repo>/Sources/ParticleWall/Resources.
pub fn resources_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Sources/ParticleWall/Resources")
}

fn pretty_name(folder: &str) -> String {
    match folder {
        "SpiderManASCIIWallpaper" => "Spider-Man ASCII",
        "DefaultWallpaper" => "Ondas Paramétricas",
        "TwinVortexWallpaper" => "Vórtice Gemelo",
        "OrbitalBloomWallpaper" => "Flor Orbital",
        "HexagonalRosetteWallpaper" => "Roseta Hexagonal",
        "NoiseRainWallpaper" => "Lluvia de Ruido",
        "PrimeSpiralWallpaper" => "Espiral Prima",
        "TorusOrbitWallpaper" => "Órbita Toroidal",
        "ChromaticRingsWallpaper" => "Anillos Cromáticos",
        "SphereTorusWallpaper" => "Toro de Esferas",
        "JellyfishPointsWallpaper" => "Medusa de Puntos",
        "NebulaWallpaper" => "Nebulosa",
        "TorusKnotWallpaper" => "Tesseract Cuántico",
        other => other,
    }
    .into()
}

pub fn bundled() -> Vec<Wallpaper> {
    let dir = resources_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut list: Vec<Wallpaper> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("index.html").is_file())
        .map(|p| Wallpaper {
            id: p.file_name().unwrap().to_string_lossy().into_owned(),
            name: pretty_name(&p.file_name().unwrap().to_string_lossy()),
            index: p.join("index.html"),
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

pub fn find(id_or_name: &str) -> Option<Wallpaper> {
    let all = bundled();
    all.iter()
        .find(|w| w.id == id_or_name || w.name.eq_ignore_ascii_case(id_or_name))
        .cloned()
}

// ---- persisted configuration ---------------------------------------------

/// Appearance settings using the JS contract's wire names.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ColorSettings {
    #[serde(rename = "backgroundColor", default, skip_serializing_if = "Option::is_none")]
    pub background: Option<u32>,
    #[serde(rename = "particleColor", default, skip_serializing_if = "Option::is_none")]
    pub particle: Option<u32>,
    #[serde(rename = "particleSize", default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// "Intensidad de puntos" on macOS (appearance.w). None = model default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f64>,
}

impl ColorSettings {
    pub fn resolved(&self) -> Self {
        Self { background: Some(self.background.unwrap_or(198153)),
            particle: Some(self.particle.unwrap_or(15269887)),
            size: Some(self.size.unwrap_or(1.6)), brightness: Some(self.brightness.unwrap_or(1.5)) }
    }
}

/// A saved background+particle pair, mirroring macOS color profiles.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(rename = "backgroundColor")]
    pub background: u32,
    #[serde(rename = "particleColor")]
    pub particle: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ASCIIColorMode {
    Original,
    #[default]
    Omarchy,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ASCIISettings {
    pub color_mode: ASCIIColorMode,
    pub clean_background: bool,
    pub separation: f64,
}

impl Default for ASCIISettings {
    fn default() -> Self {
        Self { color_mode: ASCIIColorMode::Omarchy, clean_background: true, separation: 0.5 }
    }
}

impl ASCIISettings {
    pub fn script(&self) -> String {
        format!("window.__pwASCIISettings = {}; window.dispatchEvent(new Event('pw-ascii-settings'));",
            serde_json::to_string(self).unwrap())
    }
}

fn default_fps() -> u32 { 30 }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<String>,
    #[serde(default)]
    pub colors: ColorSettings,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default = "default_fps", rename = "fpsCap")]
    pub fps_cap: u32,
    #[serde(default, rename = "asciiSettings")]
    pub ascii: std::collections::BTreeMap<String, ASCIISettings>,
}

impl Default for Config {
    fn default() -> Self {
        Self { wallpaper: None, colors: ColorSettings::default(), profiles: Vec::new(),
               fps_cap: default_fps(), ascii: Default::default() }
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.config")))
        .unwrap_or_else(|| "/tmp".into());
    Path::new(&base).join("particlewall/config.json")
}

pub fn load_config() -> Config {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_config(config: &Config) -> std::io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let body = serde_json::to_string_pretty(config)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, format!("{body}\n"))?;
    std::fs::rename(temporary, path)

}

/// Backwards-compatible helper used at startup.
pub fn load_selected() -> Option<String> {
    load_config().wallpaper
}

#[allow(dead_code)]
pub fn save_selected(id: &str) {
    let mut cfg = load_config();
    cfg.wallpaper = Some(id.into());
    let _ = save_config(&cfg);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_configuration_gets_compatible_defaults() {
        let cfg: Config = serde_json::from_str(r#"{"wallpaper":"SpiderManASCIIWallpaper","colors":{},"profiles":[]}"#).unwrap();
        assert_eq!(cfg.fps_cap, 30);
        assert!(cfg.ascii.is_empty());
        assert_eq!(ASCIISettings::default().color_mode, ASCIIColorMode::Omarchy);
    }
    #[test]
    fn ascii_modes_and_independent_cleanup_round_trip_per_wallpaper() {
        let mut cfg = Config::default();
        cfg.fps_cap = 15;
        cfg.ascii.insert("SpiderManASCIIWallpaper".into(), ASCIISettings {
            color_mode: ASCIIColorMode::Original, clean_background: true, separation: 0.8 });
        cfg.ascii.insert("OtherASCII".into(), ASCIISettings {
            color_mode: ASCIIColorMode::Omarchy, clean_background: false, separation: 0.1 });
        let encoded = serde_json::to_string(&cfg).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&encoded).unwrap(), cfg);
        assert!(encoded.contains("original"));
    }
}
