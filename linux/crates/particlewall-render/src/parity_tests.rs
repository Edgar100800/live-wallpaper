//! Numeric contract: CPU reference == committed fixture == WGSL on GPU.
//!
//! `GENERATE_FIXTURES=1 cargo test` regenerates the fixture file.

use crate::{cpu, Uniforms};
use serde_json::json;
use wgpu::util::DeviceExt;

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../shared/contracts/fixtures/renderer-vectors/parametric-waves.json")
}

fn reference_rows() -> serde_json::Value {
    let ids: [u32; 6] = [0, 1, 234, 4999, 9998, 9999];
    let times: [f32; 4] = [0.0, 1.7, 13.3, 41.25];
    let mut cases = Vec::new();
    for &t in &times {
        let mut u = Uniforms::defaults(16.0 / 9.0, [1920.0, 1080.0]);
        u.time = t;
        for &id in &ids {
            let s = cpu::parametric_waves_sample(id, &u);
            cases.push(json!({
                "id": id,
                "time": t,
                "clip": [s[0], s[1]],
                "pointSizePx": s[2],
                "color": [s[3], s[4], s[5], s[6]],
            }));
        }
    }
    json!({ "contractVersion": 1, "engine": "particle-v1", "model": "parametric-waves", "cases": cases })
}

#[test]
fn cpu_reference_matches_committed_fixture() {
    let path = fixture_path();
    if std::env::var("GENERATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&reference_rows()).unwrap()))
            .unwrap();
        println!("fixture written to {}", path.display());
        return;
    }
    let committed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let expected = reference_rows();
    let expected_cases = expected["cases"].as_array().unwrap();
    let committed_cases = committed["cases"].as_array().unwrap();
    assert_eq!(expected_cases.len(), committed_cases.len());
    for (got, want) in expected_cases.iter().zip(committed_cases) {
        assert_eq!(got["id"], want["id"], "id order changed");
        assert_eq!(got["time"], want["time"]);
        for key in ["clip", "color"] {
            for (g, w) in got[key].as_array().unwrap().iter().zip(want[key].as_array().unwrap()) {
                let (g, w) = (g.as_f64().unwrap(), w.as_f64().unwrap());
                assert!(
                    (g - w).abs() <= w.abs().max(1.0) * 1e-6,
                    "fixture drift at id {} time {} {key}: {g} vs {w}",
                    got["id"], got["time"]
                );
            }
        }
        let (g, w) = (
            got["pointSizePx"].as_f64().unwrap(),
            want["pointSizePx"].as_f64().unwrap(),
        );
        assert!((g - w).abs() <= 1e-6, "pointSizePx drift at id {}", got["id"]);
    }
}

#[test]
fn gpu_wgsl_matches_cpu_reference() {
    // Compute pass runs the same particleSample function from the shared
    // WGSL and writes raw results; readback is compared with the CPU
    // reference inside f32 rounding tolerance.
    let wgsl = crate::engine_wgsl();
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .expect("no GPU adapter available");
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor::default(),
    ))
    .expect("GPU device");

    let mut module_src = wgsl;
    module_src.push_str(
        r#"
struct Result { clip: vec2f, pointSizePx: f32, color: vec4f }
@group(0) @binding(1) var<storage, read_write> results: array<Result>;
@group(0) @binding(2) var<storage, read> ids: array<u32>;

@compute @workgroup_size(1)
fn parityMain(@builtin(global_invocation_id) gid: vec3u) {
    let s = particleSample(ids[gid.x], u);
    results[gid.x] = Result(s.clip, s.pointSizePx, s.color);
}
"#,
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("parity"),
        source: wgpu::ShaderSource::Wgsl(module_src.into()),
    });

    let ids: [u32; 6] = [0, 1, 234, 4999, 9998, 9999];
    let times: [f32; 4] = [0.0, 1.7, 13.3, 41.25];

    for &t in &times {
        let mut u = Uniforms::defaults(16.0 / 9.0, [1920.0, 1080.0]);
        u.time = t;
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uniforms"),
            contents: bytemuck::bytes_of(&u),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let out_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("results"),
            size: (ids.len() as u64) * 32, // vec2 + f32 + pad + vec4
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let ids_bytes: Vec<u8> = ids.iter().flat_map(|v| v.to_le_bytes()).collect();
        let ids_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ids"),
            contents: &ids_bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("parity"),
            layout: None,
            module: &module,
            entry_point: Some("parityMain"),
            compilation_options: Default::default(),
            cache: None,
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: out_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: ids_buf.as_entire_binding() },
            ],
        });

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(ids.len() as u32, 1, 1);
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
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
        queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        pollster::block_on(async {
            slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
            device.poll(wgpu::PollType::Wait).unwrap();
        });
        let data: Vec<f32> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
        drop(slice.get_mapped_range());

        for (i, &id) in ids.iter().enumerate() {
            let expected = cpu::parametric_waves_sample(id, &u);
            // Row layout: clip(2) + pointSize(1) + pad(1) + color(4).
            let mut row = data[i * 8..i * 8 + 8].to_vec();
            row.remove(3); // drop the padding slot
            for (g, w) in row.iter().zip(expected.iter()) {
                let diff = (g - w).abs();
                let tol = w.abs().max(1.0) * 1e-4;
                assert!(diff <= tol, "GPU/CPU divergence id={id} t={t}: {g} vs {w}");
            }
        }
    }
}
