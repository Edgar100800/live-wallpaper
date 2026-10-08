// ascii-video-v1: draws precomputed ASCII cells (see ASCII_VIDEO_NATIVE.md).
//
// Port of the WebGL2 fragment shader in
// Sources/ParticleWall/Resources/SpiderManASCIIWallpaper/index.html, which
// remains the HTML fallback. One full-screen triangle; every pixel looks up
// its cell (glyph id + RGB332 color) and the glyph's atlas texel. The grid is
// stretched over the whole output, like the WebGL canvas.

struct Params {
  // Omarchy palette (rgb); only read when flags has SYSTEM_PALETTE.
  background: vec4f,
  ink: vec4f,
  highlight: vec4f,
  // Cell columns, rows.
  grid: vec2f,
  // Output size in physical pixels.
  viewport: vec2f,
  // 0..1 background-cleanup strength.
  separation: f32,
  // bit 0: system palette, bit 1: clean background.
  flags: u32,
  pad: vec2f,
}

const SYSTEM_PALETTE: u32 = 1u;
const CLEAN_BACKGROUND: u32 = 2u;
const LUMA: vec3f = vec3f(0.2127, 0.7152, 0.0722);

@group(0) @binding(0) var<uniform> p: Params;
// Rg8Uint: r = glyph id (0..9 fill, 10..13 edge), g = RGB332 color.
@group(0) @binding(1) var cells: texture_2d<u32>;
// R8Unorm atlases in image row order: fill 80x8, edges 40x8.
@group(0) @binding(2) var fillAtlas: texture_2d<f32>;
@group(0) @binding(3) var edgeAtlas: texture_2d<f32>;

@vertex
fn vsAscii(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
  var corners = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  return vec4f(corners[vi], 0.0, 1.0);
}

fn unpackColor(packed: u32) -> vec3f {
  return vec3f(f32((packed >> 5u) & 7u) / 7.0,
               f32((packed >> 2u) & 7u) / 7.0,
               f32(packed & 3u) / 3.0);
}

fn lumaAt(coord: vec2i) -> f32 {
  let clamped = clamp(coord, vec2i(0), vec2i(p.grid) - 1);
  return dot(unpackColor(textureLoad(cells, clamped, 0).g), LUMA);
}

// WebGL samples the atlas with NEAREST at (texel + local * 8 + 0.5) / size
// after UNPACK_FLIP_Y, with CLAMP_TO_EDGE. Reproduce that texel choice
// exactly, including the half-texel offset.
fn glyphMask(glyph: u32, local: vec2f) -> f32 {
  let t = clamp(i32(floor(local.y * 8.0 + 0.5)), 0, 7);
  let row = 7 - t;
  if (glyph < 10u) {
    let x = clamp(i32(floor(f32(glyph) * 8.0 + local.x * 8.0 + 0.5)), 0, 79);
    return textureLoad(fillAtlas, vec2i(x, row), 0).r;
  }
  let x = clamp(i32(floor(f32(glyph - 9u) * 8.0 + local.x * 8.0 + 0.5)), 0, 39);
  return textureLoad(edgeAtlas, vec2i(x, row), 0).r;
}

@fragment
fn fsAscii(@builtin(position) fragCoord: vec4f) -> @location(0) vec4f {
  // Top-left origin, pixel centers: the WebGL (uv.x, 1 - uv.y).
  let sourceUV = fragCoord.xy / p.viewport;
  let cellPosition = sourceUV * p.grid;
  let cellCoord = vec2i(floor(cellPosition));
  let local = fract(cellPosition);
  let cell = textureLoad(cells, cellCoord, 0).rg;
  let mask = glyphMask(cell.r, local);
  let rgb = unpackColor(cell.g);
  let luminance = dot(rgb, LUMA);

  let themed = (p.flags & SYSTEM_PALETTE) != 0u;
  let clean = (p.flags & CLEAN_BACKGROUND) != 0u;
  var visibility = 1.0;
  if (clean) {
    // A clip-specific approximation: bright sky is farther than the subject.
    let contrast = max(max(abs(luminance - lumaAt(cellCoord + vec2i(1, 0))),
                           abs(luminance - lumaAt(cellCoord - vec2i(1, 0)))),
                       max(abs(luminance - lumaAt(cellCoord + vec2i(0, 1))),
                           abs(luminance - lumaAt(cellCoord - vec2i(0, 1)))));
    let cutoff = mix(0.78, 0.38, p.separation);
    let foreground = 1.0 - smoothstep(cutoff - 0.30, cutoff, luminance);
    visibility = foreground * mix(0.30, 1.0, smoothstep(0.02, 0.12, contrast));
  }
  if (!themed && !clean) {
    // WebGL discards faint texels, leaving the black clear color, and writes
    // rgb unblended otherwise.
    return select(vec4f(rgb, 1.0), vec4f(0.0, 0.0, 0.0, 1.0), mask < 0.05);
  }
  let ink = select(rgb, mix(p.ink.rgb, p.highlight.rgb, smoothstep(0.08, 0.42, luminance)), themed);
  let background = select(vec3f(0.0), p.background.rgb, themed);
  return vec4f(mix(background, ink, mask * visibility), 1.0);
}
