//! CPU reference for the particle-v1 models. Mirrors particleSample in
//! particle.wgsl exactly (f32 arithmetic, same operation order).

use crate::Uniforms;

/// Ondas Paramétricas canvas-space pixel position (400px canvas).
/// `engine_time` is the ALREADY-SCALED time (u.time * 1.17809725), exactly
/// like the `t` local in the WGSL/Swift sample function.
pub fn parametric_waves_pixel(id: u32, engine_time: f32) -> [f32; 2] {
    let t = engine_time;
    let i = 9999.0 - id as f32;
    let y = i / 235.0;
    let k = (4.0 + (i / 9.0 - t * 2.0).cos()) * (i / 35.0).cos();
    let e = y / 7.0 - 13.0;
    let d = (k * k + e * e).sqrt() + (e / 9.0 + t / 2.0).sin() - 4.0;
    let q = 2.0 * (k * 3.0).sin()
        - y / 35.0 * k * (9.0 + k * (e.cos() * 9.0 - d * 2.0 + t).sin());
    let c = d - t;
    [q + 40.0 * c.cos() + 200.0, q * c.sin() + d * 35.0]
}

/// Shared canvas->clip transform, mirroring the WGSL block.
pub fn canvas_to_clip(pixel: [f32; 2], canvas_size: f32, u: &Uniforms) -> [f32; 2] {
    let center = canvas_size * 0.5;
    let mut p = [
        (pixel[0] - center) / center,
        (center - pixel[1]) / center,
    ];
    let (cz, sz) = (u.rotation[2].cos(), u.rotation[2].sin());
    p = [p[0] * cz - p[1] * sz, p[0] * sz + p[1] * cz];
    let zoom = u.scale * (u.position[2] * 0.08).exp();
    p = [p[0] * zoom, p[1] * zoom];
    p = [p[0] * u.rotation[1].cos(), p[1] * u.rotation[0].cos()];
    p[0] += u.position[0] * 0.25;
    p[1] += u.position[1] * 0.25;
    p[0] *= u.screen_fit[1].max(0.05);
    p[1] *= u.screen_fit[2].max(0.05);
    if u.screen_fit[0] < 0.5 {
        p[0] /= u.aspect.max(0.1);
    }
    p
}

/// Full model-0 sample: clip position + sprite factors.
pub fn parametric_waves_sample(id: u32, u: &Uniforms) -> [f32; 2 + 1 + 4] {
    let t = u.time * 1.17809725;
    let pixel = parametric_waves_pixel(id, t);
    let clip = canvas_to_clip(pixel, 400.0, u);
    let point_size = (u.point_size * 1.0).max(1.0);
    let brightness = u.appearance[3].max(0.05);
    let alpha = (0.38 * brightness).clamp(0.08, 1.0) * 1.0;
    [
        clip[0],
        clip[1],
        point_size,
        u.appearance[0] * brightness,
        u.appearance[1] * brightness,
        u.appearance[2] * brightness,
        alpha,
    ]
}
