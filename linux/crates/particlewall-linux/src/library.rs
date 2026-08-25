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
        "DefaultWallpaper" => "Ondas Paramétricas",
        "TwinVortexWallpaper" => "Vórtice Gemelo",
        "OrbitalBloomWallpaper" => "Flor Orbital",
        "HexagonalRosetteWallpaper" => "Roseta Hexagonal",
        "NoiseRainWallpaper" => "Lluvia de Ruido",
        "PrimeSpiralWallpaper" => "Espiral Prima",
        "TorusOrbitWallpaper" => "Órbita Toroidal",
        "ChromaticRingsWallpaper" => "Anillos Cromáticos",
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

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<String>,
    #[serde(default)]
    pub colors: ColorSettings,
    #[serde(default)]
    pub profiles: Vec<Profile>,
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

pub fn save_config(config: &Config) {
    if let Some(parent) = config_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(config_path(), format!("{body}\n"));
    }
}

/// Backwards-compatible helper used at startup.
pub fn load_selected() -> Option<String> {
    load_config().wallpaper
}

#[allow(dead_code)]
pub fn save_selected(id: &str) {
    let mut cfg = load_config();
    cfg.wallpaper = Some(id.into());
    save_config(&cfg);
}
