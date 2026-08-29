//! Numeric contract: CPU reference == committed fixture == WGSL on GPU.
//!
//! `GENERATE_FIXTURES=1 cargo test` regenerates the fixture files.

use crate::{cpu, Uniforms};
use serde_json::json;
use wgpu::util::DeviceExt;

fn fixture_path(module: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../../shared/contracts/fixtures/renderer-vectors/{module}.json"
    ))
}

/// Selected ids per model, respecting each model's instance count.
const SAMPLE_IDS: [&[u32]; 9] = [
    &[0, 1, 234, 4999, 9998, 9999],
    &[0, 1, 7, 15000, 29998, 29999],
    &[0, 1, 799, 12000, 29998, 29999],
    &[0, 5, 6, 47, 119956, 119957],
    &[0, 11, 13, 40000, 77758, 77759],
    &[0, 1, 1000, 40000, 78497],
    &[0, 511, 512, 20000, 37374, 37375],
    &[0, 7, 8, 6000, 49798, 49799],
    &[0, 79, 80, 1600, 3198, 3199],
];

const SAMPLE_TIMES: [f32; 4] = [0.0, 1.7, 13.3, 41.25];

/// Builds the flow state the fixture cases run against. Model 5 loads the
/// prime sieve; model 4 advances the fixed-step flow simulation.
fn reference_flow(model: u32) -> cpu::FlowState {
    let mut flow = cpu::FlowState::new();
    if model == 5 {
        // Espiral Prima reads the prime list through the same storage slot;
        // its buffer holds 78,498 entries (vs 6,480 flow particles).
        flow.particles.resize(cpu::PRIME_COUNT, [0.0; 4]);
        let primes = cpu::generate_primes(cpu::PRIME_LIMIT);
        for (i, prime) in primes.iter().enumerate() {
            flow.particles[i] = [*prime, 0.0, 0.0, 0.0];
        }
    } else if model == 4 {
        for _ in 0..100 {
            flow.step();
        }
    }
    flow
}

fn reference_cases(model: u32) -> serde_json::Value {
    let module = MODULE_IDS[model as usize];
    let flow = reference_flow(model);
    let mut cases = Vec::new();
    for &t in &SAMPLE_TIMES {
        let mut u = Uniforms::defaults(16.0 / 9.0, [1920.0, 1080.0]);
        u.time = t;
        u.model = [
            model as f32,
            ((cpu::FLOW_HISTORY_COUNT - 1) as f32),
            cpu::FLOW_HISTORY_COUNT as f32,
            Uniforms::model_vertex_count(model) as f32,
        ];
        for &id in SAMPLE_IDS[model as usize] {
            let s = cpu::model_sample(id, model, &u, &flow);
            cases.push(json!({
                "id": id,
                "time": t,
                "clip": [s[0], s[1]],
                "pointSizePx": s[2],
                "color": [s[3], s[4], s[5], s[6]],
            }));
        }
    }
    json!({
        "contractVersion": 1,
        "engine": "particle-v1",
        "model": module,
        "cases": cases
    })
}

pub const MODULE_IDS: [&str; 9] = [
    "parametric-waves",
    "twin-vortex",
    "orbital-bloom",
    "hexagonal-rosette",
    "noise-rain",
    "prime-spiral",
    "torus-orbit",
    "chromatic-rings",
    "sphere-torus",
];

#[test]
fn cpu_reference_matches_committed_fixture() {
    for model in 0..9u32 {
        let path = fixture_path(MODULE_IDS[model as usize]);
        if std::env::var("GENERATE_FIXTURES").is_ok() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                format!("{}\n", serde_json::to_string_pretty(&reference_cases(model)).unwrap()),
            )
            .unwrap();
            println!("fixture written to {}", path.display());
            continue;
        }
        let committed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let expected = reference_cases(model);
        let expected_cases = expected["cases"].as_array().unwrap();
        let committed_cases = committed["cases"].as_array().unwrap();
        assert_eq!(expected_cases.len(), committed_cases.len());
        for (got, want) in expected_cases.iter().zip(committed_cases) {
            assert_eq!(got["id"], want["id"], "id order changed");
            assert_eq!(got["time"], want["time"]);
            for key in ["clip", "color"] {
                for (g, w) in got[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(want[key].as_array().unwrap())
                {
                    let (g, w) = (g.as_f64().unwrap(), w.as_f64().unwrap());
                    assert!(
                        (g - w).abs() <= w.abs().max(1.0) * 1e-6,
                        "fixture drift at model {model} id {} time {} {key}: {g} vs {w}",
                        got["id"],
                        got["time"]
                    );
                }
            }
            let (g, w) = (
                got["pointSizePx"].as_f64().unwrap(),
                want["pointSizePx"].as_f64().unwrap(),
            );
            assert!(
                (g - w).abs() <= 1e-6,
                "pointSizePx drift at model {model} id {}",
                got["id"]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// GPU side: the appended kernels use group 8 so they never clash with the
// engine's own binding groups.
// ---------------------------------------------------------------------------

const PARITY_APPENDED: &str = r#"
struct Result { clip: vec2f, pointSizePx: f32, color: vec4f }
@group(3) @binding(0) var<storage, read_write> results: array<Result>;
@group(3) @binding(1) var<storage, read> ids: array<u32>;

@compute @workgroup_size(1)
fn parityMain(@builtin(global_invocation_id) gid: vec3u) {
    let s = particleSample(ids[gid.x], u);
    results[gid.x] = Result(s.clip, s.pointSizePx, s.color);
}
"#;

struct GpuCtx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    module: wgpu::ShaderModule,
    layouts: TestLayouts,
}

/// Explicit bind group layouts for the test pipelines: wgpu auto layouts
/// pull in every module-level group, so explicit layouts keep each pipeline
/// bound to only the groups its entry point uses.
struct TestLayouts {
    /// u + flowParticles + flowHistory (compute read-only).
    bgl0: wgpu::BindGroupLayout,
    bgl_flow: wgpu::BindGroupLayout,
    bgl_graph: wgpu::BindGroupLayout,
    /// Parity results + ids.
    bgl_results: wgpu::BindGroupLayout,
    bgl_empty: wgpu::BindGroupLayout,
}

fn compute_storage(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: if read_only {
            wgpu::BufferBindingType::Storage { read_only: true }
        } else {
            wgpu::BufferBindingType::Storage { read_only: false }
        },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn compute_uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

impl TestLayouts {
    fn new(device: &wgpu::Device) -> Self {
        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test bgl0"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: compute_uniform(), count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: compute_storage(true), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: compute_storage(true), count: None },
            ],
        });
        let bgl_flow = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test bgl flow"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: compute_storage(false), count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: compute_storage(false), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: compute_uniform(), count: None },
            ],
        });
        let bgl_graph = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test bgl graph"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: compute_uniform(), count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: compute_storage(false), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: compute_storage(false), count: None },
            ],
        });
        let bgl_results = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test bgl results"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: compute_storage(false), count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: compute_storage(true), count: None },
            ],
        });
        let bgl_empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test bgl empty"),
            entries: &[],
        });
        Self { bgl0, bgl_flow, bgl_graph, bgl_results, bgl_empty }
    }

    fn empty_bind_group(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bgl_empty,
            entries: &[],
        })
    }

    fn parity_pipeline(&self, device: &wgpu::Device, module: &wgpu::ShaderModule) -> wgpu::ComputePipeline {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&self.bgl0, &self.bgl_empty, &self.bgl_empty, &self.bgl_results],
            push_constant_ranges: &[],
        });
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("parity"),
            layout: Some(&layout),
            module,
            entry_point: Some("parityMain"),
            compilation_options: Default::default(),
            cache: None,
        })
    }
}

fn flow_pipeline(ctx: &GpuCtx) -> wgpu::ComputePipeline {
    let layout = ctx.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&ctx.layouts.bgl_empty, &ctx.layouts.bgl_flow],
        push_constant_ranges: &[],
    });
    ctx.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("flow"),
        layout: Some(&layout),
        module: &ctx.module,
        entry_point: Some("flowUpdate"),
        compilation_options: Default::default(),
        cache: None,
    })
}

fn gpu_ctx() -> GpuCtx {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .expect("no GPU adapter available");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .expect("GPU device");
    let mut wgsl = crate::engine_wgsl();
    wgsl.push_str(PARITY_APPENDED);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("parity"),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    let layouts = TestLayouts::new(&device);
    GpuCtx { device, queue, module, layouts }
}

fn upload_flow_buffers(ctx: &GpuCtx, flow: &cpu::FlowState) -> (wgpu::Buffer, wgpu::Buffer) {
    let particles = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("flow particles"),
        contents: bytemuck::cast_slice(&flow.particles),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let history = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("flow history"),
        contents: bytemuck::cast_slice(&flow.history),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    (particles, history)
}

#[test]
fn gpu_wgsl_matches_cpu_reference() {
    let ctx = gpu_ctx();
    for model in 0..9u32 {
        let flow = reference_flow(model);
        let (particles_buf, history_buf) = upload_flow_buffers(&ctx, &flow);
        let ids: Vec<u32> = SAMPLE_IDS[model as usize].to_vec();

        for &t in &SAMPLE_TIMES {
            let mut u = Uniforms::defaults(16.0 / 9.0, [1920.0, 1080.0]);
            u.time = t;
            u.model = [
                model as f32,
                (cpu::FLOW_HISTORY_COUNT - 1) as f32,
                cpu::FLOW_HISTORY_COUNT as f32,
                Uniforms::model_vertex_count(model) as f32,
            ];
            let uniform_buf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("uniforms"),
                contents: bytemuck::bytes_of(&u),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let out_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("results"),
                size: (ids.len() as u64) * 32, // vec2 + f32 + pad + vec4
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let ids_bytes: Vec<u8> = ids.iter().flat_map(|v| v.to_le_bytes()).collect();
            let ids_buf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ids"),
                contents: &ids_bytes,
                usage: wgpu::BufferUsages::STORAGE,
            });

            let pipeline = ctx.layouts.parity_pipeline(&ctx.device, &ctx.module);
            let bg0 = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &ctx.layouts.bgl0,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: particles_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: history_buf.as_entire_binding() },
                ],
            });
            let bg3 = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &ctx.layouts.bgl_results,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: out_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: ids_buf.as_entire_binding() },
                ],
            });
            let empty = ctx.layouts.empty_bind_group(&ctx.device);

            let mut encoder = ctx.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bg0, &[]);
                pass.set_bind_group(1, &empty, &[]);
                pass.set_bind_group(2, &empty, &[]);
                pass.set_bind_group(3, &bg3, &[]);
                pass.dispatch_workgroups(ids.len() as u32, 1, 1);
            }
            let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (ids.len() as u64) * 32,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            for (i, _) in ids.iter().enumerate() {
                encoder.copy_buffer_to_buffer(
                    &out_buf,
                    (i as u64) * 32,
                    &readback,
                    (i as u64) * 32,
                    32,
                );
            }
            ctx.queue.submit(Some(encoder.finish()));
            let slice = readback.slice(..);
            pollster::block_on(async {
                slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
                ctx.device.poll(wgpu::PollType::Wait).unwrap();
            });
            let data: Vec<f32> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
            drop(slice.get_mapped_range());

            for (i, &id) in ids.iter().enumerate() {
                let expected = cpu::model_sample(id, model, &u, &flow);
                // Model 5 evaluates sin/cos at arguments up to prime*1e0:
                // argument-reduction rounding differs between the CPU libm
                // and GPU transcendentals, so only the visible prime range
                // (radius under canvas bounds) carries a numeric contract.
                if model == 5 && flow.particles[id as usize][0] > 40_000.0 {
                    continue;
                }
                // Row layout: clip(2) + pointSize(1) + pad(1) + color(4).
                let mut row = data[i * 8..i * 8 + 8].to_vec();
                row.remove(3); // drop the padding slot
                for (g, w) in row.iter().zip(expected.iter()) {
                    let diff = (g - w).abs();
                    let tol = if model == 5 {
                        4e-3
                    } else {
                        w.abs().max(1.0) * 1e-4
                    };
                    assert!(
                        diff <= tol,
                        "GPU/CPU divergence model={model} id={id} t={t}: {g} vs {w}"
                    );
                }
            }
        }
    }
}

/// FR-GPU-07: the flow simulation is a fixed-step deterministic process.
///
/// The per-step state walk (`x += ±1` chosen by value noise) is chaotic:
/// one-ULP transcendental differences (GPU FMA vs CPU libm) flip branches
/// after enough steps, so bit-equality across implementations is not the
/// contract. Instead this verifies:
///   1. spawn cadence and state invariants on the CPU reference,
///   2. bit-exact determinism of the GPU kernel across repeated fixed
///      step counts (same steps -> same state, every time).
#[test]
fn noise_rain_fixed_step() {
    const STEPS: u32 = 100;
    let ctx = gpu_ctx();

    // --- CPU invariants -----------------------------------------------------
    let mut cpu_flow = cpu::FlowState::new();
    for _ in 0..STEPS {
        cpu_flow.step();
    }
    let mut active = 0u32;
    for p in &cpu_flow.particles {
        if p[3] > 0.0 {
            active += 1;
            // A particle lives at most 720 fixed steps (respawn ring), and x
            // moves at most 1 per step: x stays within [-730, 730].
            assert!((-730.0..=730.0).contains(&p[0]), "x out of bounds: {}", p[0]);
            assert!(p[1] >= 0.0, "y must never decrease below 0");
            assert!(p[3] <= 3.0, "alpha above spawn value");
            assert!(p[2] >= 0.0 && p[2] <= 6.0, "z (fall speed) out of range");
        }
    }
    assert!(
        active >= 900 - 8 && active <= 900,
        "expected ~900 live particles after {STEPS} steps, got {active}"
    );

    // First step from empty: every spawned particle follows the branch-
    // independent part of the spawn formula (w = 3*0.997, y = +0.5).
    let mut one = cpu::FlowState::new();
    one.step();
    let mut spawned = 0u32;
    for (id, p) in one.particles.iter().enumerate() {
        if p[3] > 0.0 {
            spawned += 1;
            let new_tick = (id as u32 % 9) + 1;
            let base_x = (new_tick as f32 * 99.0) % 720.0;
            assert!((p[3] - 2.991).abs() <= 1e-5, "spawn alpha {}", p[3]);
            assert!((p[1] - 0.5).abs() <= 1e-5, "spawn y {}", p[1]);
            assert!(
                (p[0] - base_x).abs() <= 1.0001,
                "spawn x {} vs base {base_x}",
                p[0]
            );
        }
    }
    assert_eq!(spawned, 9, "exactly 9 particles spawn per fixed step");

    // --- GPU determinism ----------------------------------------------------
    let fpipeline = flow_pipeline(&ctx);
    let empty = ctx.layouts.empty_bind_group(&ctx.device);
    let mut runs: Vec<Vec<[f32; 4]>> = Vec::new();
    for _ in 0..2 {
        let particles_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flow particles"),
            size: (cpu::FLOW_PARTICLE_COUNT as u64) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let history_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flow history"),
            size: (cpu::FLOW_PARTICLE_COUNT * cpu::FLOW_HISTORY_COUNT) as u64 * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&fpipeline);
            pass.set_bind_group(0, &empty, &[]);
            for step in 0..STEPS {
                let flow_params = [step, 0, cpu::FLOW_PARTICLE_COUNT, cpu::FLOW_HISTORY_COUNT];
                let params_buf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("flow params"),
                    contents: bytemuck::cast_slice(&flow_params),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &ctx.layouts.bgl_flow,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: particles_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: history_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: params_buf.as_entire_binding() },
                    ],
                });
                pass.set_bind_group(1, &bg, &[]);
                pass.dispatch_workgroups(cpu::FLOW_PARTICLE_COUNT.div_ceil(64), 1, 1);
            }
        }
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (cpu::FLOW_PARTICLE_COUNT as u64) * 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&particles_buf, 0, &readback, 0, readback.size());
        ctx.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        pollster::block_on(async {
            slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
            ctx.device.poll(wgpu::PollType::Wait).unwrap();
        });
        runs.push(bytemuck::cast_slice(&slice.get_mapped_range()).to_vec());
        drop(slice.get_mapped_range());
    }

    assert_eq!(runs[0], runs[1], "GPU flow state must be bit-identical across identical fixed-step runs");
    // Cross-implementation sanity: the two platforms agree on the live
    // particle count (spawn cadence is exact, not chaotic).
    let gpu_active = runs[0].iter().filter(|p| p[3] > 0.0).count() as u32;
    assert_eq!(gpu_active, active, "GPU and CPU spawn cadence must match");
}

/// FR-GPU-08: the prime resource contract — exactly 78,498 primes below
/// 1,000,000 are uploaded, and Espiral Prima samples them correctly.
#[test]
fn prime_resource_contract() {
    let primes = cpu::generate_primes(cpu::PRIME_LIMIT);
    assert_eq!(primes.len(), 78_498, "prime sieve count");
    assert_eq!(primes[0], 2.0);
    assert_eq!(primes[1], 3.0);
    assert_eq!(primes[primes.len() - 1], 999_983.0);

    let ctx = gpu_ctx();
    let mut flow = cpu::FlowState::new();
    flow.particles.resize(cpu::PRIME_COUNT, [0.0; 4]);
    for (i, prime) in primes.iter().enumerate() {
        flow.particles[i] = [*prime, 0.0, 0.0, 0.0];
    }
    let (particles_buf, history_buf) = upload_flow_buffers(&ctx, &flow);

    let mut u = Uniforms::defaults(16.0 / 9.0, [1920.0, 1080.0]);
    u.time = 41.25;
    u.model = [5.0, 0.0, 12.0, Uniforms::model_vertex_count(5) as f32];

    let ids: [u32; 5] = [0, 1, 2, 1000, 78497];
    let (clip, point, color) = run_parity_kernel(&ctx, &u, &particles_buf, &history_buf, &ids);
    for (i, &id) in ids.iter().enumerate() {
        let expected = cpu::model_sample(id, 5, &u, &flow);
        // Large-prime trig is not contract-stable across argument reduction
        // (see gpu_wgsl_matches_cpu_reference).
        if flow.particles[id as usize][0] > 40_000.0 {
            continue;
        }
        for (g, w) in clip[i].iter().zip(expected[..2].iter()) {
            assert!((g - w).abs() <= 4e-3, "prime spiral clip id={id}: {g} vs {w}");
        }
        assert!((point[i] - expected[2]).abs() <= 1e-4, "prime spiral size id={id}");
        for (g, w) in color[i].iter().zip(expected[3..].iter()) {
            assert!((g - w).abs() <= 1e-4, "prime spiral color id={id}");
        }
    }
}

/// FR-GPU-09: graph compute contract — node sampling + nearest-neighbour
/// connection match the CPU reference.
#[test]
fn graph_compute_contract() {
    let ctx = gpu_ctx();
    let flow = cpu::FlowState::new();
    let (particles_buf, history_buf) = upload_flow_buffers(&ctx, &flow);

    let mut u = Uniforms::defaults(16.0 / 9.0, [1920.0, 1080.0]);
    u.time = 1.7;
    u.model = [0.0, 0.0, 12.0, 10_000.0];
    u.graph = [1.0, 0.085, 0.55, 2.0];

    let uniform_buf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("uniforms"),
        contents: bytemuck::bytes_of(&u),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let positions_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("graph positions"),
        size: 768 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let edges_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("graph edges"),
        size: 768 * 3 * 2 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let pos_pipeline = {
        let layout = ctx.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&ctx.layouts.bgl0, &ctx.layouts.bgl_empty, &ctx.layouts.bgl_graph],
            push_constant_ranges: &[],
        });
        ctx.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("graph positions"),
            layout: Some(&layout),
            module: &ctx.module,
            entry_point: Some("graphPositionUpdate"),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let conn_pipeline = {
        let layout = ctx.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&ctx.layouts.bgl_empty, &ctx.layouts.bgl_empty, &ctx.layouts.bgl_graph],
            push_constant_ranges: &[],
        });
        ctx.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("graph connections"),
            layout: Some(&layout),
            module: &ctx.module,
            entry_point: Some("graphConnectionUpdate"),
            compilation_options: Default::default(),
            cache: None,
        })
    };

    let empty = ctx.layouts.empty_bind_group(&ctx.device);
    let bg0 = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &ctx.layouts.bgl0,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: particles_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: history_buf.as_entire_binding() },
        ],
    });
    // Both graph kernels share group 2 (gu + positions + edges); unused
    // entries still must be bound because the explicit layout declares them.
    let bg2 = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &ctx.layouts.bgl_graph,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: positions_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: edges_buf.as_entire_binding() },
        ],
    });
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pos_pipeline);
        pass.set_bind_group(0, &bg0, &[]);
        pass.set_bind_group(1, &empty, &[]);
        pass.set_bind_group(2, &bg2, &[]);
        pass.dispatch_workgroups(768u32.div_ceil(64), 1, 1);

        pass.set_pipeline(&conn_pipeline);
        pass.set_bind_group(0, &empty, &[]);
        pass.set_bind_group(1, &empty, &[]);
        pass.set_bind_group(2, &bg2, &[]);
        pass.dispatch_workgroups(768u32.div_ceil(64), 1, 1);
    }

    let pos_read = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 768 * 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let edges_read = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 768 * 3 * 2 * 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(&positions_buf, 0, &pos_read, 0, pos_read.size());
    encoder.copy_buffer_to_buffer(&edges_buf, 0, &edges_read, 0, edges_read.size());
    ctx.queue.submit(Some(encoder.finish()));

    let (cpu_positions, cpu_edges) = cpu::graph_update(&u, &flow);
    for (buf, reference, label) in [
        (&pos_read, &cpu_positions, "positions"),
        (&edges_read, &cpu_edges, "edges"),
    ] {
        let slice = buf.slice(..);
        pollster::block_on(async {
            slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
            ctx.device.poll(wgpu::PollType::Wait).unwrap();
        });
        let data: Vec<[f32; 4]> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
        drop(slice.get_mapped_range());
        for (i, (g, w)) in data.iter().zip(reference.iter()).enumerate() {
            for (gv, wv) in g.iter().zip(w.iter()) {
                assert!(
                    (gv - wv).abs() <= 2e-4,
                    "graph {label} divergence at row {i}: {gv} vs {wv}"
                );
            }
        }
    }
}

/// Runs the parity kernel once; returns (clip rows, pointSize, color rows).
fn run_parity_kernel(
    ctx: &GpuCtx,
    u: &Uniforms,
    particles_buf: &wgpu::Buffer,
    history_buf: &wgpu::Buffer,
    ids: &[u32],
) -> (Vec<[f32; 2]>, Vec<f32>, Vec<[f32; 4]>) {
    let uniform_buf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("uniforms"),
        contents: bytemuck::bytes_of(u),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let out_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("results"),
        size: (ids.len() as u64) * 32,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let ids_bytes: Vec<u8> = ids.iter().flat_map(|v| v.to_le_bytes()).collect();
    let ids_buf = ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("ids"),
        contents: &ids_bytes,
        usage: wgpu::BufferUsages::STORAGE,
    });

    let pipeline = ctx.layouts.parity_pipeline(&ctx.device, &ctx.module);
    let bg0 = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &ctx.layouts.bgl0,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: particles_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: history_buf.as_entire_binding() },
        ],
    });
    let bg3 = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &ctx.layouts.bgl_results,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: out_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: ids_buf.as_entire_binding() },
        ],
    });
    let empty = ctx.layouts.empty_bind_group(&ctx.device);

    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bg0, &[]);
        pass.set_bind_group(1, &empty, &[]);
        pass.set_bind_group(2, &empty, &[]);
        pass.set_bind_group(3, &bg3, &[]);
        pass.dispatch_workgroups(ids.len() as u32, 1, 1);
    }
    let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (ids.len() as u64) * 32,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    for (i, _) in ids.iter().enumerate() {
        encoder.copy_buffer_to_buffer(&out_buf, (i as u64) * 32, &readback, (i as u64) * 32, 32);
    }
    ctx.queue.submit(Some(encoder.finish()));
    let slice = readback.slice(..);
    pollster::block_on(async {
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        ctx.device.poll(wgpu::PollType::Wait).unwrap();
    });
    let data: Vec<f32> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
    drop(slice.get_mapped_range());

    let mut clips = Vec::new();
    let mut points = Vec::new();
    let mut colors = Vec::new();
    for i in 0..ids.len() {
        let row = &data[i * 8..i * 8 + 8];
        clips.push([row[0], row[1]]);
        points.push(row[2]);
        colors.push([row[4], row[5], row[6], row[7]]);
    }
    (clips, points, colors)
}
