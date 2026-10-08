// particle-v1: shared particle engine (WGSL source of truth).
//
// Mirrors MetalParticleRenderer in Sources/ParticleWall/WallpaperRenderer.swift
// (all ten models, the flow-state compute kernel and the graph overlay).
// Point sprites are expanded to quads (WebGPU has no point_size). vsMain
// emits 6 vertices per particle (vertex_index/6 = particle id); vsIndexed
// emits 4 unique vertices per particle for indexed draws (vertex_index/4).
// Uniforms layout must stay 16-byte aligned and identical to the Swift
// Uniforms struct; viewport is the engine-v1 addition for quad expansion.
//
// Binding groups:
//   0: u (uniform), flowParticles ro, flowHistory ro, graphEdges ro
//      (render + graph position compute)
//   1: flowParticles rw, flowHistory rw, flow params uniform (flowUpdate)
//   2: u (uniform), graphPositions rw, graphEdges rw (graph compute)
// Buffers are shared; read-only and read_write views live in different
// groups because WGSL access modes are module-level declarations.

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
@group(0) @binding(1) var<storage, read> flowParticles: array<vec4f>;
@group(0) @binding(2) var<storage, read> flowHistory: array<vec4f>;
@group(0) @binding(3) var<storage, read> graphEdgesRo: array<vec4f>;

@group(1) @binding(0) var<storage, read_write> flowParticlesMut: array<vec4f>;
@group(1) @binding(1) var<storage, read_write> flowHistoryMut: array<vec4f>;
@group(1) @binding(2) var<uniform> flow: vec4u;

@group(2) @binding(0) var<uniform> gu: Uniforms;
@group(2) @binding(1) var<storage, read_write> graphPositions: array<vec4f>;
@group(2) @binding(2) var<storage, read_write> graphEdgesMut: array<vec4f>;

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

fn flowHash(p0: vec3f) -> f32 {
  var p = fract(p0 * 0.1031);
  p += vec3f(dot(p, p.yzx + 33.33));
  return fract((p.x + p.y) * p.z);
}

fn flowNoise(p: vec3f) -> f32 {
  let cell = floor(p);
  var fraction = fract(p);
  fraction = fraction * fraction * (3.0 - 2.0 * fraction);
  let x00 = mix(flowHash(cell + vec3f(0.0, 0.0, 0.0)),
                flowHash(cell + vec3f(1.0, 0.0, 0.0)), fraction.x);
  let x10 = mix(flowHash(cell + vec3f(0.0, 1.0, 0.0)),
                flowHash(cell + vec3f(1.0, 1.0, 0.0)), fraction.x);
  let x01 = mix(flowHash(cell + vec3f(0.0, 0.0, 1.0)),
                flowHash(cell + vec3f(1.0, 0.0, 1.0)), fraction.x);
  let x11 = mix(flowHash(cell + vec3f(0.0, 1.0, 1.0)),
                flowHash(cell + vec3f(1.0, 1.0, 1.0)), fraction.x);
  return mix(mix(x00, x10, fraction.y),
             mix(x01, x11, fraction.y), fraction.z);
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

// Vórtice Gemelo (model 1): direct translation of the supplied 30k-point dweet.
fn sampleTwinVortex(id: u32, t: f32) -> vec2f {
  let i = 29999.0 - f32(id);
  let m = i % 2.0 * 3.0;
  let k = 14.0 * cos(i / 39.0);
  let e = i / 1200.0 - 13.0;
  let d = dot(vec2f(k, e), vec2f(k, e)) / 59.0 + 1.0;
  let q = 89.0 - sin(k) * d
        + k * (8.0 / d + sin(d * 3.0 + e / 9.0 - t));
  let c = d * 0.45 - sin(t - d) / 8.0 - t / 8.0 + m;
  return vec2f(q * sin(c) + 200.0,
               (q + 40.0 + 30.0 * sin(c * 2.0 + m)) * cos(c) + 200.0);
}

// Flor Orbital (model 2): the bitwise JS term becomes 80 or 160 by parity.
fn sampleOrbitalBloom(id: u32, t: f32) -> vec2f {
  let i = 29999.0 - f32(id);
  let y = i / 799.0;
  let k = 5.0 * cos(i / 48.0);
  let e = 5.0 * cos(y / 9.0);
  let divisor = 6.0 + i % 4.0;
  let d = pow(length(vec2f(k, e)) / divisor, 4.0) + 4.0;
  let parityOffset = 80.0 * (1.0 + i % 2.0);
  let q = k * (3.0 + e / 2.0 * sin(d * 8.0 + k / 9.0 - t))
        - 3.0 * sin(k * d / 3.0) + parityOffset;
  let c = d - t / 9.0 + i % 5.0;
  return vec2f(q * sin(c) + 200.0,
               q * cos(c - i % 2.0 + i % 5.0 * 3.0 + 7.0) + 200.0);
}

// Roseta Hexagonal (model 3): generate the six feedback rotations directly
// instead of copying the previous framebuffer six times.
fn sampleHexagonalRosette(id: u32, t: f32) -> vec2f {
  let sidx = id / 6u;
  let rot = id % 6u;
  let i = 19999.0 - f32(sidx);
  let k = i % 25.0 - 12.0;
  let e = i / 800.0;
  let d = 7.0 * cos(length(vec2f(k, e)) / 3.0 + t / 2.0);
  var centered = vec2f(k * 4.0 + d * k * sin(d + e / 9.0 + t),
                       e * 2.0 - d * 9.0 - d * 9.0 * cos(d + t));
  let angle = f32(rot) * 1.04719755;
  let ca = cos(angle);
  let sa = sin(angle);
  centered = vec2f(centered.x * ca - centered.y * sa,
                   centered.x * sa + centered.y * ca);
  return centered + 200.0;
}

// Órbita Toroidal (model 6): 64 toruses represented by a balanced 36,864
// point cloud, plus a 512-point light sphere.
fn sampleTorusOrbit(id: u32, t: f32, pointScale: ptr<function, f32>) -> vec2f {  let f = t;
  var world: vec3f;
  if (id < 512u) {
    let sidx = f32(id) + 0.5;
    let sphereY = 1.0 - 2.0 * sidx / 512.0;
    let sphereRadius = sqrt(max(0.0, 1.0 - sphereY * sphereY));
    let sphereAngle = sidx * 2.39996323;
    let unitSphere = vec3f(cos(sphereAngle) * sphereRadius,
                           sphereY,
                           sin(sphereAngle) * sphereRadius);
    world = vec3f(60.0 * sin(f) + 99.0 * sin(-f),
                  -20.0,
                  170.0 + 99.0 * cos(f)) + unitSphere * 4.0;
    *pointScale = 0.75;
  } else {
    let localID = id - 512u;
    let torusID = localID / (96u * 6u);
    let torusPoint = localID % (96u * 6u);
    let majorID = torusPoint / 6u;
    let tubeID = torusPoint % 6u;
    let major = f32(majorID) * 6.28318531 / 96.0;
    let tube = f32(tubeID) * 6.28318531 / 6.0;
    let radius = 80.0 + 4.0 * cos(tube);
    var local = vec3f(radius * cos(major),
                      radius * sin(major),
                      4.0 * sin(tube));

    let cx = cos(0.8);
    let sx = sin(0.8);
    local = vec3f(local.x,
                  local.y * cx - local.z * sx,
                  local.y * sx + local.z * cx);
    local.x += 120.0;

    let ringAngle = f32(torusID) * 3.14159265 / 32.0 + f;
    let cy = cos(ringAngle);
    let sy = sin(ringAngle);
    world = vec3f(local.x * cy + local.z * sy,
                  local.y,
                  -local.x * sy + local.z * cy);
    world += vec3f(60.0 * sin(f), -20.0, 170.0);
    *pointScale = 0.42;
  }

  let eyeZ = 346.41016;
  let perspective = eyeZ / max(72.0, eyeZ - world.z);
  return vec2f(200.0 + world.x * perspective,
               200.0 + world.y * perspective);
}

// Toro de Esferas (model 8): ring of spheres whose radius breathes with
// cos(v); t wraps every unit like the source dweet (t=(t+.02)%1, 60 FPS
// baseline -> t = time * 1.2).
fn sampleSphereTorus(id: u32, t: f32, pointScale: ptr<function, f32>) -> vec2f {
  let p = 0.078539816; // PI/40
  let y = id / 80u;
  let x = id % 80u;
  let v = (f32(y) + t) * p * 2.0;
  let u = (f32(x) + t) * p;
  let r = 90.0;
  let ring = 2.0 + sin(v);
  var local = vec3f(ring * cos(u) * r,
                    ring * sin(u) * r,
                    cos(v) * r);

  // p5 applies rotateX(.5) then rotateY(-.5); the model matrix is
  // Rx * Ry, so rotateY lands on the point first.
  let cy = cos(-0.5);
  let sy = sin(-0.5);
  local = vec3f(local.x * cy + local.z * sy,
                local.y,
                -local.x * sy + local.z * cy);
  let cx = cos(0.5);
  let sx = sin(0.5);
  local = vec3f(local.x,
                local.y * cx - local.z * sx,
                local.y * sx + local.z * cx);

  // p5 default WEBGL camera for a 600px canvas: 300 / tan(PI/6).
  let eyeZ = 519.61524;
  let perspective = eyeZ / max(72.0, eyeZ - local.z);
  // Sphere radius cos(v)+0.3 drives the sprite size.
  *pointScale = max(0.1, cos(v) + 0.3);
  return vec2f(300.0 + local.x * perspective,
               300.0 + local.y * perspective);
}

// Medusa de Puntos (model 9): dotted jellyfish drawn by a 10k-point spiral
// (one radian between consecutive points). The JS `y^8` bitwise term is an
// XOR on the int32 truncation of y.
fn sampleJellyfish(id: u32, t: f32) -> vec2f {
  let i = 9999.0 - f32(id);
  let y = i / 345.0;
  let base = select(y / 5.0 + cos(y / 2.0),
                    6.0 + sin(f32(i32(y) ^ 8)) * 6.0,
                    y < 11.0);
  let k = base * cos(i - t / 4.0);
  let e = y / 7.0 - 13.0;
  let d = length(vec2f(k, e)) + sin(e / 4.0 + t) / 2.0;
  let c = d / 2.0 + 1.0 - t / 2.0;
  let q = y * k / d * (3.0 + sin(d * 2.0 + y / 2.0 - t * 4.0));
  return vec2f(q + 60.0 * cos(c) + 200.0,
               q * sin(c) + d * 29.0 - 170.0);
}

// Nebulosa (model 10): nine soft clouds swept by two noise fields, drawn
// as 512 angles x 200 radii per layer. p5's Perlin noise is represented
// by the shared flowNoise value noise (same stand-in as Lluvia de Ruido
// and Anillos Cromaticos); the source stroke alpha (W-i*W)/22 ramps up
// while the radius shrinks, so the cloud cores glow and the rims fade.
// Variation over the source dweet: the nine clouds occupy a jittered 3x3
// layout across the canvas. Each base has a deterministic depth and gently
// breathes along Z, preserving p5 WEBGL perspective (eyeZ = 300/tan(PI/6))
// without an orbit that can temporarily cluster the formation.
fn sampleNebula(id: u32, f: f32, horizontalSpan: f32,
                trail: ptr<function, f32>,
                scale: ptr<function, f32>) -> vec2f {
  let layer = id / (512u * 200u);
  let j = (id / 200u) % 512u;
  let k = id % 200u;
  let n = 8.0 - f32(layer);
  let a = -3.14159265 + f32(j) * 0.0122718463; // PI/256
  let i = 1.0 - f32(k) * 0.005;
  let eyeZ = 519.61524;
  let column = f32(layer % 3u);
  let row = f32(layer / 3u);
  let jitter = vec2f((flowHash(vec3f(n, 1.9, 6.3)) - 0.5) * 36.0,
                     (flowHash(vec3f(n, 6.3, 1.9)) - 0.5) * 36.0);
  let gridTarget = vec2f(200.0 + ((column - 1.0) * 140.0 + jitter.x)
                             * horizontalSpan,
                         45.0 + row * 120.0 + jitter.y);
  let basePerspective = 0.85 + flowHash(vec3f(n, 7.7, 3.1)) * 0.5;
  let phase = flowHash(vec3f(n, 9.1, 5.5)) * 6.28318531;
  let world = vec3f((gridTarget.x - 200.0) / basePerspective,
                    (gridTarget.y - 200.0) / basePerspective,
                    eyeZ - eyeZ / basePerspective
                        + 20.0 * sin(f * 0.4 + phase));
  let perspective = eyeZ / max(72.0, eyeZ - world.z);
  *scale = perspective;
  *trail = (1.0 - i) * (400.0 / 22.0 / 255.0);
  return vec2f(200.0 + world.x * perspective,
               200.0 + world.y * perspective)
      + vec2f(flowNoise(vec3f(i - f, f / 3.0 + n, a)) * i * 400.0,
              flowNoise(vec3f(f / 2.0 + n, i - f, a)) * i * 400.0);
}

// Tesseract Cuántico (model 11): preserves the full mathematical model from
// the user's THREE.js ParticlesSwarm (350 stippled wireframe blocks on a
// (2,5) torus knot, four staggered tracks, bundle twist, flow and a 5%
// stardust shell), but presents it like the other ParticleWall models:
// monochrome user tint, fixed-size points and p5 WEBGL perspective on a
// 400px canvas. Wire/face/corner/pulse/stardust hierarchy moves to alpha;
// there is no THREE fog, bloom, camera-near clipping or perspective sprite
// scaling. Time stays unbounded because the flow terms share no period.
fn sampleTesseract(id: u32, time: f32,
                   trail: ptr<function, f32>,
                   scale: ptr<function, f32>) -> vec2f {
  let macroRadius = 50.0;
  let microRadius = 15.0;
  let pLoops = 2.0;
  let qTwists = 5.0;
  let blockCount = 350.0;
  let blockLength = 7.0;
  let blockSize = 2.5;
  let stagger = 4.0;
  let bundleTwist = 1.5;
  let flow = 0.3;
  let stardustCount = 1000.0;  // 5% of 20000
  let remainingCount = 19000.0;

  var world: vec3f;
  var pointAlpha = 1.0;

  if (f32(id) < stardustCount) {
    let fi = f32(id);
    let sd1 = fract(sin(fi * 11.11) * 43758.54);
    let sd2 = fract(cos(fi * 22.22) * 43758.54);
    let sd3 = fract(sin(fi * 33.33) * 43758.54);

    let radiusDist = macroRadius * 1.2 + sd1 * 80.0;
    let theta = sd2 * 6.28318531;
    let phi = acos(sd3 * 2.0 - 1.0);

    let sx = radiusDist * sin(phi) * cos(theta)
        + sin(time * 0.2 + fi * 0.1) * 10.0;
    let sy = radiusDist * sin(phi) * sin(theta)
        + cos(time * 0.25 + fi * 0.1) * 10.0;
    let sz = radiusDist * cos(phi) + sin(time * 0.15 + fi * 0.2) * 10.0;
    world = vec3f(sx, sy, sz);

    let twinkle = pow((sin(time * 3.0 + fi) + 1.0) * 0.5, 8.0);
    pointAlpha = 0.15 + twinkle * 0.85;
  } else {
    let iRem = f32(id) - stardustCount;
    let ppb = remainingCount / blockCount;
    let blockId = floor(iRem / ppb);
    let localId = iRem - blockId * ppb;

    let numWire = ppb * 0.85;
    let isWire = localId < numWire;

    let tBase = blockId / blockCount * 6.28318531;
    let t = tBase + time * flow * 0.1;

    let cosQt = cos(qTwists * t);
    let sinQt = sin(qTwists * t);
    let cosPt = cos(pLoops * t);
    let sinPt = sin(pLoops * t);

    let ringRadius = macroRadius + microRadius * cosQt;
    let center = vec3f(ringRadius * cosPt,
                       ringRadius * sinPt,
                       microRadius * sinQt);

    let tangentRaw = vec3f(
        -pLoops * ringRadius * sinPt - qTwists * microRadius * sinQt * cosPt,
         pLoops * ringRadius * cosPt - qTwists * microRadius * sinQt * sinPt,
         qTwists * microRadius * cosQt);
    let tangent = tangentRaw / (length(tangentRaw) + 0.0001);

    let torusNormal = vec3f(cosPt, sinPt, 0.0);
    let binormalRaw = cross(tangent, torusNormal);
    let binormal = binormalRaw / (length(binormalRaw) + 0.0001);
    let normal = cross(binormal, tangent);

    let twistAngle = tBase * bundleTwist + time * flow * 0.5;
    let cosTw = cos(twistAngle);
    let sinTw = sin(twistAngle);
    let fnv = normal * cosTw - binormal * sinTw;
    let fb = normal * sinTw + binormal * cosTw;

    let track = blockId % 4.0;
    let c1 = select(-1.0, 1.0, track % 2.0 < 0.5);
    let c2 = select(-1.0, 1.0, track < 2.0);
    let blockCenter = center + fnv * (c1 * stagger) + fb * (c2 * stagger);

    var local = vec3f(0.0);
    var u = 0.0;
    if (isWire) {
      let edgePosRaw = localId / numWire * 12.0;
      let edgeId = min(11.0, floor(edgePosRaw));
      u = (edgePosRaw - edgeId) * 2.0 - 1.0;

      let axis = edgeId % 3.0;
      let corner = floor(edgeId / 3.0);
      let e1 = select(-1.0, 1.0, corner % 2.0 > 0.5);
      let e2 = select(-1.0, 1.0, corner > 1.5);

      if (axis < 0.5) { local = vec3f(u, e1, e2); }
      else if (axis < 1.5) { local = vec3f(e1, u, e2); }
      else { local = vec3f(e1, e2, u); }
    } else {
      let s1 = fract(sin(localId * 12.989 + blockId * 78.233) * 43758.545);
      let s2 = fract(cos(localId * 39.346 + blockId * 53.211) * 43758.545);
      let s3 = fract(sin(localId * 73.156 + blockId * 12.742) * 43758.545);
      let s4 = fract(cos(localId * 23.456 + blockId * 89.123) * 43758.545);

      let faceAxis = min(2.0, floor(s1 * 3.0));
      let signFace = select(-1.0, 1.0, s2 > 0.5);
      let u2 = s3 * 2.0 - 1.0;
      let v2 = s4 * 2.0 - 1.0;

      if (faceAxis < 0.5) { local = vec3f(signFace, u2, v2); }
      else if (faceAxis < 1.5) { local = vec3f(u2, signFace, v2); }
      else { local = vec3f(u2, v2, signFace); }
    }

    let stretched = vec3f(local.x * blockLength,
                          local.y * blockSize,
                          local.z * blockSize);
    world = blockCenter + stretched.x * tangent
        + stretched.y * fnv + stretched.z * fb;

    pointAlpha = select(0.4, 1.0, isWire);

    if (isWire) {
      let pulseEnv = sin(tBase * pLoops * 12.0 - time * 5.0);
      if (pulseEnv > 0.8) {
        pointAlpha = pointAlpha + (pulseEnv - 0.8) * 1.5;
      }
      let isCorner = select(0.0, 1.0, abs(u) > 0.90);
      pointAlpha = pointAlpha + isCorner * 0.4;
    }
    pointAlpha = clamp(pointAlpha, 0.0, 1.0);
  }

  // Global spin: rotateX(time*0.11) then rotateY(time*0.17).
  let cgx = cos(time * 0.11);
  let sgx = sin(time * 0.11);
  let cgy = cos(time * 0.17);
  let sgy = sin(time * 0.17);
  let y1 = world.y * cgx - world.z * sgx;
  let z1 = world.y * sgx + world.z * cgx;
  let x2 = world.x * cgy + z1 * sgy;
  let z2 = -world.x * sgy + z1 * cgy;

  // Same p5 WEBGL camera contract as Órbita Toroidal: the mathematical
  // model is scaled into a 400px canvas and points keep one uniform size.
  let modelScale = 2.2;
  let scaled = vec3f(x2, y1, z2) * modelScale;
  let eyeZ = 346.41016; // 200 / tan(PI/6)
  let perspective = eyeZ / max(72.0, eyeZ - scaled.z);
  *trail = pointAlpha;
  *scale = 1.0;
  return vec2f(200.0 + scaled.x * perspective,
               200.0 + scaled.y * perspective);
}

/// Samples a particle; `style` selects the model (model.x), exactly like the
/// Swift particleSample switch. Returns canvas-space pixel + sprite factors.
fn particleSample(id: u32, u: Uniforms) -> ParticleSample {
  var pixel: vec2f;
  var canvasSize: f32 = 400.0;
  var pointScale: f32 = 1.0;
  var trailAlpha: f32 = 1.0;
  var renderColor: vec3f = u.appearance.rgb;
  let style = u.model.x;

  if (style < 0.5) {
    // Ondas Paramétricas: PI/80 per p5 frame at a 30 FPS baseline.
    let t = u.time * 1.17809725;
    pixel = sampleParametricWaves(id, t);
  } else if (style < 1.5) {
    // Vórtice Gemelo: 2PI/3 per p5 frame at a 30 FPS baseline.
    let t = u.time * 2.09439510;
    pixel = sampleTwinVortex(id, t);
  } else if (style < 2.5) {
    // Flor Orbital: PI/2 per p5 frame at a 30 FPS baseline.
    let t = u.time * 1.57079633;
    pixel = sampleOrbitalBloom(id, t);
  } else if (style < 3.5) {
    // Roseta Hexagonal: PI/8 per p5 frame at a 30 FPS baseline.
    let t = u.time * 0.39269908;
    pixel = sampleHexagonalRosette(id, t);
  } else if (style < 4.5) {
    // Lluvia de Ruido: positions come from the persistent compute
    // buffer. Twelve history slots approximate p5's BLUR feedback
    // without copying and filtering the entire framebuffer.
    let historyCount = 12u;
    let particleID = id / historyCount;
    let trailID = id % historyCount;
    let latestSlot = u32(u.model.y + 0.5);
    let slot = (latestSlot + historyCount - trailID) % historyCount;
    let s = flowHistory[particleID * historyCount + slot];
    pixel = s.xy;
    canvasSize = 720.0;
    pointScale = max(0.16, s.z / 3.0);
    trailAlpha = s.w * exp(-f32(trailID) * 0.2);
  } else if (style < 5.5) {
    // Espiral Prima: upload only the 78,498 primes represented by the
    // million-entry Processing sieve, not the composite placeholders.
    let prime = flowParticles[id].x;
    let t = 1.0 + u.time * 0.000003;
    pixel = vec2f(prime * sin(prime * t) / 99.0 + 400.0,
                  prime * cos(prime * t) / 99.0 + 400.0);
    canvasSize = 800.0;
    pointScale = 0.38;
  } else if (style < 6.5) {
    // Órbita Toroidal: 0.3 rad/s on the Swift animation time.
    pixel = sampleTorusOrbit(id, u.time * 0.3, &pointScale);
  } else if (style < 7.5) {
    // Anillos Cromáticos: the source's eleven HSB rings contain 6,225
    // points. Eight nearby time samples reproduce the soft BLUR trail
    // without filtering or retaining a full 720x720 framebuffer.
    let trailCount = 8u;
    let baseID = id / trailCount;
    let trailID = id % trailCount;
    var localID = baseID;
    var d = 330.0;
    var ringPointCount = 0u;
    loop {
      ringPointCount = u32(ceil(3.14159265 * d));
      if (localID < ringPointCount) { break; }
      localID -= ringPointCount;
      d -= 30.0;
    }

    let t = max(0.0, u.time * 30.0 - f32(trailID) * 0.85);
    let r = f32(localID) * 2.0 / d;
    let tangent = tan(d / 199.0 - t / 99.0);
    let energy = min(tangent * tangent, 64.0);
    let noiseValue = flowNoise(vec3f(r * 99.0, d, 0.0));
    let radius = d
        + sin(r * 9.0 + t / 9.0 * d / 720.0)
        * d * 0.25 * noiseValue * energy;
    let angle = r - 1.57079633;
    pixel = vec2f(cos(angle) * radius + 360.0,
                  sin(angle) * radius + 360.0);
    canvasSize = 720.0;
    pointScale = 0.9;

    // p5's default HSB range is 0...255. The tint control multiplies
    // the generated hue, so white keeps the original palette.
    let hue = clamp(d, 0.0, 255.0) / 255.0;
    let hsbColor = hsvToRGB(vec3f(hue, 50.0 / 255.0, 1.0));
    renderColor = hsbColor * u.appearance.rgb;
    let sourceAlpha = clamp(0.7 / max(energy, 0.025), 0.018, 0.82);
    trailAlpha = sourceAlpha * exp(-f32(trailID) * 0.32);
  } else if (style < 8.5) {
    // Toro de Esferas (model 8): t wraps every unit in the source dweet.
    let t = (u.time * 1.2) % 1.0;
    pixel = sampleSphereTorus(id, t, &pointScale);
    canvasSize = 600.0;
  } else if (style < 9.5) {
    // Medusa de Puntos (model 9): PI/120 per p5 frame at a 60 FPS baseline
    // (speed PI/2). Wrapping at 8*PI returns every phase term (t, t/2,
    // t/4, 4t) to its start: seamless loop with bounded arguments for the
    // high-frequency cos(i - t/4).
    let t = (u.time * 1.57079633) % 25.13274123;
    pixel = sampleJellyfish(id, t);
  } else if (style < 10.5) {
    // Nebulosa (model 10): 0.005 per p5 frame at a 60 FPS baseline. The
    // noise fields are not periodic, so f stays unbounded (same precedent
    // as the prime and ring models).
    let horizontalSpan = select(u.aspect, 1.0, u.screenFit.x >= 0.5);
    pixel = sampleNebula(id, u.time * 0.3, horizontalSpan,
                         &trailAlpha, &pointScale);
  } else {
    // Tesseract Cuántico (model 11): the flow terms share no period, so
    // time stays unbounded (same precedent as the prime, ring and nebula
    // models). Geometry uses the shared monochrome point presentation.
    pixel = sampleTesseract(id, u.time, &trailAlpha, &pointScale);
    canvasSize = 400.0;
  }

  // Convert the source canvas into aspect-correct clip space.
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
  // Asymptotic alpha: linear 0.38*b saturates at ~2.6 (nothing changes
  // beyond that); the exponential keeps a visible ramp across the whole
  // control range (b=6 -> 0.96, b=10 -> 0.996) with the same look at the
  // 1.5 default.
  out.color = vec4f(renderColor * brightness,
                    (1.0 - exp(-0.55 * brightness)) * trailAlpha);
  return out;
}

struct VertexOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
  @location(1) color: vec4f,
}

// Expands one quad corner (`f` in 0..1 sprite fractions) in physical pixels.
fn quadCorner(clip: vec2f, pointSizePx: f32, color: vec4f, f: vec2f) -> VertexOut {
  let ndc = (f - vec2f(0.5)) * 2.0 * pointSizePx / u.viewport;

  var out: VertexOut;
  out.position = vec4f(clip + ndc, 0.0, 1.0);
  out.uv = f;
  out.color = color;
  return out;
}

// Corner order of the 4 unique quad vertices: (0,0) (1,0) (0,1) (1,1).
fn cornerOffset(vi: u32) -> vec2f {
  return vec2f(f32(vi & 1u), f32(vi >> 1u));
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
  return quadCorner(s.clip, s.pointSizePx, s.color, offsets[corner]);
}

// Indexed quads: 4 unique vertices per particle (vertex_index / 4 = particle
// id) drawn with the index pattern 0 1 2 2 1 3, so the post-transform cache
// shares the diagonal and particleSample runs 4 times instead of 6.
@vertex
fn vsIndexed(@builtin(vertex_index) vi: u32) -> VertexOut {
  let s = particleSample(vi >> 2u, u);
  return quadCorner(s.clip, s.pointSizePx, s.color, cornerOffset(vi & 3u));
}

@fragment
fn fsMain(in: VertexOut) -> @location(0) vec4f {
  let centered = in.uv * 2.0 - vec2f(1.0);
  let alpha = smoothstep(1.0, 0.15, length(centered)) * in.color.a;
  return vec4f(in.color.rgb, alpha);
}

// ---------------------------------------------------------------------------
// Flow state (Lluvia de Ruido): one fixed step advances every particle.
// flow = (frame, unused, particleCount, historyCount).
// ---------------------------------------------------------------------------

@compute @workgroup_size(64)
fn flowUpdate(@builtin(global_invocation_id) gid: vec3u) {
  let id = gid.x;
  let particleCount = flow.z;
  let historyCount = flow.w;
  if (id >= particleCount) { return; }

  let frame = flow.x;
  let first = (frame * 9u) % particleCount;
  let insertionOffset = (id + particleCount - first) % particleCount;
  let spawned = insertionOffset < 9u;
  var state = flowParticlesMut[id];

  if (spawned) {
    let newTick = frame * 9u + insertionOffset + 1u;
    state = vec4f((f32(newTick) * 99.0) % 720.0, 0.0, 0.0, 3.0);
    for (var slot = 0u; slot < historyCount; slot++) {
      flowHistoryMut[id * historyCount + slot] = vec4f(0.0);
    }
  } else if (state.w <= 0.0) {
    return;
  }

  state.w *= 0.997;
  let globalTick = f32((frame + 1u) * 9u);
  let n = flowNoise(vec3f(state.x / 720.0, state.y / 9.0,
                          globalTick / 720.0));
  if (n > 0.4) {
    state.z += 0.5;
    state.y += state.z;
  } else {
    state.x += select(-1.0, 1.0, (n % 0.1) > 0.05);
    state.z = 0.0;
    state.y += 0.5;
  }

  flowParticlesMut[id] = state;
  let historySlot = frame % historyCount;
  flowHistoryMut[id * historyCount + historySlot] =
      vec4f(state.xy, state.w, 1.0);
}

// ---------------------------------------------------------------------------
// Graph overlay: sample 768 node positions from the active model, connect
// nearest neighbours, draw the edge list as lines.
// ---------------------------------------------------------------------------

const GRAPH_NODE_COUNT: u32 = 768u;
const GRAPH_MAX_CONNECTIONS: u32 = 3u;

@compute @workgroup_size(64)
fn graphPositionUpdate(@builtin(global_invocation_id) gid: vec3u) {
  let id = gid.x;
  if (id >= GRAPH_NODE_COUNT) { return; }
  let sourceCount = max(1u, u32(u.model.w + 0.5));
  let sourceID = min(sourceCount - 1u,
                     u32(f32(id) * f32(sourceCount) / f32(GRAPH_NODE_COUNT)));
  let s = particleSample(sourceID, u);
  let visible = all(abs(s.clip) <= vec2f(1.15)) && s.color.a > 0.005;
  graphPositions[id] = vec4f(s.clip,
                             select(0.0, s.color.a, visible),
                             f32(sourceID));
}

@compute @workgroup_size(64)
fn graphConnectionUpdate(@builtin(global_invocation_id) gid: vec3u) {
  let id = gid.x;
  if (id >= GRAPH_NODE_COUNT) { return; }

  let source = graphPositions[id];
  let threshold = max(0.001, gu.graph.y);
  let thresholdSquared = threshold * threshold;
  var bestDistance0 = thresholdSquared;
  var bestDistance1 = thresholdSquared;
  var bestDistance2 = thresholdSquared;
  var bestID0 = 0xffffffffu;
  var bestID1 = 0xffffffffu;
  var bestID2 = 0xffffffffu;

  if (source.z > 0.0) {
    for (var candidate = id + 1u; candidate < GRAPH_NODE_COUNT; candidate++) {
      let tgt = graphPositions[candidate];
      if (tgt.z <= 0.0) { continue; }
      let delta = source.xy - tgt.xy;
      let distanceSquared = dot(delta, delta);
      if (distanceSquared >= bestDistance2) { continue; }
      if (distanceSquared < bestDistance0) {
        bestDistance2 = bestDistance1;
        bestID2 = bestID1;
        bestDistance1 = bestDistance0;
        bestID1 = bestID0;
        bestDistance0 = distanceSquared;
        bestID0 = candidate;
      } else if (distanceSquared < bestDistance1) {
        bestDistance2 = bestDistance1;
        bestID2 = bestID1;
        bestDistance1 = distanceSquared;
        bestID1 = candidate;
      } else {
        bestDistance2 = distanceSquared;
        bestID2 = candidate;
      }
    }
  }

  let requestedConnections = min(GRAPH_MAX_CONNECTIONS, u32(gu.graph.w + 0.5));
  for (var slot = 0u; slot < GRAPH_MAX_CONNECTIONS; slot++) {
    let targetID = select(select(bestID2, bestID1, slot == 1u), bestID0, slot == 0u);
    let distanceSquared = select(select(bestDistance2, bestDistance1, slot == 1u),
                                 bestDistance0, slot == 0u);
    let edge = (id * GRAPH_MAX_CONNECTIONS + slot) * 2u;
    if (slot < requestedConnections && targetID != 0xffffffffu) {
      let tgt = graphPositions[targetID];
      let proximity = 1.0 - sqrt(distanceSquared) / threshold;
      let alpha = clamp(gu.graph.z * proximity * min(source.z, tgt.z),
                        0.0, 1.0);
      graphEdgesMut[edge] = vec4f(source.xy, alpha, 0.0);
      graphEdgesMut[edge + 1u] = vec4f(tgt.xy, alpha, 0.0);
    } else {
      graphEdgesMut[edge] = vec4f(0.0);
      graphEdgesMut[edge + 1u] = vec4f(0.0);
    }
  }
}

struct LineOut {
  @builtin(position) position: vec4f,
  @location(0) color: vec4f,
}

@vertex
fn vsLine(@builtin(vertex_index) vi: u32) -> LineOut {
  let edge = graphEdgesRo[vi];
  var out: LineOut;
  out.position = vec4f(edge.xy, 0.0, 1.0);
  out.color = vec4f(u.appearance.rgb * max(0.05, u.appearance.w), edge.z);
  return out;
}

@fragment
fn fsLine(in: LineOut) -> @location(0) vec4f {
  return in.color;
}
