//! Manifest v1 compatibility and renderer-to-module mapping.
//!
//! Mirrors Models.swift. Renderer wire names ("metal-*") are macOS-era names;
//! module ids are neutral and shared with Linux. Unknown renderer values are
//! invalid, matching strict enum decoding in Swift.

use serde::Deserialize;

pub const MODULE_PARAMETRIC_WAVES: &str = "parametric-waves";
pub const MODULE_WEB: &str = "web";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererKind {
    Web,
    MetalParticles,
    MetalTwinVortex,
    MetalOrbitalBloom,
    MetalHexagonalRosette,
    MetalNoiseRain,
    MetalPrimeSpiral,
    MetalTorusOrbit,
    MetalChromaticRings,
}

impl RendererKind {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "web" => Self::Web,
            "metal-particles" => Self::MetalParticles,
            "metal-twin-vortex" => Self::MetalTwinVortex,
            "metal-orbital-bloom" => Self::MetalOrbitalBloom,
            "metal-hexagonal-rosette" => Self::MetalHexagonalRosette,
            "metal-noise-rain" => Self::MetalNoiseRain,
            "metal-prime-spiral" => Self::MetalPrimeSpiral,
            "metal-torus-orbit" => Self::MetalTorusOrbit,
            "metal-chromatic-rings" => Self::MetalChromaticRings,
            _ => return None,
        })
    }

    pub fn module_id(self) -> &'static str {
        match self {
            Self::Web => MODULE_WEB,
            Self::MetalParticles => MODULE_PARAMETRIC_WAVES,
            Self::MetalTwinVortex => "twin-vortex",
            Self::MetalOrbitalBloom => "orbital-bloom",
            Self::MetalHexagonalRosette => "hexagonal-rosette",
            Self::MetalNoiseRain => "noise-rain",
            Self::MetalPrimeSpiral => "prime-spiral",
            Self::MetalTorusOrbit => "torus-orbit",
            Self::MetalChromaticRings => "chromatic-rings",
        }
    }
}

// Unknown extra fields are ignored, matching Swift's Codable behavior.
#[derive(Debug, Clone, Deserialize)]
pub struct ManifestV1 {
    pub name: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    pub source: String,
    #[serde(default)]
    pub fps: Option<i64>,
    #[serde(default)]
    pub renderer: Option<String>,
}

impl ManifestV1 {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Effective shared-module id. Mirrors WallpaperManifest.effectiveRenderer:
    /// explicit renderer wins; otherwise bundled defaults to parametric-waves,
    /// everything else to web. Returns None for unknown renderer values.
    pub fn effective_module(&self) -> Option<&'static str> {
        match &self.renderer {
            Some(raw) => RendererKind::parse(raw).map(|k| k.module_id()),
            None => Some(match self.source.as_str() {
                "bundled" => MODULE_PARAMETRIC_WAVES,
                _ => MODULE_WEB,
            }),
        }
    }
}
