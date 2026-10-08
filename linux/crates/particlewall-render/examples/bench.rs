//! Offscreen GPU/CPU cost per particle-v1 model and vertex path.
//!
//!   cargo run --release -p particlewall-render --example bench -- \
//!       [--size 2560x1440] [--frames 240] [--paths list6,indexed4] [--models 0,10]
//!
//! GPU time comes from timestamp queries around each frame's commands; CPU
//! time is the wall time spent recording and submitting the frame (what the
//! daemon's main thread pays per tick, excluding swapchain acquire/present).
//! "busy@30" is GPU time x 30 frames/s: the share of GPU time the wallpaper
//! occupies at the default FPS cap.

use std::time::Instant;

use particlewall_render::gpu::{GpuContext, ParticleRenderer, VertexPath};
use particlewall_render::Uniforms;

const MODEL_NAMES: [&str; 12] = [
    "parametric-waves",
    "twin-vortex",
    "orbital-bloom",
    "hexagonal-rosette",
    "noise-rain",
    "prime-spiral",
    "torus-orbit",
    "chromatic-rings",
    "sphere-torus",
    "jellyfish-points",
    "nebula",
    "torus-knot",
];

struct Args {
    size: (u32, u32),
    frames: u32,
    paths: Vec<VertexPath>,
    models: Vec<u32>,
    graph: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        size: (2560, 1440),
        frames: 240,
        paths: VertexPath::ALL.to_vec(),
        models: (0..12).collect(),
        graph: false,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let value = argv.get(i + 1).cloned().unwrap_or_default();
        match argv[i].as_str() {
            "--size" => {
                let (w, h) = value.split_once('x').expect("--size WxH");
                args.size = (w.parse().unwrap(), h.parse().unwrap());
            }
            "--frames" => args.frames = value.parse().unwrap(),
            "--paths" => {
                args.paths = value
                    .split(',')
                    .map(|p| match p {
                        "list6" => VertexPath::List6,
                        "indexed4" => VertexPath::Indexed4,
                        other => panic!("unknown path {other}"),
                    })
                    .collect()
            }
            "--models" => args.models = value.split(',').map(|m| m.parse().unwrap()).collect(),
            "--graph" => {
                args.graph = true;
                i += 1;
                continue;
            }
            other => panic!("unknown argument {other}"),
        }
        i += 2;
    }
    args
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let args = parse_args();
    let ctx = GpuContext::headless(
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
    )
    .expect("GPU with timestamp queries");
    let info = ctx.adapter_info();
    println!("adapter: {} ({:?}, {})", info.name, info.backend, info.driver_info);
    println!(
        "target: {}x{} Bgra8Unorm, {} timed frames{}",
        args.size.0,
        args.size.1,
        args.frames,
        if args.graph { ", graph overlay on" } else { "" }
    );

    let format = wgpu::TextureFormat::Bgra8Unorm;
    let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench target"),
        size: wgpu::Extent3d { width: args.size.0, height: args.size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());

    let frames = args.frames;
    let queries = ctx.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("bench timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: frames * 2,
    });
    let resolve = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("timestamp resolve"),
        size: u64::from(frames * 2) * 8,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("timestamp readback"),
        size: resolve.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let period_ns = f64::from(ctx.queue.get_timestamp_period());

    println!(
        "{:<18} {:>9} {:<10} {:>9} {:>9} {:>9} {:>8}",
        "model", "particles", "path", "gpu med", "gpu p90", "cpu med", "busy@30"
    );
    for &model in &args.models {
        let mut list6_median = None;
        for &path in &args.paths {
            ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let mut renderer = ParticleRenderer::new(&ctx, format, model, path);
            let mut u = Uniforms::defaults(
                args.size.0 as f32 / args.size.1 as f32,
                [args.size.0 as f32, args.size.1 as f32],
            );
            // Daemon defaults: particleSize 1.6, brightness 1.5.
            u.point_size = 1.25 * 1.6;
            if args.graph {
                u.graph = [1.0, 0.085, 1.0, 3.0];
            }
            let delta = 1.0 / 30.0;
            let mut cpu_us = Vec::with_capacity(frames as usize);
            for frame in 0..(frames + 30) {
                let timed = frame >= 30;
                u.time = frame as f32 * delta;
                u.model = [
                    model as f32,
                    renderer.latest_flow_slot(),
                    12.0,
                    Uniforms::model_vertex_count(model) as f32,
                ];
                let started = Instant::now();
                let mut encoder = ctx.device.create_command_encoder(&Default::default());
                let slot = (frame.saturating_sub(30)) * 2;
                if timed {
                    encoder.write_timestamp(&queries, slot);
                }
                renderer.encode(&mut encoder, &view, &u, [0.0, 0.0, 0.0, 1.0], delta);
                if timed {
                    encoder.write_timestamp(&queries, slot + 1);
                }
                ctx.queue.submit(Some(encoder.finish()));
                if timed {
                    cpu_us.push(started.elapsed().as_secs_f64() * 1e6);
                }
                // Keep at most one frame in flight, like the presenter.
                ctx.device.poll(wgpu::PollType::Wait).unwrap();
            }
            let mut encoder = ctx.device.create_command_encoder(&Default::default());
            encoder.resolve_query_set(&queries, 0..frames * 2, &resolve, 0);
            encoder.copy_buffer_to_buffer(&resolve, 0, &readback, 0, resolve.size());
            ctx.queue.submit(Some(encoder.finish()));
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |r| r.expect("map timestamps"));
            ctx.device.poll(wgpu::PollType::Wait).unwrap();
            let ticks: Vec<u64> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
            readback.unmap();

            let mut gpu_ms: Vec<f64> = ticks
                .chunks(2)
                .map(|t| t[1].saturating_sub(t[0]) as f64 * period_ns / 1e6)
                .collect();
            gpu_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
            cpu_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median = percentile(&gpu_ms, 0.5);
            let error = pollster::block_on(ctx.device.pop_error_scope());
            let relative = match (path, list6_median) {
                (VertexPath::List6, _) => {
                    list6_median = Some(median);
                    String::new()
                }
                (_, Some(base)) => format!("  ({:.2}x vs list6)", base / median),
                _ => String::new(),
            };
            println!(
                "{:<18} {:>9} {:<10} {:>7.3}ms {:>7.3}ms {:>7.1}us {:>7.2}%{}{}",
                MODEL_NAMES[model as usize],
                Uniforms::model_vertex_count(model),
                format!("{path:?}").to_lowercase(),
                median,
                percentile(&gpu_ms, 0.9),
                percentile(&cpu_us, 0.5),
                median * 30.0 / 10.0,
                relative,
                error.map(|e| format!("  VALIDATION ERROR: {e}")).unwrap_or_default(),
            );
        }
    }
}
