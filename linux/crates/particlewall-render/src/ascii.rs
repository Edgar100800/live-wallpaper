//! ascii-video-v1 on wgpu: plays precomputed ASCII cells (ASCII_VIDEO_NATIVE.md)
//! without WebKit. Runtime work per new clip frame is one cell read (2 bytes
//! per cell) and one small texture upload; drawing is a single full-screen
//! pass over two glyph atlases.

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::gpu::GpuContext;

pub const ASCII_WGSL: &str =
    include_str!("../../../../shared/backgrounds/engines/ascii-video-v1/ascii.wgsl");
/// AcerolaFX atlases (MIT, see Resources/ASCII/LICENSE-AcerolaFX.txt).
const FILL_ATLAS_PNG: &[u8] =
    include_bytes!("../../../../Sources/ParticleWall/Resources/ASCII/fillASCII.png");
const EDGE_ATLAS_PNG: &[u8] =
    include_bytes!("../../../../Sources/ParticleWall/Resources/ASCII/edgesASCII.png");

const HEADER_SIZE: u64 = 48;
/// Limits shared with the WebGL player and the importer.
const MAX_CELLS: usize = 524_288;
const MAX_CHUNK_FRAMES: u32 = 60;

enum Source {
    /// Bundled `clip.asciivideo`: 48-byte header + consecutive frames.
    Single(File),
    /// Imported clips: `stream.json` + `frames/<n>.bin`, `chunk_frames` each.
    Chunked { dir: PathBuf, chunk_frames: u32, open: Option<(u32, File)> },
}

/// Reads frames of an ascii-video-v1 wallpaper directory on demand; only the
/// current frame is held in memory.
pub struct AsciiClip {
    pub columns: u32,
    pub rows: u32,
    pub frame_count: u32,
    pub fps: f64,
    source: Source,
}

impl AsciiClip {
    /// True when `dir` holds a native clip in either layout.
    pub fn exists_in(dir: &Path) -> bool {
        dir.join("stream.json").is_file() || dir.join("clip.asciivideo").is_file()
    }

    /// Opens `dir`, preferring `stream.json` like the WebGL player, and
    /// validates the metadata against the payload size.
    pub fn open(dir: &Path) -> Result<Self, String> {
        if let Ok(text) = std::fs::read(dir.join("stream.json")) {
            return Self::open_stream(dir, &text);
        }
        Self::open_single(&dir.join("clip.asciivideo"))
    }

    fn open_stream(dir: &Path, text: &[u8]) -> Result<Self, String> {
        let meta: serde_json::Value =
            serde_json::from_slice(text).map_err(|e| format!("stream.json: {e}"))?;
        let int = |key: &str| {
            meta[key]
                .as_u64()
                .filter(|v| *v > 0 && *v <= u64::from(u32::MAX))
                .map(|v| v as u32)
                .ok_or_else(|| format!("stream.json: invalid {key}"))
        };
        let (columns, rows, frame_count, chunk_frames) =
            (int("columns")?, int("rows")?, int("frameCount")?, int("chunkFrames")?);
        let fps = meta["fps"].as_f64().filter(|f| f.is_finite() && *f > 0.0 && *f <= 60.0);
        let Some(fps) = fps else { return Err("stream.json: invalid fps".into()) };
        if columns as usize * rows as usize > MAX_CELLS || chunk_frames > MAX_CHUNK_FRAMES {
            return Err("stream.json: clip too large".into());
        }
        Ok(Self {
            columns,
            rows,
            frame_count,
            fps,
            source: Source::Chunked { dir: dir.to_path_buf(), chunk_frames, open: None },
        })
    }

    fn open_single(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut header = [0u8; HEADER_SIZE as usize];
        file.read_exact_at(&mut header, 0).map_err(|_| "ASCII header truncated".to_string())?;
        let u16_at = |o: usize| u16::from_le_bytes([header[o], header[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes(header[o..o + 4].try_into().unwrap());
        if &header[..8] != b"PWASCII1" || u16_at(8) != 1 || u64::from(u16_at(10)) != HEADER_SIZE {
            return Err("invalid ASCII header".into());
        }
        let (columns, rows, frame_count) = (u32_at(24), u32_at(28), u32_at(32));
        let (numerator, denominator) = (u32_at(36), u32_at(40));
        if columns == 0 || rows == 0 || frame_count == 0 || numerator == 0 || denominator == 0 {
            return Err("invalid ASCII header".into());
        }
        if columns as usize * rows as usize > MAX_CELLS {
            return Err("ASCII clip too large".into());
        }
        let frame_bytes = u64::from(columns) * u64::from(rows) * 2;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if len != HEADER_SIZE + frame_bytes * u64::from(frame_count) {
            return Err("ASCII payload length mismatch".into());
        }
        Ok(Self {
            columns,
            rows,
            frame_count,
            fps: f64::from(numerator) / f64::from(denominator),
            source: Source::Single(file),
        })
    }

    pub fn frame_bytes(&self) -> usize {
        self.columns as usize * self.rows as usize * 2
    }

    /// Reads frame `index` (row-major cells, 2 bytes each) into `out`.
    pub fn read_frame(&mut self, index: u32, out: &mut [u8]) -> Result<(), String> {
        let frame_bytes = self.frame_bytes();
        let out = &mut out[..frame_bytes];
        let frame_count = self.frame_count;
        match &mut self.source {
            Source::Single(file) => file
                .read_exact_at(out, HEADER_SIZE + index as u64 * frame_bytes as u64)
                .map_err(|e| format!("ASCII frame {index}: {e}")),
            Source::Chunked { dir, chunk_frames, open } => {
                let chunk = index / *chunk_frames;
                if open.as_ref().is_none_or(|(n, _)| *n != chunk) {
                    let path = dir.join(format!("frames/{chunk}.bin"));
                    let file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                    let expected = (*chunk_frames).min(frame_count - chunk * *chunk_frames) as u64
                        * frame_bytes as u64;
                    if file.metadata().map_err(|e| e.to_string())?.len() != expected {
                        return Err(format!("incomplete ASCII chunk {chunk}"));
                    }
                    *open = Some((chunk, file));
                }
                let (_, file) = open.as_ref().unwrap();
                file.read_exact_at(out, (index % *chunk_frames) as u64 * frame_bytes as u64)
                    .map_err(|e| format!("ASCII frame {index}: {e}"))
            }
        }
    }
}

/// Omarchy palette colors (0..1 rgb).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AsciiPalette {
    pub background: [f32; 3],
    pub ink: [f32; 3],
    pub highlight: [f32; 3],
}

/// Presentation settings resolved by the host.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AsciiStyle {
    /// Some when the color mode is the system palette and one is available.
    pub palette: Option<AsciiPalette>,
    pub clean_background: bool,
    /// Clamped to 0..1.
    pub separation: f32,
}

impl Default for AsciiStyle {
    fn default() -> Self {
        Self { palette: None, clean_background: false, separation: 0.5 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    background: [f32; 4],
    ink: [f32; 4],
    highlight: [f32; 4],
    grid: [f32; 2],
    viewport: [f32; 2],
    separation: f32,
    flags: u32,
    pad: [f32; 2],
}

impl Params {
    fn new(style: &AsciiStyle, grid: (u32, u32), viewport: (u32, u32)) -> Self {
        let rgba = |c: [f32; 3]| [c[0], c[1], c[2], 1.0];
        let palette = style.palette.unwrap_or(AsciiPalette {
            background: [0.0; 3],
            ink: [0.0; 3],
            highlight: [0.0; 3],
        });
        Self {
            background: rgba(palette.background),
            ink: rgba(palette.ink),
            highlight: rgba(palette.highlight),
            grid: [grid.0 as f32, grid.1 as f32],
            viewport: [viewport.0 as f32, viewport.1 as f32],
            separation: style.separation.clamp(0.0, 1.0),
            flags: u32::from(style.palette.is_some()) | (u32::from(style.clean_background) << 1),
            pad: [0.0; 2],
        }
    }
}

fn decode_red_channel(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().expect("embedded atlas PNG");
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).expect("embedded atlas PNG");
    let channels = info.color_type.samples();
    let red = pixels[..info.buffer_size()].chunks(channels).map(|px| px[0]).collect();
    (info.width, info.height, red)
}

/// Atlas textures + pipeline, shared by every ASCII player of a context
/// (cached in `GpuContext::ascii_pipelines`).
pub(crate) struct AsciiPipeline {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    fill: wgpu::TextureView,
    edges: wgpu::TextureView,
}

fn ascii_pipeline(ctx: &Rc<GpuContext>, format: wgpu::TextureFormat) -> Rc<AsciiPipeline> {
    if let Some(found) = ctx.ascii_pipelines.borrow().iter().find(|p| p.format == format) {
        return found.clone();
    }
    {
        let device = &ctx.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ascii-video-v1"),
            source: wgpu::ShaderSource::Wgsl(ASCII_WGSL.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ascii-video-v1"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vsAscii"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fsAscii"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let atlas = |label: &str, png_bytes: &[u8]| {
            let (width, height, red) = decode_red_channel(png_bytes);
            let texture = wgpu::util::DeviceExt::create_texture_with_data(
                device,
                &ctx.queue,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &red,
            );
            texture.create_view(&Default::default())
        };
        let built = Rc::new(AsciiPipeline {
            format,
            pipeline,
            fill: atlas("ascii fill atlas", FILL_ATLAS_PNG),
            edges: atlas("ascii edge atlas", EDGE_ATLAS_PNG),
        });
        ctx.ascii_pipelines.borrow_mut().push(built.clone());
        built
    }
}

/// Plays one ASCII clip: tracks clip time, reads a frame only when the clip
/// frame changes, and draws it.
pub struct AsciiPlayer {
    ctx: Rc<GpuContext>,
    pipeline: Rc<AsciiPipeline>,
    clip: AsciiClip,
    dir: PathBuf,
    cells: wgpu::Texture,
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    frame: Vec<u8>,
    elapsed: f64,
    uploaded: Option<u32>,
    failed: bool,
}

impl AsciiPlayer {
    pub fn new(ctx: &Rc<GpuContext>, format: wgpu::TextureFormat, dir: &Path) -> Result<Self, String> {
        let clip = AsciiClip::open(dir)?;
        let pipeline = ascii_pipeline(ctx, format);
        let device = &ctx.device;
        let cells = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ascii cells"),
            size: wgpu::Extent3d { width: clip.columns, height: clip.rows, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ascii params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cells_view = cells.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ascii"),
            layout: &pipeline.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&cells_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&pipeline.fill) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&pipeline.edges) },
            ],
        });
        let frame = vec![0; clip.frame_bytes()];
        Ok(Self {
            ctx: ctx.clone(),
            pipeline,
            clip,
            dir: dir.to_path_buf(),
            cells,
            params,
            bind_group,
            frame,
            elapsed: 0.0,
            uploaded: None,
            failed: false,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn grid(&self) -> (u32, u32) {
        (self.clip.columns, self.clip.rows)
    }

    /// Advances clip time; `delta` is clamped like the WebGL player.
    pub fn advance(&mut self, delta: f32) {
        self.elapsed += f64::from(delta.clamp(0.0, 0.1));
    }

    /// Jumps to an absolute clip time; hosts that share one clock across
    /// outputs drive playback with it.
    pub fn seek(&mut self, seconds: f64) {
        self.elapsed = seconds.max(0.0);
    }

    pub fn current_frame(&self) -> u32 {
        self.frame_at(self.elapsed)
    }

    /// Clip frame shown at `seconds` of playback.
    pub fn frame_at(&self, seconds: f64) -> u32 {
        ((seconds.max(0.0) * self.clip.fps).floor() as u64 % u64::from(self.clip.frame_count)) as u32
    }

    /// True when drawing at `seconds` would show a different clip frame than
    /// the one already on screen. Hosts skip presenting otherwise.
    pub fn changes_at(&self, seconds: f64) -> bool {
        self.uploaded != Some(self.frame_at(seconds))
    }

    /// Time from `seconds` until the next clip frame starts.
    pub fn until_next_frame(&self, seconds: f64) -> std::time::Duration {
        let position = seconds.max(0.0) * self.clip.fps;
        let next = (position.floor() + 1.0) / self.clip.fps;
        std::time::Duration::from_secs_f64((next - seconds.max(0.0)).max(0.0))
    }

    /// Uploads the current clip frame if it changed, then draws it into
    /// `view` (`viewport` = its size in pixels). A read error keeps the last
    /// uploaded frame on screen and is reported once.
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        viewport: (u32, u32),
        style: &AsciiStyle,
    ) {
        let frame = self.current_frame();
        if self.uploaded != Some(frame) {
            match self.clip.read_frame(frame, &mut self.frame) {
                Ok(()) => {
                    self.ctx.queue.write_texture(
                        self.cells.as_image_copy(),
                        &self.frame,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(self.clip.columns * 2),
                            rows_per_image: None,
                        },
                        self.cells.size(),
                    );
                    self.uploaded = Some(frame);
                    self.failed = false;
                }
                Err(e) if !self.failed => {
                    eprintln!("pw: {e}");
                    self.failed = true;
                }
                Err(_) => {}
            }
        }
        let params = Params::new(style, self.grid(), viewport);
        self.ctx.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ascii"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Every pixel is written by the full-screen triangle.
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// CPU transcription of the WebGL fragment shader, for parity tests.
#[cfg(test)]
pub(crate) mod reference {
    use super::*;

    fn unpack(packed: u8) -> [f32; 3] {
        let p = u32::from(packed);
        [((p >> 5) & 7) as f32 / 7.0, ((p >> 2) & 7) as f32 / 7.0, (p & 3) as f32 / 3.0]
    }

    fn luma(c: [f32; 3]) -> f32 {
        c[0] * 0.2127 + c[1] * 0.7152 + c[2] * 0.0722
    }

    fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    fn mix(a: f32, b: f32, t: f32) -> f32 {
        a * (1.0 - t) + b * t
    }

    pub struct Atlases {
        fill: (u32, Vec<u8>),
        edges: (u32, Vec<u8>),
    }

    impl Atlases {
        pub fn load() -> Self {
            let (fw, _, fill) = decode_red_channel(FILL_ATLAS_PNG);
            let (ew, _, edges) = decode_red_channel(EDGE_ATLAS_PNG);
            Self { fill: (fw, fill), edges: (ew, edges) }
        }

        fn mask(&self, glyph: u8, local: (f32, f32)) -> f32 {
            let t = ((local.1 * 8.0 + 0.5).floor() as i32).clamp(0, 7);
            let row = (7 - t) as u32;
            let (base, (width, data)) = if glyph < 10 {
                (f32::from(glyph), &self.fill)
            } else {
                (f32::from(glyph - 9), &self.edges)
            };
            let x = ((base * 8.0 + local.0 * 8.0 + 0.5).floor() as i32).clamp(0, *width as i32 - 1);
            f32::from(data[(row * width + x as u32) as usize]) / 255.0
        }
    }

    /// RGB (0..1) at pixel (x, y) for `cells` at `viewport`, sampled at the
    /// pixel center plus `nudge` pixels.
    pub fn pixel(
        atlases: &Atlases,
        cells: &[u8],
        grid: (u32, u32),
        viewport: (u32, u32),
        style: &AsciiStyle,
        (x, y): (u32, u32),
        nudge: (f32, f32),
    ) -> [f32; 3] {
        let (cols, rows) = (grid.0 as i32, grid.1 as i32);
        let cell_at = |cx: i32, cy: i32| {
            let i = ((cy.clamp(0, rows - 1) * cols + cx.clamp(0, cols - 1)) * 2) as usize;
            (cells[i], cells[i + 1])
        };
        let u = (x as f32 + 0.5 + nudge.0) / viewport.0 as f32 * grid.0 as f32;
        let v = (y as f32 + 0.5 + nudge.1) / viewport.1 as f32 * grid.1 as f32;
        let (cx, cy) = (u.floor() as i32, v.floor() as i32);
        let (glyph, packed) = cell_at(cx, cy);
        let mask = atlases.mask(glyph, (u.fract(), v.fract()));
        let rgb = unpack(packed);
        let l = luma(rgb);
        let mut visibility = 1.0;
        if style.clean_background {
            let la = |dx: i32, dy: i32| luma(unpack(cell_at(cx + dx, cy + dy).1));
            let contrast = (l - la(1, 0)).abs().max((l - la(-1, 0)).abs())
                .max((l - la(0, 1)).abs().max((l - la(0, -1)).abs()));
            let cutoff = mix(0.78, 0.38, style.separation.clamp(0.0, 1.0));
            let foreground = 1.0 - smoothstep(cutoff - 0.30, cutoff, l);
            visibility = foreground * mix(0.30, 1.0, smoothstep(0.02, 0.12, contrast));
        }
        match style.palette {
            None if !style.clean_background => {
                if mask < 0.05 { [0.0; 3] } else { rgb }
            }
            palette => {
                let (ink, background) = match palette {
                    Some(p) => {
                        let t = smoothstep(0.08, 0.42, l);
                        ([0, 1, 2].map(|i| mix(p.ink[i], p.highlight[i], t)), p.background)
                    }
                    None => (rgb, [0.0; 3]),
                };
                [0, 1, 2].map(|i| mix(background[i], ink[i], mask * visibility))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pw-ascii-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn header(columns: u32, rows: u32, frames: u32) -> Vec<u8> {
        let mut h = Vec::from(*b"PWASCII1");
        h.extend(1u16.to_le_bytes());
        h.extend(48u16.to_le_bytes());
        h.extend(8u16.to_le_bytes());
        h.extend(0u16.to_le_bytes());
        for v in [columns * 8, rows * 8, columns, rows, frames, 30, 1, 0] {
            h.extend(v.to_le_bytes());
        }
        h
    }

    #[test]
    fn single_clip_reads_frames_and_rejects_bad_files() {
        let dir = temp_dir("single");
        let mut bytes = header(3, 2, 2);
        bytes.extend((0..24).map(|i| i as u8));
        std::fs::write(dir.join("clip.asciivideo"), &bytes).unwrap();
        let mut clip = AsciiClip::open(&dir).unwrap();
        assert_eq!((clip.columns, clip.rows, clip.frame_count, clip.fps), (3, 2, 2, 30.0));
        let mut frame = vec![0; clip.frame_bytes()];
        clip.read_frame(1, &mut frame).unwrap();
        assert_eq!(frame, (12..24).collect::<Vec<u8>>());

        std::fs::write(dir.join("clip.asciivideo"), &bytes[..bytes.len() - 1]).unwrap();
        assert!(AsciiClip::open(&dir).is_err(), "truncated payload");
        bytes[0] = b'X';
        std::fs::write(dir.join("clip.asciivideo"), &bytes).unwrap();
        assert!(AsciiClip::open(&dir).is_err(), "bad magic");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Renders `player` at `viewport` and returns RGB rows (BGRA readback).
    fn render(ctx: &Rc<GpuContext>, player: &mut AsciiPlayer, viewport: (u32, u32), style: &AsciiStyle) -> Vec<[u8; 3]> {
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: viewport.0, height: viewport.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let row = (viewport.0 * 4).div_ceil(256) * 256;
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * viewport.1),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        player.encode(&mut encoder, &target.create_view(&Default::default()), viewport, style);
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
            },
            target.size(),
        );
        ctx.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        ctx.device.poll(wgpu::PollType::Wait).unwrap();
        let data = slice.get_mapped_range();
        (0..viewport.1)
            .flat_map(|y| {
                let line = &data[(y * row) as usize..][..(viewport.0 * 4) as usize];
                line.chunks(4).map(|px| [px[2], px[1], px[0]]).collect::<Vec<_>>()
            })
            .collect()
    }

    fn styles() -> [AsciiStyle; 4] {
        let palette = AsciiPalette {
            background: [0.118, 0.118, 0.180],
            ink: [0.580, 0.886, 0.835],
            highlight: [0.580, 0.886, 0.835],
        };
        [
            AsciiStyle { palette: None, clean_background: false, separation: 0.5 },
            AsciiStyle { palette: None, clean_background: true, separation: 0.2 },
            AsciiStyle { palette: Some(palette), clean_background: false, separation: 0.5 },
            AsciiStyle { palette: Some(AsciiPalette { highlight: [0.9, 0.95, 1.0], ..palette }), clean_background: true, separation: 0.8 },
        ]
    }

    /// Max channel difference (0..255) between the GPU frame and the CPU
    /// transcription of the WebGL shader. Pixels whose center lands exactly
    /// on a cell or atlas texel edge may round to either side on any GPU
    /// (WebGL included), so a pixel also matches when it equals the reference
    /// sampled a thousandth of a pixel away.
    fn max_difference(player: &mut AsciiPlayer, viewport: (u32, u32), style: &AsciiStyle, ctx: &Rc<GpuContext>) -> u8 {
        let gpu = render(ctx, player, viewport, style);
        let mut cells = vec![0; player.clip.frame_bytes()];
        player.clip.read_frame(player.current_frame(), &mut cells).unwrap();
        let atlases = reference::Atlases::load();
        let difference = |want: [f32; 3], got: [u8; 3]| {
            (0..3)
                .map(|c| ((want[c].clamp(0.0, 1.0) * 255.0).round() as i32 - i32::from(got[c])).unsigned_abs() as u8)
                .max()
                .unwrap()
        };
        let e = 1e-3;
        let nudges = [(0.0, 0.0), (e, 0.0), (-e, 0.0), (0.0, e), (0.0, -e), (e, e), (-e, -e), (e, -e), (-e, e)];
        let mut worst = 0;
        for y in 0..viewport.1 {
            for x in 0..viewport.0 {
                let got = gpu[(y * viewport.0 + x) as usize];
                let best = nudges
                    .iter()
                    .map(|&n| difference(reference::pixel(&atlases, &cells, player.grid(), viewport, style, (x, y), n), got))
                    .min()
                    .unwrap();
                worst = worst.max(best);
            }
        }
        worst
    }

    #[test]
    fn gpu_matches_webgl_reference_on_synthetic_cells() {
        let dir = temp_dir("parity");
        // Every glyph id with varied RGB332 colors, two frames.
        let (columns, rows) = (16u32, 9u32);
        let mut bytes = header(columns, rows, 2);
        for frame in 0..2u32 {
            for i in 0..columns * rows {
                bytes.push(((i + frame) % 14) as u8);
                bytes.push((i.wrapping_mul(37) + frame * 101) as u8);
            }
        }
        std::fs::write(dir.join("clip.asciivideo"), &bytes).unwrap();
        let ctx = GpuContext::headless(wgpu::Features::empty()).expect("GPU adapter");
        let mut player = AsciiPlayer::new(&ctx, wgpu::TextureFormat::Bgra8Unorm, &dir).unwrap();
        // 10 px cells exercise the half-texel atlas offset; 7 px cells do not
        // divide the atlas evenly.
        for viewport in [(160, 90), (112, 63)] {
            for (i, style) in styles().iter().enumerate() {
                for seconds in [0.0, 1.0 / 30.0] {
                    player.seek(seconds);
                    let worst = max_difference(&mut player, viewport, style, &ctx);
                    assert!(worst <= 1, "viewport {viewport:?} style {i} t {seconds}: diff {worst}");
                }
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gpu_matches_webgl_reference_on_bundled_clip() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Sources/ParticleWall/Resources/SpiderManASCIIWallpaper");
        let ctx = GpuContext::headless(wgpu::Features::empty()).expect("GPU adapter");
        let mut player = AsciiPlayer::new(&ctx, wgpu::TextureFormat::Bgra8Unorm, &dir).unwrap();
        for (i, style) in styles().iter().enumerate() {
            for seconds in [0.0, 3.5, 9.25] {
                player.seek(seconds);
                let worst = max_difference(&mut player, (640, 360), style, &ctx);
                assert!(worst <= 1, "style {i} t {seconds}: diff {worst}");
            }
        }
    }

    #[test]
    fn player_reports_frame_changes_and_next_frame_time() {
        let dir = temp_dir("timing");
        let mut bytes = header(2, 1, 3);
        bytes.extend([0u8; 12]);
        std::fs::write(dir.join("clip.asciivideo"), &bytes).unwrap();
        let ctx = GpuContext::headless(wgpu::Features::empty()).expect("GPU adapter");
        let mut player = AsciiPlayer::new(&ctx, wgpu::TextureFormat::Bgra8Unorm, &dir).unwrap();
        // 30 fps, 3 frames: frame boundaries every 1/30 s, wrapping at 0.1 s.
        assert_eq!(player.frame_at(0.05), 1);
        assert_eq!(player.frame_at(0.11), 0);
        let wait = player.until_next_frame(0.05).as_secs_f64();
        assert!((wait - (2.0 / 30.0 - 0.05)).abs() < 1e-9, "{wait}");
        assert!(player.changes_at(0.0), "nothing uploaded yet");
        player.seek(0.0);
        render(&ctx, &mut player, (8, 4), &AsciiStyle::default());
        assert!(!player.changes_at(0.02), "same clip frame");
        assert!(player.changes_at(0.04), "next clip frame");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn chunked_clip_reads_across_chunks_and_rejects_short_chunks() {
        let dir = temp_dir("chunked");
        std::fs::create_dir(dir.join("frames")).unwrap();
        std::fs::write(
            dir.join("stream.json"),
            r#"{"columns":2,"rows":1,"frameCount":3,"fps":30.0,"chunkFrames":2}"#,
        )
        .unwrap();
        std::fs::write(dir.join("frames/0.bin"), [0u8, 1, 2, 3, 4, 5, 6, 7]).unwrap();
        std::fs::write(dir.join("frames/1.bin"), [8u8, 9, 10, 11]).unwrap();
        let mut clip = AsciiClip::open(&dir).unwrap();
        let mut frame = vec![0; 4];
        for (index, expected) in [(1, [4, 5, 6, 7]), (2, [8, 9, 10, 11]), (0, [0, 1, 2, 3])] {
            clip.read_frame(index, &mut frame).unwrap();
            assert_eq!(frame, expected);
        }
        std::fs::write(dir.join("frames/1.bin"), [8u8, 9]).unwrap();
        let mut clip = AsciiClip::open(&dir).unwrap();
        assert!(clip.read_frame(2, &mut frame).is_err(), "short chunk");
        std::fs::write(dir.join("stream.json"), r#"{"columns":2,"rows":1,"frameCount":3,"fps":0,"chunkFrames":2}"#).unwrap();
        assert!(AsciiClip::open(&dir).is_err(), "invalid fps");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
