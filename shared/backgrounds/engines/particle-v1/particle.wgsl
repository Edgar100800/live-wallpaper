// particle-v1: shared particle engine (WGSL source of truth).
//
// Mirrors MetalParticleRenderer in Sources/ParticleWall/WallpaperRenderer.swift
// (all eight models, the flow-state compute kernel and the graph overlay).
// Point sprites are expanded to instanced quads (WebGPU has no point_size):
// 6 vertices per particle, vertex_index/6 = particle id.
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
  } else {
    // Toro de Esferas (model 8): t wraps every unit in the source dweet.
    let t = (u.time * 1.2) % 1.0;
    pixel = sampleSphereTorus(id, t, &pointScale);
    canvasSize = 600.0;
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
  out.color = vec4f(renderColor * brightness,
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
