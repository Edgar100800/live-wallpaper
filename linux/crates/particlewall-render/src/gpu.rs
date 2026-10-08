//! wgpu presenter for particle-v1 modules. Renders into a native Wayland
//! surface (Vulkan) obtained from a dedicated layer-shell connection.
//!
//! Split in three layers so outputs share one device and the frame encoding
//! runs offscreen in tests and benchmarks:
//!   - `GpuContext`: instance, adapter, device, shader and pipelines (one per
//!     process, shared by every output).
//!   - `ParticleRenderer`: per-output state buffers and bind groups; encodes a
//!     frame into any texture view.
//!   - `GpuPresenter`: a renderer bound to a swapchain.

use std::cell::RefCell;
use std::rc::Rc;

use crate::Uniforms;

/// Instance counts for the state buffers, mirroring the Swift renderer.
const FLOW_PARTICLE_COUNT: u32 = 720 * 9;
const FLOW_HISTORY_COUNT: u32 = 12;
/// Espiral Prima uploads the prime list through the same storage slot.
const PRIME_COUNT: u32 = 78_498;
const GRAPH_NODE_COUNT: u32 = 768;
const GRAPH_MAX_CONNECTIONS: u32 = 3;
/// Flow stepping runs at most 6 fixed steps per frame (Swift encodeFlowUpdates).
const MAX_FLOW_STEPS_PER_FRAME: usize = 6;
/// Particles per indexed draw. The index pattern repeats, so one 384 KiB
/// buffer serves every model through base_vertex offsets.
const INDEXED_BATCH: u32 = 16_384;

/// How particle quads reach the rasterizer. Both draw the same triangles in
/// the same order; they differ in how often particleSample runs.
///
/// Measured with `examples/bench.rs` (RTX 3060 Ti, 2560x1440), Indexed4 was
/// 1.07-1.69x faster than List6 on every model (Nebulosa 0.79 -> 0.50 ms).
/// Instanced 4-vertex strips lost on Espiral Prima (tiny instances), and
/// baking samples once per particle in a compute pass lost to its buffer
/// traffic, so neither is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexPath {
    /// `vsMain`: 6 vertices per particle, 6 particleSample calls. Kept as
    /// the macOS path and as the parity reference.
    List6,
    /// `vsIndexed`: indexed quads in batches; the post-transform cache shares
    /// the diagonal, so 4 particleSample calls per particle.
    Indexed4,
}

impl VertexPath {
    pub const ALL: [VertexPath; 2] = [VertexPath::List6, VertexPath::Indexed4];
}

// Presenters switch models without recreating bind groups. Reserve the largest
// storage requirement even when the initial model does not use the prime list.
fn create_flow_buffers(device: &wgpu::Device) -> (wgpu::Buffer, wgpu::Buffer) {
    let particles = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flow particles / primes"),
        size: u64::from(PRIME_COUNT.max(FLOW_PARTICLE_COUNT)) * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let history = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flow history"),
        size: u64::from(FLOW_PARTICLE_COUNT * FLOW_HISTORY_COUNT) * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    (particles, history)
}

fn initialize_flow_buffers(
    queue: &wgpu::Queue,
    particles: &wgpu::Buffer,
    history: &wgpu::Buffer,
    model: u32,
) {
    match model {
        4 => {
            let zeros = vec![[0.0f32; 4]; (FLOW_PARTICLE_COUNT * FLOW_HISTORY_COUNT) as usize];
            queue.write_buffer(history, 0, bytemuck::cast_slice(&zeros));
            let zeros = vec![[0.0f32; 4]; FLOW_PARTICLE_COUNT as usize];
            queue.write_buffer(particles, 0, bytemuck::cast_slice(&zeros));
        }
        5 => {
            let primes = crate::cpu::generate_primes(crate::cpu::PRIME_LIMIT);
            let data: Vec<[f32; 4]> = primes.iter().map(|p| [*p, 0.0, 0.0, 0.0]).collect();
            queue.write_buffer(particles, 0, bytemuck::cast_slice(&data));
        }
        _ => {}
    }
}

#[derive(Debug)]
pub enum GpuError {
    NoAdapter,
    NoDevice(wgpu::RequestDeviceError),
    Surface(wgpu::CreateSurfaceError),
    Configure(String),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "no Vulkan/GL adapter available"),
            Self::NoDevice(e) => write!(f, "GPU device error: {e}"),
            Self::Surface(e) => write!(f, "surface error: {e}"),
            Self::Configure(s) => write!(f, "configure error: {s}"),
        }
    }
}

/// Unsafe pointer pair from a Wayland surface/display.
pub struct WaylandHandles {
    pub display: *mut std::ffi::c_void,
    pub surface: *mut std::ffi::c_void,
}

// SAFETY: the raw Wayland pointers are owned by the daemon's dedicated
// connection and outlive the presenter (GpuOutput drops the presenter first).
unsafe impl Send for WaylandHandles {}

fn storage_entry(binding: u32, visibility: wgpu::ShaderStages, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Render pipelines depend on the target format; outputs normally share one.
struct RenderPipelines {
    format: wgpu::TextureFormat,
    list6: wgpu::RenderPipeline,
    indexed4: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
}

/// Device, shader and pipelines shared by every output of the process.
///
/// Bind group layouts never expose the same buffer twice to one dispatch:
/// wgpu rejects a buffer bound read-only and read-write in the same usage
/// scope, so compute pipelines leave unused groups empty (as the parity
/// tests do) instead of reusing the render group 0.
pub struct GpuContext {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    module: wgpu::ShaderModule,
    /// Render group 0: uniforms + flowParticles/flowHistory/graphEdges ro.
    render_bgl0: wgpu::BindGroupLayout,
    /// Compute group 0: uniforms + flowParticles/flowHistory ro (no edges).
    compute_bgl0: wgpu::BindGroupLayout,
    flow_bgl: wgpu::BindGroupLayout,
    graph_bgl: wgpu::BindGroupLayout,
    empty_bind_group: wgpu::BindGroup,
    render_layout: wgpu::PipelineLayout,
    flow_pipeline: wgpu::ComputePipeline,
    graph_pos_pipeline: wgpu::ComputePipeline,
    graph_conn_pipeline: wgpu::ComputePipeline,
    /// Quad index pattern for INDEXED_BATCH particles (VertexPath::Indexed4).
    quad_indices: wgpu::Buffer,
    render_pipelines: RefCell<Vec<Rc<RenderPipelines>>>,
}

impl GpuContext {
    /// Context whose adapter can present to `surface` (created from
    /// `instance`). The surface is returned to the caller untouched.
    pub fn for_surface(
        instance: wgpu::Instance,
        surface: &wgpu::Surface<'_>,
    ) -> Result<Rc<Self>, GpuError> {
        Self::build(instance, Some(surface), wgpu::Features::empty())
    }

    /// Offscreen context for tests and benchmarks. `features` lets the
    /// benchmark request timestamp queries.
    pub fn headless(features: wgpu::Features) -> Result<Rc<Self>, GpuError> {
        Self::build(Self::new_instance(), None, features)
    }

    pub fn new_instance() -> wgpu::Instance {
        wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..Default::default()
        })
    }

    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }

    fn build(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        features: wgpu::Features,
    ) -> Result<Rc<Self>, GpuError> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: surface,
            force_fallback_adapter: false,
        }))
        .map_err(|_| GpuError::NoAdapter)?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("particlewall-gpu"),
            required_features: features,
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: Default::default(),
        }))
        .map_err(GpuError::NoDevice)?;

        // Surface/swapchain failures must not kill the daemon: log them and
        // let the caller fall back to the web renderer.
        device.on_uncaptured_error(Box::new(|e| eprintln!("pw: wgpu error: {e}")));

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle-v1"),
            source: wgpu::ShaderSource::Wgsl(crate::ENGINE_WGSL.into()),
        });

        let vertex = wgpu::ShaderStages::VERTEX;
        let compute = wgpu::ShaderStages::COMPUTE;
        let render_bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("engine render group0"),
            entries: &[
                uniform_entry(0, vertex),
                storage_entry(1, vertex, true),
                storage_entry(2, vertex, true),
                storage_entry(3, vertex, true),
            ],
        });
        let compute_bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("engine compute group0"),
            entries: &[
                uniform_entry(0, compute),
                storage_entry(1, compute, true),
                storage_entry(2, compute, true),
            ],
        });
        let flow_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("engine flow"),
            entries: &[
                storage_entry(0, compute, false),
                storage_entry(1, compute, false),
                uniform_entry(2, compute),
            ],
        });
        let graph_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("engine graph"),
            entries: &[
                uniform_entry(0, compute),
                storage_entry(1, compute, false),
                storage_entry(2, compute, false),
            ],
        });
        let empty_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("engine empty"),
            entries: &[],
        });
        let empty_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("engine empty"),
            layout: &empty_bgl,
            entries: &[],
        });

        let layout = |label: &str, groups: &[&wgpu::BindGroupLayout]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                push_constant_ranges: &[],
            })
        };
        let render_layout = layout("render", &[&render_bgl0]);
        let flow_layout = layout("flow", &[&empty_bgl, &flow_bgl]);
        let graph_pos_layout = layout("graph positions", &[&compute_bgl0, &empty_bgl, &graph_bgl]);
        let graph_conn_layout = layout("graph connections", &[&empty_bgl, &empty_bgl, &graph_bgl]);

        let compute_pipeline = |label: &str, layout: &wgpu::PipelineLayout, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let flow_pipeline = compute_pipeline("particle-v1-flow", &flow_layout, "flowUpdate");
        let graph_pos_pipeline =
            compute_pipeline("particle-v1-graph-positions", &graph_pos_layout, "graphPositionUpdate");
        let graph_conn_pipeline =
            compute_pipeline("particle-v1-graph-connections", &graph_conn_layout, "graphConnectionUpdate");

        let indices: Vec<u32> = (0..INDEXED_BATCH)
            .flat_map(|p| [0, 1, 2, 2, 1, 3].map(|corner| p * 4 + corner))
            .collect();
        let quad_indices = wgpu::util::DeviceExt::create_buffer_init(
            &device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("quad indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            },
        );

        Ok(Rc::new(Self {
            instance,
            adapter,
            device,
            queue,
            module,
            render_bgl0,
            compute_bgl0,
            flow_bgl,
            graph_bgl,
            empty_bind_group,
            render_layout,
            flow_pipeline,
            graph_pos_pipeline,
            graph_conn_pipeline,
            quad_indices,
            render_pipelines: RefCell::new(Vec::new()),
        }))
    }

    /// Creates a Wayland surface on this context's instance.
    ///
    /// # Safety
    /// `handles` must stay valid until the returned surface is dropped.
    pub unsafe fn create_wayland_surface(
        &self,
        handles: &WaylandHandles,
    ) -> Result<wgpu::Surface<'static>, GpuError> {
        create_wayland_surface(&self.instance, handles)
    }

    fn render_pipelines(&self, format: wgpu::TextureFormat) -> Rc<RenderPipelines> {
        if let Some(found) = self.render_pipelines.borrow().iter().find(|p| p.format == format) {
            return found.clone();
        }
        let pipeline = |label: &str,
                        layout: &wgpu::PipelineLayout,
                        vs: &str,
                        fs: &str,
                        topology: wgpu::PrimitiveTopology| {
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState { topology, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        use wgpu::PrimitiveTopology::{LineList, TriangleList};
        let layout = &self.render_layout;
        let pipelines = Rc::new(RenderPipelines {
            format,
            list6: pipeline("particle-v1-points", layout, "vsMain", "fsMain", TriangleList),
            indexed4: pipeline("particle-v1-indexed", layout, "vsIndexed", "fsMain", TriangleList),
            lines: pipeline("particle-v1-graph-lines", layout, "vsLine", "fsLine", LineList),
        });
        self.render_pipelines.borrow_mut().push(pipelines.clone());
        pipelines
    }
}

unsafe fn create_wayland_surface(
    instance: &wgpu::Instance,
    handles: &WaylandHandles,
) -> Result<wgpu::Surface<'static>, GpuError> {
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
    let display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
        std::ptr::NonNull::new(handles.display).ok_or(GpuError::Configure("null display".into()))?,
    ));
    let window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
        std::ptr::NonNull::new(handles.surface).ok_or(GpuError::Configure("null surface".into()))?,
    ));
    instance
        .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: display_handle,
            raw_window_handle: window_handle,
        })
        .map_err(GpuError::Surface)
}

/// Per-output engine state: uniforms, flow/prime storage, graph buffers and
/// the bind groups over them. Encodes a frame into any compatible view.
pub struct ParticleRenderer {
    ctx: Rc<GpuContext>,
    pipelines: Rc<RenderPipelines>,
    vertex_path: VertexPath,
    uniforms_buf: wgpu::Buffer,
    render_group0: wgpu::BindGroup,
    compute_group0: wgpu::BindGroup,
    graph_group: wgpu::BindGroup,
    /// vec4 slots: primes for model 5, live state for model 4, unused else.
    flow_particles: wgpu::Buffer,
    flow_history: wgpu::Buffer,
    /// Per-step flow params; queue ordering makes reuse across frames safe.
    flow_params: Vec<wgpu::Buffer>,
    flow_bind_groups: Vec<wgpu::BindGroup>,
    flow_frame: u32,
    flow_step_accumulator: f32,
    model: u32,
}

impl ParticleRenderer {
    pub fn new(
        ctx: &Rc<GpuContext>,
        format: wgpu::TextureFormat,
        model: u32,
        vertex_path: VertexPath,
    ) -> Self {
        let device = &ctx.device;
        let uniforms_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (flow_particles, flow_history) = create_flow_buffers(device);
        let graph_positions = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("graph positions"),
            size: (GRAPH_NODE_COUNT as u64) * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let graph_edges = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("graph edges"),
            size: (GRAPH_NODE_COUNT * GRAPH_MAX_CONNECTIONS * 2) as u64 * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        initialize_flow_buffers(&ctx.queue, &flow_particles, &flow_history, model);

        let render_group0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("engine render group0"),
            layout: &ctx.render_bgl0,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniforms_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: flow_particles.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: flow_history.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: graph_edges.as_entire_binding() },
            ],
        });
        let compute_group0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("engine compute group0"),
            layout: &ctx.compute_bgl0,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniforms_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: flow_particles.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: flow_history.as_entire_binding() },
            ],
        });
        let graph_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("engine graph"),
            layout: &ctx.graph_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniforms_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: graph_positions.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: graph_edges.as_entire_binding() },
            ],
        });

        let flow_params = (0..MAX_FLOW_STEPS_PER_FRAME)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("flow params"),
                    size: 16,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect::<Vec<_>>();
        // One bind group per flow step; only the params content changes per
        // frame (queue ordering guarantees writes land before execution).
        let flow_bind_groups = flow_params
            .iter()
            .map(|params| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("flow step"),
                    layout: &ctx.flow_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: flow_particles.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: flow_history.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                    ],
                })
            })
            .collect();

        Self {
            ctx: ctx.clone(),
            pipelines: ctx.render_pipelines(format),
            vertex_path,
            uniforms_buf,
            render_group0,
            compute_group0,
            graph_group,
            flow_particles,
            flow_history,
            flow_params,
            flow_bind_groups,
            flow_frame: 0,
            flow_step_accumulator: 0.0,
            model,
        }
    }

    pub fn vertex_path(&self) -> VertexPath {
        self.vertex_path
    }

    pub fn set_vertex_path(&mut self, path: VertexPath) {
        self.vertex_path = path;
    }

    /// Last written history slot for Lluvia de Ruido (u.model.y).
    pub fn latest_flow_slot(&self) -> f32 {
        if self.flow_frame == 0 {
            0.0
        } else {
            ((self.flow_frame - 1) % FLOW_HISTORY_COUNT) as f32
        }
    }

    /// Reconfigures the renderer for another particle-v1 model by rewriting
    /// its state buffers; the swapchain and device are untouched.
    pub fn set_model(&mut self, model: u32) {
        if self.model == model {
            return;
        }
        self.model = model;
        self.flow_frame = 0;
        self.flow_step_accumulator = 0.0;
        initialize_flow_buffers(&self.ctx.queue, &self.flow_particles, &self.flow_history, model);
    }

    pub fn model(&self) -> u32 {
        self.model
    }

    /// Records one frame into `encoder`, drawing into `view`. `delta` is the
    /// ALREADY-SPEED-SCALED frame delta (the Swift renderer feeds delta*speed
    /// to both the clock and the flow accumulator).
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        u: &Uniforms,
        background: [f64; 4],
        delta: f32,
    ) {
        let ctx = self.ctx.clone();
        ctx.queue.write_buffer(&self.uniforms_buf, 0, bytemuck::bytes_of(u));
        let instances = (u.model[3] as u32).min(Uniforms::model_vertex_count(self.model));

        // Fixed-step flow simulation (Lluvia de Ruido).
        if self.model == 4 {
            self.flow_step_accumulator += delta * 30.0;
            let step_count = (self.flow_step_accumulator as usize).min(MAX_FLOW_STEPS_PER_FRAME);
            if step_count > 0 {
                self.flow_step_accumulator -= step_count as f32;
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("flow"),
                    ..Default::default()
                });
                pass.set_pipeline(&ctx.flow_pipeline);
                pass.set_bind_group(0, &ctx.empty_bind_group, &[]);
                for step in 0..step_count {
                    let params: [u32; 4] = [self.flow_frame, 0, FLOW_PARTICLE_COUNT, FLOW_HISTORY_COUNT];
                    ctx.queue.write_buffer(&self.flow_params[step], 0, bytemuck::bytes_of(&params));
                    pass.set_bind_group(1, &self.flow_bind_groups[step], &[]);
                    pass.dispatch_workgroups(FLOW_PARTICLE_COUNT.div_ceil(64), 1, 1);
                    self.flow_frame = self.flow_frame.wrapping_add(1);
                }
            }
        }

        // Graph overlay: sample nodes, then connect them.
        let graph_enabled = u.graph[0] >= 0.5;
        if graph_enabled {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("graph"),
                ..Default::default()
            });
            pass.set_pipeline(&ctx.graph_pos_pipeline);
            pass.set_bind_group(0, &self.compute_group0, &[]);
            pass.set_bind_group(1, &ctx.empty_bind_group, &[]);
            pass.set_bind_group(2, &self.graph_group, &[]);
            pass.dispatch_workgroups(GRAPH_NODE_COUNT.div_ceil(64), 1, 1);
            pass.set_pipeline(&ctx.graph_conn_pipeline);
            pass.set_bind_group(0, &ctx.empty_bind_group, &[]);
            pass.dispatch_workgroups(GRAPH_NODE_COUNT.div_ceil(64), 1, 1);
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particles"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: background[0],
                        g: background[1],
                        b: background[2],
                        a: background[3],
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &self.render_group0, &[]);
        if graph_enabled {
            pass.set_pipeline(&self.pipelines.lines);
            pass.draw(0..(GRAPH_NODE_COUNT * GRAPH_MAX_CONNECTIONS * 2), 0..1);
        }
        match self.vertex_path {
            VertexPath::Indexed4 => {
                pass.set_pipeline(&self.pipelines.indexed4);
                pass.set_index_buffer(ctx.quad_indices.slice(..), wgpu::IndexFormat::Uint32);
                let mut first = 0;
                while first < instances {
                    let count = (instances - first).min(INDEXED_BATCH);
                    pass.draw_indexed(0..count * 6, (first * 4) as i32, 0..1);
                    first += count;
                }
            }
            VertexPath::List6 => {
                pass.set_pipeline(&self.pipelines.list6);
                pass.draw(0..instances * 6, 0..1);
            }
        }
    }
}

/// A `ParticleRenderer` presenting to one Wayland swapchain.
pub struct GpuPresenter {
    // Field order matters: the renderer and surface drop before the context
    // that owns the instance/device.
    renderer: ParticleRenderer,
    surface: wgpu::Surface<'static>,
    ctx: Rc<GpuContext>,
    pub config: wgpu::SurfaceConfiguration,
}

impl GpuPresenter {
    /// Creates a presenter on `handles`, reusing the process-wide context in
    /// `shared` (created on first use so the adapter matches the surface).
    pub fn new(
        shared: &mut Option<Rc<GpuContext>>,
        handles: WaylandHandles,
        size: (u32, u32),
        model: u32,
    ) -> Result<Self, GpuError> {
        let (ctx, surface) = match shared {
            Some(ctx) => {
                let surface = unsafe { ctx.create_wayland_surface(&handles)? };
                if !ctx.adapter.is_surface_supported(&surface) {
                    return Err(GpuError::Configure("output not supported by the shared adapter".into()));
                }
                (ctx.clone(), surface)
            }
            None => {
                let instance = GpuContext::new_instance();
                let surface = unsafe { create_wayland_surface(&instance, &handles)? };
                let ctx = GpuContext::for_surface(instance, &surface)?;
                *shared = Some(ctx.clone());
                (ctx, surface)
            }
        };

        let caps = surface.get_capabilities(&ctx.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f == &wgpu::TextureFormat::Bgra8Unorm)
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.0.max(1),
            height: size.1.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            // The wallpaper submits at most ~60 frames/s against a faster
            // display, so one frame in flight never starves the GPU and
            // the swapchain needs one image less (8-15 MB per output).
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&ctx.device, &config);

        // Probe the swapchain: Wayland-level failures (e.g. fifo conflicts,
        // protocol errors) surface on the first acquire, not in configure.
        let probe = surface
            .get_current_texture()
            .map_err(|e| GpuError::Configure(format!("swapchain unavailable: {e}")))?;
        probe.present();

        let renderer = ParticleRenderer::new(&ctx, format, model, VertexPath::Indexed4);
        Ok(Self { renderer, surface, ctx, config })
    }

    pub fn resize(&mut self, size: (u32, u32)) {
        self.config.width = size.0.max(1);
        self.config.height = size.1.max(1);
        self.surface.configure(&self.ctx.device, &self.config);
    }

    pub fn latest_flow_slot(&self) -> f32 {
        self.renderer.latest_flow_slot()
    }

    /// See `ParticleRenderer::set_model`. Presenters are permanent per-output
    /// resources: the NVIDIA Wayland WSI corrupts state when a second Vulkan
    /// swapchain is created on the same connection.
    pub fn set_model(&mut self, model: u32) {
        self.renderer.set_model(model);
    }

    pub fn model(&self) -> u32 {
        self.renderer.model()
    }

    /// Renders and presents one frame. The swapchain image is acquired
    /// before any work is recorded, so a skipped frame leaves the flow
    /// simulation untouched.
    pub fn render(&mut self, u: &Uniforms, background: [f64; 4], delta: f32) {
        let frame = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(_) => {
                self.surface.configure(&self.ctx.device, &self.config);
                match self.surface.get_current_texture() {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("pw: GPU frame skipped ({e})");
                        return;
                    }
                }
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        self.renderer.encode(&mut encoder, &view, u, background, delta);
        self.ctx.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_switch_uploads_fit_the_original_buffers() {
        let ctx = GpuContext::headless(wgpu::Features::empty())
            .expect("GPU adapter required for the buffer regression test");
        let (device, queue) = (&ctx.device, &ctx.queue);
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (particles, history) = create_flow_buffers(device);
        // Same allocations throughout, including starting in a non-prime model.
        for model in [9, 5, 4, 5] {
            initialize_flow_buffers(queue, &particles, &history, model);
            queue.submit([]);
            device.poll(wgpu::PollType::Wait).unwrap();
        }
        assert!(pollster::block_on(device.pop_error_scope()).is_none());
    }

    const TEST_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
    const TEST_SIZE: (u32, u32) = (640, 360);

    fn test_target(ctx: &GpuContext) -> wgpu::Texture {
        ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test target"),
            size: wgpu::Extent3d { width: TEST_SIZE.0, height: TEST_SIZE.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEST_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn test_uniforms(time: f32, graph: bool) -> Uniforms {
        let mut u = Uniforms::defaults(
            TEST_SIZE.0 as f32 / TEST_SIZE.1 as f32,
            [TEST_SIZE.0 as f32, TEST_SIZE.1 as f32],
        );
        u.time = time;
        if graph {
            u.graph = [1.0, 0.085, 1.0, 3.0];
        }
        u
    }

    /// Renders `frames` frames of `model` and returns the last one's pixels.
    fn render_frames(ctx: &Rc<GpuContext>, model: u32, path: VertexPath, graph: bool, frames: u32) -> Vec<u8> {
        let target = test_target(ctx);
        let view = target.create_view(&Default::default());
        let mut renderer = ParticleRenderer::new(ctx, TEST_FORMAT, model, path);
        for frame in 0..frames {
            let mut u = test_uniforms(13.3 + frame as f32 / 30.0, graph);
            u.model = [
                model as f32,
                renderer.latest_flow_slot(),
                12.0,
                Uniforms::model_vertex_count(model) as f32,
            ];
            let mut encoder = ctx.device.create_command_encoder(&Default::default());
            renderer.encode(&mut encoder, &view, &u, [0.02, 0.03, 0.08, 1.0], 1.0 / 30.0);
            ctx.queue.submit(Some(encoder.finish()));
        }
        let row = TEST_SIZE.0 * 4;
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * TEST_SIZE.1),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            target.size(),
        );
        ctx.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map pixels"));
        ctx.device.poll(wgpu::PollType::Wait).unwrap();
        let pixels = slice.get_mapped_range().to_vec();
        pixels
    }

    /// Exercises the real presenter passes (flow steps, graph compute, both
    /// vertex paths) for every model: buffers bound read-only and
    /// read-write in one dispatch invalidate the whole frame in wgpu.
    #[test]
    fn every_model_encodes_without_validation_errors() {
        let ctx = GpuContext::headless(wgpu::Features::empty()).expect("GPU adapter");
        for model in 0..12 {
            for graph in [false, true] {
                for path in VertexPath::ALL {
                    ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
                    render_frames(&ctx, model, path, graph, 3);
                    let error = pollster::block_on(ctx.device.pop_error_scope());
                    assert!(error.is_none(), "model {model} graph {graph} {path:?}: {error:?}");
                }
            }
        }
    }

    /// Indexed4 draws the same triangles in the same order as vsMain, so
    /// blending must produce the same image.
    #[test]
    fn indexed_quads_match_list6_pixels() {
        let ctx = GpuContext::headless(wgpu::Features::empty()).expect("GPU adapter");
        for model in 0..12 {
            // Lluvia de Ruido spawns 9 particles per fixed step.
            let frames = if model == 4 { 90 } else { 3 };
            let reference = render_frames(&ctx, model, VertexPath::List6, true, frames);
            let indexed = render_frames(&ctx, model, VertexPath::Indexed4, true, frames);
            let lit = reference.chunks(4).filter(|p| p[..3] != [20, 8, 5]).count();
            let max_diff = reference
                .iter()
                .zip(&indexed)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(lit > 100, "model {model} rendered almost nothing ({lit} lit pixels)");
            assert!(max_diff <= 1, "model {model}: max channel difference {max_diff}");
        }
    }
}
