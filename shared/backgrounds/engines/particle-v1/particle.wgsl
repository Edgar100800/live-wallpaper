// particle-v1: shared particle engine (WGSL source of truth).
//
// Mirrors MetalParticleRenderer in Sources/ParticleWall/WallpaperRenderer.swift.
// Point sprites are expanded to instanced quads (WebGPU has no point_size):
// 6 vertices per particle, vertex_index/6 = particle id.
// Uniforms layout must stay 16-byte aligned and identical to the Swift
// Uniforms struct; viewport is the engine-v1 addition for quad expansion.

struct Uniforms {
  time: f32,
  aspect: f32,
  pointSize: f32,
  scale: f32,
  rotation: vec3f,
  position: vec3f,
  appearance: vec4f,
  model: vec4f,
  screenFit: vec4f,
  graph: vec4f,
  viewport: vec2f,
  pad: vec2f,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

struct ParticleSample {
  // Clip-space xy after the shared transform.
  clip: vec2f,
  // Physical pixel size of the sprite quad.
  pointSizePx: f32,
  color: vec4f,
}

fn hsvToRGB(hsv: vec3f) -> vec3f {
  let constants = vec4f(1.0, 0.66666667, 0.33333333, 3.0);
  let p = abs(fract(vec3f(hsv.x) + constants.xyz) * 6.0 - constants.www);
  return hsv.z * mix(vec3f(constants.x), clamp(p - vec3f(constants.x), vec3f(0.0), vec3f(1.0)), hsv.y);
}

// Ondas Paramétricas (model 0): PI/80 per p5 frame at a 30 FPS baseline.
fn sampleParametricWaves(id: u32, t: f32) -> vec2f {
  let i = 9999.0 - f32(id);
  let y = i / 235.0;
  let k = (4.0 + cos(i / 9.0 - t * 2.0)) * cos(i / 35.0);
  let e = y / 7.0 - 13.0;
  let d = length(vec2f(k, e)) + sin(e / 9.0 + t / 2.0) - 4.0;
  let q = 2.0 * sin(k * 3.0)
        - y / 35.0 * k * (9.0 + k * sin(cos(e) * 9.0 - d * 2.0 + t));
  let c = d - t;
  return vec2f(q + 40.0 * cos(c) + 200.0,
               q * sin(c) + d * 35.0);
}

/// Samples a particle; `style` selects the model (model.x), exactly like the
/// Swift particleSample switch. Returns canvas-space pixel + sprite factors.
fn particleSample(id: u32, u: Uniforms) -> ParticleSample {
  var pixel: vec2f;
  var canvasSize: f32 = 400.0;
  var pointScale: f32 = 1.0;
  var trailAlpha: f32 = 1.0;
  let style = u.model.x;

  if (style < 0.5) {
    let t = u.time * 1.17809725;
    pixel = sampleParametricWaves(id, t);
  } else {
    // Models 1..7 land in phase M2; park off-screen until then.
    pixel = vec2f(-1.0e6, -1.0e6);
    trailAlpha = 0.0;
  }

  // Shared transform: canvas pixels -> aspect-correct clip space.
  let center = canvasSize * 0.5;
  var p = vec2f((pixel.x - center) / center,
                (center - pixel.y) / center);
  let cz = cos(u.rotation.z);
  let sz = sin(u.rotation.z);
  p = vec2f(p.x * cz - p.y * sz, p.x * sz + p.y * cz);
  p *= u.scale * exp(u.position.z * 0.08);
  // X/Y rotations become a subtle 2D perspective compression.
  p *= vec2f(cos(u.rotation.y), cos(u.rotation.x));
  p += u.position.xy * 0.25;
  p *= max(vec2f(0.05), u.screenFit.yz);
  if (u.screenFit.x < 0.5) {
    p.x /= max(0.1, u.aspect);
  }

  var out: ParticleSample;
  out.clip = p;
  out.pointSizePx = max(1.0, u.pointSize * pointScale);
  let brightness = max(0.05, u.appearance.w);
  out.color = vec4f(u.appearance.rgb * brightness,
                    clamp(0.38 * brightness, 0.08, 1.0) * trailAlpha);
  return out;
}

struct VertexOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
  @location(1) color: vec4f,
}

@vertex
fn vsMain(@builtin(vertex_index) vi: u32) -> VertexOut {
  let corner = vi % 6u;
  let particleID = vi / 6u;
  let s = particleSample(particleID, u);

  // Quad corners as fractions of the sprite, expanded in physical pixels.
  var offsets = array<vec2f, 6>(
    vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0),
    vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0)
  );
  let f = vec2f(offsets[corner]);
  let ndc = (f - vec2f(0.5)) * 2.0 * s.pointSizePx / u.viewport;

  var out: VertexOut;
  out.position = vec4f(s.clip + ndc, 0.0, 1.0);
  out.uv = f;
  out.color = s.color;
  return out;
}

@fragment
fn fsMain(in: VertexOut) -> @location(0) vec4f {
  let centered = in.uv * 2.0 - vec2f(1.0);
  let alpha = smoothstep(1.0, 0.15, length(centered)) * in.color.a;
  return vec4f(in.color.rgb, alpha);
}
