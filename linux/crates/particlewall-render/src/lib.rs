//! particle-v1 engine: CPU reference implementation and GPU host.
//!
//! The CPU functions mirror the WGSL formula bit-for-bit (f32) and lock the
//! numeric contract via fixtures in shared/contracts/fixtures/renderer-vectors.
//! The GPU path renders the shared WGSL through wgpu (Vulkan on Linux).

pub mod cpu;
pub mod gpu;

#[cfg(test)]
mod parity_tests;

/// Uniform block shared with particle.wgsl (128 bytes, 16-byte aligned).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub time: f32,
    pub aspect: f32,
    pub point_size: f32,
    pub scale: f32,
    pub rotation: [f32; 3],
    _rot_pad: f32,
    pub position: [f32; 3],
    _pos_pad: f32,
    pub appearance: [f32; 4],
    pub model: [f32; 4],
    pub screen_fit: [f32; 4],
    pub graph: [f32; 4],
    pub viewport: [f32; 2],
    pub pad: [f32; 2],
}

impl Uniforms {
    pub const VERTEX_COUNT_MODEL0: u32 = 10_000;

    /// Instance (particle) count per model index, mirroring Swift vertexCount.
    pub const MODEL_VERTEX_COUNTS: [u32; 8] = [
        10_000,             // 0 parametric-waves
        30_000,             // 1 twin-vortex
        30_000,             // 2 orbital-bloom
        19_993 * 6,         // 3 hexagonal-rosette
        720 * 9 * 12,       // 4 noise-rain
        78_498,             // 5 prime-spiral
        512 + 64 * 96 * 6,  // 6 torus-orbit
        6_225 * 8,          // 7 chromatic-rings
    ];

    pub fn model_vertex_count(model: u32) -> u32 {
        Self::MODEL_VERTEX_COUNTS[(model as usize).min(7)]
    }

    pub fn defaults(aspect: f32, viewport_px: [f32; 2]) -> Self {
        Self {
            time: 0.0,
            aspect,
            point_size: 2.0, // pixel-ratio 1.0 * particleSize 1.6 clamped path
            scale: 1.0,
            rotation: [0.0; 3],
            _rot_pad: 0.0,
            position: [0.0; 3],
            _pos_pad: 0.0,
            appearance: [
                0xE8 as f32 / 255.0,
                0xFF as f32 / 255.0,
                0xFF as f32 / 255.0,
                1.5,
            ],
            model: [0.0, 0.0, 12.0, Self::VERTEX_COUNT_MODEL0 as f32],
            screen_fit: [0.0, 1.0, 1.0, 0.0],
            graph: [0.0, 0.1, 1.0, 3.0],
            viewport: viewport_px,
            pad: [0.0; 2],
        }
    }
}

/// Path to the shared engine WGSL, relative to this crate.
pub fn engine_wgsl_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../shared/backgrounds/engines/particle-v1/particle.wgsl")
}

pub fn engine_wgsl() -> String {
    std::fs::read_to_string(engine_wgsl_path())
        .expect("shared particle-v1 WGSL present")
}
