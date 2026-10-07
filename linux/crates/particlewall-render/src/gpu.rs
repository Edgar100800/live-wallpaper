//! wgpu presenter for particle-v1 modules. Renders into a native Wayland
//! surface (Vulkan) obtained from a GTK layer window.

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

pub struct GpuPresenter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    flow_pipeline: wgpu::ComputePipeline,
    graph_pos_pipeline: wgpu::ComputePipeline,
    graph_conn_pipeline: wgpu::ComputePipeline,
    uniforms_buf: wgpu::Buffer,
    /// Group 0: uniform + flowParticles ro + flowHistory ro + graphEdges ro.
    bind_group: wgpu::BindGroup,
    graph_bgl: wgpu::BindGroupLayout,
    /// vec4 slots: primes for model 5, live state for model 4, dummy else.
    flow_particles: wgpu::Buffer,
    flow_history: wgpu::Buffer,
    graph_positions: wgpu::Buffer,
    graph_edges: wgpu::Buffer,
    /// Per-step flow params; queue ordering makes reuse across frames safe.
    flow_params: Vec<wgpu::Buffer>,
    flow_bind_groups: Vec<wgpu::BindGroup>,
    flow_frame: u32,
    flow_step_accumulator: f32,
    model: u32,
    pub config: wgpu::SurfaceConfiguration,
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

/// Unsafe pointer pair from a GDK Wayland surface/display.
pub struct WaylandHandles {
    pub display: *mut std::ffi::c_void,
    pub surface: *mut std::ffi::c_void,
}

impl GpuPresenter {
    pub fn new(handles: WaylandHandles, size: (u32, u32), model: u32) -> Result<Self, GpuError> {
        use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..Default::default()
        });
        let display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            std::ptr::NonNull::new(handles.display).ok_or(GpuError::Configure("null display".into()))?,
        ));
        let window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
            std::ptr::NonNull::new(handles.surface).ok_or(GpuError::Configure("null surface".into()))?,
        ));
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: display_handle,
            raw_window_handle: window_handle,
        };
        let surface = unsafe {
            instance
                .create_surface_unsafe(target)
                .map_err(GpuError::Surface)?
        };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|_| GpuError::NoAdapter)?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("particlewall-gpu"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
                trace: Default::default(),
            },
        ))
        .map_err(GpuError::NoDevice)?;

        // Surface/swapchain failures must not kill the daemon: log them and
        // let the caller fall back to the web renderer.
        device.on_uncaptured_error(Box::new(|e| eprintln!("pw: wgpu error: {e}")));

        let caps = surface.get_capabilities(&adapter);
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
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Probe the swapchain: Wayland-level failures (e.g. fifo conflicts,
        // protocol errors) surface on the first acquire, not in configure.
        let probe = surface
            .get_current_texture()
            .map_err(|e| GpuError::Configure(format!("swapchain unavailable: {e}")))?;
        probe.present();

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle-v1"),
            source: wgpu::ShaderSource::Wgsl(crate::engine_wgsl().into()),
        });

        // ---- bind group layouts -------------------------------------------
        let compute_vertex = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::COMPUTE;
        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("engine group0"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: compute_vertex,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: compute_vertex,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: compute_vertex,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let flow_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("engine flow"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let graph_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("engine graph"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // ---- pipelines ------------------------------------------------------
        let particle_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bgl0],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle-v1-points"),
            layout: Some(&particle_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vsMain"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fsMain"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle-v1-graph-lines"),
            layout: Some(&particle_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vsLine"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fsLine"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let flow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bgl0, &flow_bgl],
            push_constant_ranges: &[],
        });
        let flow_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particle-v1-flow"),
            layout: Some(&flow_layout),
            module: &module,
            entry_point: Some("flowUpdate"),
            compilation_options: Default::default(),
            cache: None,
        });
        let graph_pos_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bgl0, &flow_bgl, &graph_bgl],
            push_constant_ranges: &[],
        });
        let graph_pos_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particle-v1-graph-positions"),
            layout: Some(&graph_pos_layout),
            module: &module,
            entry_point: Some("graphPositionUpdate"),
            compilation_options: Default::default(),
            cache: None,
        });
        let graph_conn_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particle-v1-graph-connections"),
            layout: Some(&graph_pos_layout),
            module: &module,
            entry_point: Some("graphConnectionUpdate"),
            compilation_options: Default::default(),
            cache: None,
        });

        // ---- buffers ---------------------------------------------------------
        let uniforms_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (flow_particles, flow_history) = create_flow_buffers(&device);
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

        initialize_flow_buffers(&queue, &flow_particles, &flow_history, model);

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("engine group0 bind"),
            layout: &bgl0,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniforms_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: flow_particles.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: flow_history.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: graph_edges.as_entire_binding() },
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
                    layout: &flow_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: flow_particles.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: flow_history.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                    ],
                })
            })
            .collect();

        Ok(Self {
            device,
            queue,
            surface,
            pipeline,
            line_pipeline,
            flow_pipeline,
            graph_pos_pipeline,
            graph_conn_pipeline,
            uniforms_buf,
            bind_group,
            graph_bgl,
            flow_particles,
            flow_history,
            graph_positions,
            graph_edges,
            flow_params,
            flow_bind_groups,
            flow_frame: 0,
            flow_step_accumulator: 0.0,
            model,
            config,
        })
    }

    pub fn resize(&mut self, size: (u32, u32)) {
        self.config.width = size.0.max(1);
        self.config.height = size.1.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Last written history slot for Lluvia de Ruido (u.model.y).
    pub fn latest_flow_slot(&self) -> f32 {
        if self.flow_frame == 0 {
            0.0
        } else {
            ((self.flow_frame - 1) % FLOW_HISTORY_COUNT) as f32
        }
    }

    /// Reconfigures the presenter for another particle-v1 model without
    /// touching the swapchain/device: the NVIDIA Wayland WSI corrupts state
    /// when a second Vulkan swapchain is created on the same connection, so
    /// presenters are permanent per-output resources whose model switches
    /// by rewriting their state buffers only.
    pub fn set_model(&mut self, model: u32) {
        if self.model == model {
            return;
        }
        self.model = model;
        self.flow_frame = 0;
        self.flow_step_accumulator = 0.0;
        initialize_flow_buffers(&self.queue, &self.flow_particles, &self.flow_history, model);
    }

    pub fn model(&self) -> u32 {
        self.model
    }

    /// Renders one frame. `background` is the clear color (linear 0..1);
    /// `delta` is the ALREADY-SPEED-SCALED frame delta (the Swift renderer
    /// feeds delta*speed to both the clock and the flow accumulator).
    pub fn render(&mut self, u: &Uniforms, background: [f64; 4], delta: f32) {
        self.queue.write_buffer(&self.uniforms_buf, 0, bytemuck::bytes_of(u));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });

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
                pass.set_pipeline(&self.flow_pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                for step in 0..step_count {
                    let params: [u32; 4] = [
                        self.flow_frame,
                        0,
                        FLOW_PARTICLE_COUNT,
                        FLOW_HISTORY_COUNT,
                    ];
                    self.queue.write_buffer(
                        &self.flow_params[step],
                        0,
                        bytemuck::bytes_of(&params),
                    );
                    pass.set_bind_group(1, &self.flow_bind_groups[step], &[]);
                    pass.dispatch_workgroups(FLOW_PARTICLE_COUNT.div_ceil(64), 1, 1);
                    self.flow_frame = self.flow_frame.wrapping_add(1);
                }
            }
        }

        // Graph overlay: sample nodes, connect, then draw the edge list.
        let graph_enabled = u.graph[0] >= 0.5;
        if graph_enabled {
            let gu = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.graph_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.uniforms_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: self.graph_positions.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: self.graph_edges.as_entire_binding() },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("graph"),
                ..Default::default()
            });
            pass.set_pipeline(&self.graph_pos_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &self.bind_group, &[]);
            pass.set_bind_group(2, &gu, &[]);
            pass.dispatch_workgroups(GRAPH_NODE_COUNT.div_ceil(64), 1, 1);
            pass.set_pipeline(&self.graph_conn_pipeline);
            pass.set_bind_group(2, &gu, &[]);
            pass.dispatch_workgroups(GRAPH_NODE_COUNT.div_ceil(64), 1, 1);
        }

        let frame = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(_) => {
                self.surface.configure(&self.device, &self.config);
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
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("particles"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            if graph_enabled {
                pass.set_pipeline(&self.line_pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..(GRAPH_NODE_COUNT * GRAPH_MAX_CONNECTIONS * 2), 0..1);
            }
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            let instances = u.model[3] as u32;
            pass.draw(0..instances * 6, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}

// SAFETY: the raw Wayland pointers are owned by GTK and outlive the presenter
// (the daemon keeps the Gtk window alive for the presenter's lifetime).
unsafe impl Send for WaylandHandles {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_switch_uploads_fit_the_original_buffers() {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions::default(),
        )).expect("GPU adapter required for the buffer regression test");
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor::default(),
        )).unwrap();
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (particles, history) = create_flow_buffers(&device);
        // Same allocations throughout, including starting in a non-prime model.
        for model in [9, 5, 4, 5] {
            initialize_flow_buffers(&queue, &particles, &history, model);
            queue.submit([]);
            device.poll(wgpu::PollType::Wait).unwrap();
        }
        assert!(pollster::block_on(device.pop_error_scope()).is_none());
    }
}
