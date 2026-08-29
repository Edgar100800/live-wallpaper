//! CPU reference for the particle-v1 models. Mirrors particle.wgsl exactly
//! (f32 arithmetic, same operation order) so fixtures and GPU readback can be
//! compared bit-for-bit within rounding tolerance.

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

/// Vórtice Gemelo (400px canvas).
pub fn twin_vortex_pixel(id: u32, engine_time: f32) -> [f32; 2] {
    let t = engine_time;
    let i = 29999.0 - id as f32;
    let m = i % 2.0 * 3.0;
    let k = 14.0 * (i / 39.0).cos();
    let e = i / 1200.0 - 13.0;
    let d = (k * k + e * e) / 59.0 + 1.0;
    let q = 89.0 - k.sin() * d + k * (8.0 / d + (d * 3.0 + e / 9.0 - t).sin());
    let c = d * 0.45 - (t - d).sin() / 8.0 - t / 8.0 + m;
    [
        q * c.sin() + 200.0,
        (q + 40.0 + 30.0 * (c * 2.0 + m).sin()) * c.cos() + 200.0,
    ]
}

/// Flor Orbital (400px canvas).
pub fn orbital_bloom_pixel(id: u32, engine_time: f32) -> [f32; 2] {
    let t = engine_time;
    let i = 29999.0 - id as f32;
    let y = i / 799.0;
    let k = 5.0 * (i / 48.0).cos();
    let e = 5.0 * (y / 9.0).cos();
    let divisor = 6.0 + i % 4.0;
    let d = ((k * k + e * e).sqrt() / divisor).powi(4) + 4.0;
    let parity_offset = 80.0 * (1.0 + i % 2.0);
    let q = k * (3.0 + e / 2.0 * (d * 8.0 + k / 9.0 - t).sin())
        - 3.0 * (k * d / 3.0).sin()
        + parity_offset;
    let c = d - t / 9.0 + i % 5.0;
    [
        q * c.sin() + 200.0,
        q * (c - i % 2.0 + i % 5.0 * 3.0 + 7.0).cos() + 200.0,
    ]
}

/// Roseta Hexagonal (400px canvas).
pub fn hexagonal_rosette_pixel(id: u32, engine_time: f32) -> [f32; 2] {
    let t = engine_time;
    let sample = id / 6;
    let rot = id % 6;
    let i = 19999.0 - sample as f32;
    let k = i % 25.0 - 12.0;
    let e = i / 800.0;
    let d = 7.0 * ((k * k + e * e).sqrt() / 3.0 + t / 2.0).cos();
    let cx = k * 4.0 + d * k * (d + e / 9.0 + t).sin();
    let cy = e * 2.0 - d * 9.0 - d * 9.0 * (d + t).cos();
    let angle = rot as f32 * 1.04719755;
    let (ca, sa) = (angle.cos(), angle.sin());
    [cx * ca - cy * sa + 200.0, cx * sa + cy * ca + 200.0]
}

/// Value-noise hash shared by Lluvia de Ruido and Anillos Cromáticos.
pub fn flow_hash(p: [f32; 3]) -> f32 {
    let mut v = [p[0] * 0.1031, p[1] * 0.1031, p[2] * 0.1031].map(|x| x.fract());
    let d = v[0] * (v[1] + 33.33) + v[1] * (v[2] + 33.33) + v[2] * (v[0] + 33.33);
    v = [v[0] + d, v[1] + d, v[2] + d];
    ((v[0] + v[1]) * v[2]).fract()
}

/// Smooth value noise; matches flowNoise in the WGSL.
pub fn flow_noise(p: [f32; 3]) -> f32 {
    let cell = [p[0].floor(), p[1].floor(), p[2].floor()];
    let mut fraction = [p[0].fract(), p[1].fract(), p[2].fract()];
    for f in &mut fraction {
        *f = *f * *f * (3.0 - 2.0 * *f);
    }
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = mix(flow_hash([cell[0], cell[1], cell[2]]),
                 flow_hash([cell[0] + 1.0, cell[1], cell[2]]), fraction[0]);
    let x10 = mix(flow_hash([cell[0], cell[1] + 1.0, cell[2]]),
                 flow_hash([cell[0] + 1.0, cell[1] + 1.0, cell[2]]), fraction[0]);
    let x01 = mix(flow_hash([cell[0], cell[1], cell[2] + 1.0]),
                 flow_hash([cell[0] + 1.0, cell[1], cell[2] + 1.0]), fraction[0]);
    let x11 = mix(flow_hash([cell[0], cell[1] + 1.0, cell[2] + 1.0]),
                 flow_hash([cell[0] + 1.0, cell[1] + 1.0, cell[2] + 1.0]), fraction[0]);
    mix(mix(x00, x10, fraction[1]), mix(x01, x11, fraction[1]), fraction[2])
}

pub const FLOW_PARTICLE_COUNT: u32 = 720 * 9;
pub const FLOW_HISTORY_COUNT: u32 = 12;

/// Persistent flow state for Lluvia de Ruido. Mirrors the WGSL flowUpdate
/// kernel: one `step` advances every particle by one fixed tick.
#[derive(Clone)]
pub struct FlowState {
    pub frame: u32,
    pub particles: Vec<[f32; 4]>,
    pub history: Vec<[f32; 4]>,
}

impl FlowState {
    pub fn new() -> Self {
        Self {
            frame: 0,
            particles: vec![[0.0; 4]; FLOW_PARTICLE_COUNT as usize],
            history: vec![[0.0; 4]; (FLOW_PARTICLE_COUNT * FLOW_HISTORY_COUNT) as usize],
        }
    }

    pub fn step(&mut self) {
        let particle_count = FLOW_PARTICLE_COUNT;
        let history_count = FLOW_HISTORY_COUNT;
        let frame = self.frame;
        let first = (frame * 9) % particle_count;
        for id in 0..particle_count {
            let insertion_offset = (id + particle_count - first) % particle_count;
            let spawned = insertion_offset < 9;
            let mut state = self.particles[id as usize];

            if spawned {
                let new_tick = frame * 9 + insertion_offset + 1;
                state = [(new_tick as f32 * 99.0) % 720.0, 0.0, 0.0, 3.0];
                for slot in 0..history_count {
                    self.history[(id * history_count + slot) as usize] = [0.0; 4];
                }
            } else if state[3] <= 0.0 {
                continue;
            }

            state[3] *= 0.997;
            let global_tick = ((frame + 1) * 9) as f32;
            let n = flow_noise([state[0] / 720.0, state[1] / 9.0, global_tick / 720.0]);
            if n > 0.4 {
                state[2] += 0.5;
                state[1] += state[2];
            } else {
                state[0] += if n % 0.1 > 0.05 { 1.0 } else { -1.0 };
                state[2] = 0.0;
                state[1] += 0.5;
            }

            self.particles[id as usize] = state;
            let history_slot = frame % history_count;
            self.history[(id * history_count + history_slot) as usize] =
                [state[0], state[1], state[2], 1.0];
        }
        self.frame += 1;
    }
}

/// Espiral Prima reads the prime list uploaded to flowParticles.
pub const PRIME_LIMIT: u32 = 999_999;
pub const PRIME_COUNT: usize = 78_498;

/// Sieve of Eratosthenes; 78,498 primes below 1,000,000.
pub fn generate_primes(limit: u32) -> Vec<f32> {
    if limit < 2 {
        return Vec::new();
    }
    let n = limit as usize;
    let mut composite = vec![false; n + 1];
    let mut p = 2usize;
    while p * p <= n {
        if !composite[p] {
            (p * p..=n).step_by(p).for_each(|i| composite[i] = true);
        }
        p += 1;
    }
    (2..=n).filter(|&i| !composite[i]).map(|i| i as f32).collect()
}

/// Órbita Toroidal (400px canvas). Returns the pixel position and updates
/// `point_scale` in place (0.75 for the light sphere, 0.42 for torus points).
pub fn torus_orbit_pixel(id: u32, engine_time: f32, point_scale: &mut f32) -> [f32; 2] {
    let f = engine_time;
    let world: [f32; 3];
    if id < 512 {
        let sample = id as f32 + 0.5;
        let sphere_y = 1.0 - 2.0 * sample / 512.0;
        let sphere_radius = (1.0 - sphere_y * sphere_y).max(0.0).sqrt();
        let sphere_angle = sample * 2.39996323;
        let unit_sphere = [
            sphere_angle.cos() * sphere_radius,
            sphere_y,
            sphere_angle.sin() * sphere_radius,
        ];
        world = [
            60.0 * f.sin() + 99.0 * (-f).sin() + unit_sphere[0] * 4.0,
            -20.0 + unit_sphere[1] * 4.0,
            170.0 + 99.0 * f.cos() + unit_sphere[2] * 4.0,
        ];
        *point_scale = 0.75;
    } else {
        let local_id = id - 512;
        let torus_id = local_id / (96 * 6);
        let torus_point = local_id % (96 * 6);
        let major_id = torus_point / 6;
        let tube_id = torus_point % 6;
        let major = major_id as f32 * 6.28318531 / 96.0;
        let tube = tube_id as f32 * 6.28318531 / 6.0;
        let radius = 80.0 + 4.0 * tube.cos();
        let mut local = [radius * major.cos(), radius * major.sin(), 4.0 * tube.sin()];
        let (cx, sx) = (0.8f32.cos(), 0.8f32.sin());
        local = [local[0], local[1] * cx - local[2] * sx, local[1] * sx + local[2] * cx];
        local[0] += 120.0;

        let ring_angle = torus_id as f32 * 3.14159265 / 32.0 + f;
        let (cy, sy) = (ring_angle.cos(), ring_angle.sin());
        world = [
            local[0] * cy + local[2] * sy + 60.0 * f.sin(),
            local[1] - 20.0,
            -local[0] * sy + local[2] * cy + 170.0,
        ];
        *point_scale = 0.42;
    }

    let eye_z = 346.41016;
    let perspective = eye_z / (eye_z - world[2]).max(72.0);
    [200.0 + world[0] * perspective, 200.0 + world[1] * perspective]
}

/// Toro de Esferas (600px canvas). Ring of spheres with breathing radius;
/// returns the pixel position and updates `point_scale` (cos(v)+0.3).
pub fn sphere_torus_pixel(id: u32, engine_time: f32, point_scale: &mut f32) -> [f32; 2] {
    let t = engine_time % 1.0;
    let p = 0.078539816f32; // PI/40
    let y = id / 80;
    let x = id % 80;
    let v = (y as f32 + t) * p * 2.0;
    let u = (x as f32 + t) * p;
    let r = 90.0;
    let ring = 2.0 + v.sin();
    let mut local = [
        ring * u.cos() * r,
        ring * u.sin() * r,
        v.cos() * r,
    ];

    // p5 applies rotateX(.5) then rotateY(-.5); the model matrix is Rx * Ry,
    // so rotateY lands on the point first.
    let (cy, sy) = ((-0.5f32).cos(), (-0.5f32).sin());
    local = [local[0] * cy + local[2] * sy, local[1], -local[0] * sy + local[2] * cy];
    let (cx, sx) = (0.5f32.cos(), 0.5f32.sin());
    local = [local[0], local[1] * cx - local[2] * sx, local[1] * sx + local[2] * cx];

    // p5 default WEBGL camera for a 600px canvas: 300 / tan(PI/6).
    let eye_z = 519.61524;
    let perspective = eye_z / (eye_z - local[2]).max(72.0);
    *point_scale = (v.cos() + 0.3).max(0.1);
    [300.0 + local[0] * perspective, 300.0 + local[1] * perspective]
}

/// Medusa de Puntos (400px canvas). The JS `y^8` term is a bitwise XOR on
/// the int32 truncation of y, mirrored with an i32 cast.
pub fn jellyfish_pixel(id: u32, engine_time: f32) -> [f32; 2] {
    let t = engine_time;
    let i = 9999.0 - id as f32;
    let y = i / 345.0;
    let base = if y < 11.0 {
        6.0 + ((y as i32 ^ 8) as f32).sin() * 6.0
    } else {
        y / 5.0 + (y / 2.0).cos()
    };
    let k = base * (i - t / 4.0).cos();
    let e = y / 7.0 - 13.0;
    let d = (k * k + e * e).sqrt() + (e / 4.0 + t).sin() / 2.0;
    let c = d / 2.0 + 1.0 - t / 2.0;
    let q = y * k / d * (3.0 + (d * 2.0 + y / 2.0 - t * 4.0).sin());
    [q + 60.0 * c.cos() + 200.0, q * c.sin() + d * 29.0 - 170.0]
}

/// Full shared-canvas transform, mirroring the WGSL block. Returns
/// `[clip_x, clip_y, point_size_px, r, g, b, a]`.
pub fn model_sample(id: u32, model: u32, u: &Uniforms, flow: &FlowState) -> [f32; 7] {
    let pixel;
    let mut canvas_size = 400.0;
    let mut point_scale = 1.0;
    let mut trail_alpha = 1.0;
    let mut render_color = [u.appearance[0], u.appearance[1], u.appearance[2]];

    match model {
        0 => {
            let t = u.time * 1.17809725;
            pixel = parametric_waves_pixel(id, t);
        }
        1 => {
            let t = u.time * 2.09439510;
            pixel = twin_vortex_pixel(id, t);
        }
        2 => {
            let t = u.time * 1.57079633;
            pixel = orbital_bloom_pixel(id, t);
        }
        3 => {
            let t = u.time * 0.39269908;
            pixel = hexagonal_rosette_pixel(id, t);
        }
        4 => {
            let history_count = FLOW_HISTORY_COUNT;
            let particle_id = id / history_count;
            let trail_id = id % history_count;
            let latest_slot = (u.model[1] + 0.5) as u32;
            let slot = (latest_slot + history_count - trail_id) % history_count;
            let s = flow.history[(particle_id * history_count + slot) as usize];
            pixel = [s[0], s[1]];
            canvas_size = 720.0;
            point_scale = (s[2] / 3.0).max(0.16);
            trail_alpha = s[3] * (-(trail_id as f32) * 0.2).exp();
        }
        5 => {
            let prime = flow.particles[id as usize][0];
            let t = 1.0 + u.time * 0.000003;
            pixel = [prime * (prime * t).sin() / 99.0 + 400.0,
                     prime * (prime * t).cos() / 99.0 + 400.0];
            canvas_size = 800.0;
            point_scale = 0.38;
        }
        6 => {
            pixel = torus_orbit_pixel(id, u.time * 0.3, &mut point_scale);
        }
        8 => {
            // t wraps every unit in the source dweet.
            let t = u.time * 1.2;
            pixel = sphere_torus_pixel(id, t, &mut point_scale);
            canvas_size = 600.0;
        }
        9 => {
            // 8*PI wrap keeps t, t/2, t/4 and 4t seamless (see the WGSL).
            let t = (u.time * 1.57079633) % 25.13274123;
            pixel = jellyfish_pixel(id, t);
        }
        _ => {
            let trail_count = 8u32;
            let base_id = id / trail_count;
            let trail_id = id % trail_count;
            let mut local_id = base_id;
            let mut d = 330.0f32;
            let mut ring_point_count;
            loop {
                ring_point_count = (3.14159265 * d).ceil() as u32;
                if local_id < ring_point_count {
                    break;
                }
                local_id -= ring_point_count;
                d -= 30.0;
            }

            let t = (u.time * 30.0 - trail_id as f32 * 0.85).max(0.0);
            let r = local_id as f32 * 2.0 / d;
            let tangent = (d / 199.0 - t / 99.0).tan();
            let energy = tangent * tangent;
            let energy = if energy < 64.0 { energy } else { 64.0 };
            let noise_value = flow_noise([r * 99.0, d, 0.0]);
            let radius = d
                + (r * 9.0 + t / 9.0 * d / 720.0).sin() * d * 0.25 * noise_value * energy;
            let angle = r - 1.57079633;
            pixel = [angle.cos() * radius + 360.0, angle.sin() * radius + 360.0];
            canvas_size = 720.0;
            point_scale = 0.9;

            let hue = d.clamp(0.0, 255.0) / 255.0;
            let hsb = hsv_to_rgb([hue, 50.0 / 255.0, 1.0]);
            render_color = [hsb[0] * u.appearance[0], hsb[1] * u.appearance[1], hsb[2] * u.appearance[2]];
            let source_alpha = (0.7 / (energy.max(0.025))).clamp(0.018, 0.82);
            trail_alpha = source_alpha * (-(trail_id as f32) * 0.32).exp();
        }
    }

    let clip = canvas_to_clip(pixel, canvas_size, u);
    let point_size = (u.point_size * point_scale).max(1.0);
    let brightness = u.appearance[3].max(0.05);
    // Asymptotic alpha mirrors the WGSL: never saturates, so the whole
    // brightness range stays visible (0.38*b clamped died at ~2.6).
    let alpha = (1.0 - (-0.55 * brightness).exp()) * trail_alpha;
    [
        clip[0],
        clip[1],
        point_size,
        render_color[0] * brightness,
        render_color[1] * brightness,
        render_color[2] * brightness,
        alpha,
    ]
}

/// p5-style HSB (0..1) to RGB; mirrors hsvToRGB in the WGSL.
pub fn hsv_to_rgb(hsv: [f32; 3]) -> [f32; 3] {
    let p = [
        ((hsv[0] + 1.0).fract() * 6.0 - 3.0).abs(),
        ((hsv[0] + 2.0 / 3.0).fract() * 6.0 - 3.0).abs(),
        ((hsv[0] + 1.0 / 3.0).fract() * 6.0 - 3.0).abs(),
    ];
    let clamped = [p[0] - 1.0, p[1] - 1.0, p[2] - 1.0];
    let mix_component = |c: f32| {
        let cl = c.clamp(0.0, 1.0);
        1.0 + (cl - 1.0) * hsv[1]
    };
    [
        hsv[2] * mix_component(clamped[0]),
        hsv[2] * mix_component(clamped[1]),
        hsv[2] * mix_component(clamped[2]),
    ]
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
pub fn parametric_waves_sample(id: u32, u: &Uniforms) -> [f32; 7] {
    model_sample(id, 0, u, &FlowState::new())
}

// ---------------------------------------------------------------------------
// Graph overlay reference (768 nodes sampled from the active model).
// ---------------------------------------------------------------------------

pub const GRAPH_NODE_COUNT: u32 = 768;
pub const GRAPH_MAX_CONNECTIONS: u32 = 3;

/// Node positions + connection pass. Mirrors graphPositionUpdate and
/// graphConnectionUpdate in the WGSL.
pub fn graph_update(u: &Uniforms, flow: &FlowState) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let mut positions = vec![[0.0; 4]; GRAPH_NODE_COUNT as usize];
    let source_count = (u.model[3] + 0.5) as u32;
    let source_count = source_count.max(1);
    for (id, pos) in positions.iter_mut().enumerate() {
        let source_id = (((id as f32) * (source_count as f32) / (GRAPH_NODE_COUNT as f32)) as u32)
            .min(source_count - 1);
        let s = model_sample(source_id, u.model[0] as u32, u, flow);
        let visible = s[0].abs() <= 1.15 && s[1].abs() <= 1.15 && s[6] > 0.005;
        *pos = [s[0], s[1], if visible { s[6] } else { 0.0 }, source_id as f32];
    }

    let mut edges = vec![[0.0; 4]; (GRAPH_NODE_COUNT * GRAPH_MAX_CONNECTIONS * 2) as usize];
    let threshold = u.graph[1].max(0.001);
    let threshold_squared = threshold * threshold;
    let requested = (u.graph[3] + 0.5) as u32;
    let requested = requested.min(GRAPH_MAX_CONNECTIONS);
    for id in 0..GRAPH_NODE_COUNT as usize {
        let source = positions[id];
        let mut best0 = (threshold_squared, u32::MAX);
        let mut best1 = (threshold_squared, u32::MAX);
        let mut best2 = (threshold_squared, u32::MAX);
        if source[2] > 0.0 {
            for candidate in (id as u32 + 1)..GRAPH_NODE_COUNT {
                let target = positions[candidate as usize];
                if target[2] <= 0.0 {
                    continue;
                }
                let dx = source[0] - target[0];
                let dy = source[1] - target[1];
                let d2 = dx * dx + dy * dy;
                if d2 >= best2.0 {
                    continue;
                }
                if d2 < best0.0 {
                    best2 = best1;
                    best1 = best0;
                    best0 = (d2, candidate);
                } else if d2 < best1.0 {
                    best2 = best1;
                    best1 = (d2, candidate);
                } else {
                    best2 = (d2, candidate);
                }
            }
        }

        for slot in 0..GRAPH_MAX_CONNECTIONS as usize {
            let (d2, target_id) = match slot {
                0 => best0,
                1 => best1,
                _ => best2,
            };
            let edge = (id as u32 * GRAPH_MAX_CONNECTIONS + slot as u32) as usize * 2;
            if (slot as u32) < requested && target_id != u32::MAX {
                let target = positions[target_id as usize];
                let proximity = 1.0 - d2.sqrt() / threshold;
                let alpha = (u.graph[2] * proximity * source[2].min(target[2])).clamp(0.0, 1.0);
                edges[edge] = [source[0], source[1], alpha, 0.0];
                edges[edge + 1] = [target[0], target[1], alpha, 0.0];
            } else {
                edges[edge] = [0.0; 4];
                edges[edge + 1] = [0.0; 4];
            }
        }
    }
    (positions, edges)
}
