//! wgpu presenter for particle-v1 modules. Renders into a native Wayland
//! surface (Vulkan) obtained from a GTK layer window.

use crate::Uniforms;

pub struct GpuPresenter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    pipeline: wgpu::RenderPipeline,
    uniforms_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
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
    pub fn new(handles: WaylandHandles, size: (u32, u32)) -> Result<Self, GpuError> {
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&device.create_bind_group_layout(
                &wgpu::BindGroupLayoutDescriptor {
                    label: Some("uniforms"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                },
            )],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle-v1-points"),
            layout: Some(&pipeline_layout),
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

        let uniforms_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms_buf.as_entire_binding(),
            }],
        });

        Ok(Self {
            device,
            queue,
            surface,
            pipeline,
            uniforms_buf,
            bind_group,
            config,
        })
    }

    pub fn resize(&mut self, size: (u32, u32)) {
        self.config.width = size.0.max(1);
        self.config.height = size.1.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Renders one frame. `background` is the clear color (linear 0..1).
    pub fn render(&mut self, u: &Uniforms, background: [f64; 4]) {
        self.queue.write_buffer(
            &self.uniforms_buf,
            0,
            bytemuck::bytes_of(u),
        );
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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..Uniforms::VERTEX_COUNT_MODEL0 * 6, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}

// SAFETY: the raw Wayland pointers are owned by GTK and outlive the presenter
// (the daemon keeps the Gtk window alive for the presenter's lifetime).
unsafe impl Send for WaylandHandles {}
