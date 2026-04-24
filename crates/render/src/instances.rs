//! Feature-instance renderer (Phase 3.6.3c.5c.1).
//!
//! Draws a single primitive mesh (a unit box) replicated per-instance
//! from a GPU storage buffer. Per-instance data = position, rotation
//! around world Y, per-axis scale, baked color — computed at bake time
//! (crate `flowplan-world`, `feature_instances`) and streamed to the
//! renderer by `game_app`'s active-cell-set loop.
//!
//! # Pipeline overview
//!
//! - Vertex buffer: one primitive mesh's worth of vertices (box = 24
//!   verts) uploaded once.
//! - Index buffer: one primitive mesh's worth of indices (36 for the
//!   box) uploaded once.
//! - Storage buffer: `InstanceGpuData` per instance. Grows as needed
//!   via `upload`. Bound to `@group(1) @binding(0)`.
//! - Draw call: `draw_indexed(mesh.indices, 0, 0..instance_count)`.
//!   Vertex shader reads `instance_id = @builtin(instance_index)` and
//!   indexes into `instances[instance_id]`.
//!
//! # Incremental active-set updates (5c.1 MVP)
//!
//! 5c.1 uploads the full active-set buffer whenever it changes (active
//! set churns slowly — roughly once per rebuild event, not per frame).
//! Upload is 200 × 48 B = 9.6 KB at upper bound, which is negligible.
//! Incremental `write_buffer` at per-cell offsets is an optimization
//! reserved for 5c.2 when the total instance count rises.

#![deny(unsafe_code)]

use bytemuck::{Pod, Zeroable};
use flowplan_assets::{CpuMesh, Vertex};
use wgpu::util::DeviceExt;

use crate::renderer::DEPTH_FORMAT;

/// GPU-side per-instance layout — matches `struct InstanceData` in
/// `instances.wgsl`. 48 B per instance.
///
/// `repr(C)` + `Pod` guarantees a predictable layout we can
/// `bytemuck::cast_slice` straight into a storage buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct InstanceGpuData {
    /// World-space x, y, z + rotation around Y (radians) in .w.
    pub pos_rot: [f32; 4],
    /// Per-axis scale in xyz; w reserved.
    pub scale: [f32; 4],
    /// RGBA in [0, 1] (palette-adapted, baked at world-gen time).
    pub color: [f32; 4],
}

impl InstanceGpuData {
    /// Build GPU layout from the CPU-side `FeatureInstance` fields.
    ///
    /// Color is decoded from `u8[4]` → `f32[4] / 255.0`.
    #[must_use]
    pub fn from_fields(
        position: [f32; 3],
        rotation_y_rad: f32,
        scale: [f32; 3],
        color: [u8; 4],
    ) -> Self {
        Self {
            pos_rot: [position[0], position[1], position[2], rotation_y_rad],
            scale: [scale[0], scale[1], scale[2], 0.0],
            color: [
                f32::from(color[0]) / 255.0,
                f32::from(color[1]) / 255.0,
                f32::from(color[2]) / 255.0,
                f32::from(color[3]) / 255.0,
            ],
        }
    }
}

/// Renderer for instanced feature primitives. Owns the primitive mesh
/// buffers + the growable instance storage buffer + the pipeline.
pub struct InstanceRenderer {
    pipeline: wgpu::RenderPipeline,
    // Primitive mesh (unit box).
    vbo: wgpu::Buffer,
    ibo: wgpu::Buffer,
    index_count: u32,

    // Instance storage buffer + bind group.
    instance_bgl: wgpu::BindGroupLayout,
    instance_buf: wgpu::Buffer,
    instance_bg: wgpu::BindGroup,
    /// Capacity of the storage buffer in instances (not bytes). Grows
    /// via `upload` when an upload exceeds it.
    capacity: u32,
    /// Number of instances currently resident. Drives the `draw_indexed`
    /// instance range.
    instance_count: u32,
}

/// Initial capacity in instances. Sized to comfortably hold the 5c.1
/// active-set upper bound (~25 cells × ~8 houses). Doubled on overflow.
const INITIAL_CAPACITY: u32 = 512;

/// Baked-in WGSL source; exposed to lib-test so the shader-presence
/// sanity tests can keep the stale-build failure mode out of the build.
pub const INSTANCES_WGSL_SOURCE: &str = include_str!("instances.wgsl");

impl InstanceRenderer {
    /// Construct the renderer: compile shader, build pipeline, upload
    /// the primitive mesh, allocate empty instance storage buffer.
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        camera_bgl: &wgpu::BindGroupLayout,
        scene_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        primitive_mesh: &CpuMesh,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowplan-instances-shader"),
            source: wgpu::ShaderSource::Wgsl(INSTANCES_WGSL_SOURCE.into()),
        });

        let instance_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instance-storage-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        core::mem::size_of::<InstanceGpuData>() as u64,
                    ),
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("instance-pipeline-layout"),
            bind_group_layouts: &[camera_bgl, &instance_bgl, scene_bgl],
            push_constant_ranges: &[],
        });

        let pipeline = build_instance_pipeline(device, &pipeline_layout, &shader, surface_format);

        // Upload the primitive mesh.
        let vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{}-vbo", primitive_mesh.label)),
            contents: bytemuck::cast_slice(&primitive_mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{}-ibo", primitive_mesh.label)),
            contents: bytemuck::cast_slice(&primitive_mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let index_count = primitive_mesh.indices.len() as u32;

        let (instance_buf, instance_bg) =
            create_instance_storage(device, &instance_bgl, INITIAL_CAPACITY);

        Self {
            pipeline,
            vbo,
            ibo,
            index_count,
            instance_bgl,
            instance_buf,
            instance_bg,
            capacity: INITIAL_CAPACITY,
            instance_count: 0,
        }
    }

    /// Replace the entire active-set instance list. Grows the storage
    /// buffer if needed (doubles capacity until it fits). Uploads the
    /// full buffer in one `queue.write_buffer` call.
    ///
    /// 5c.1 MVP: called by `game_app` when the active cell set changes.
    /// 5c.2+ may add `upload_range` for incremental cell swap-in/out.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[InstanceGpuData]) {
        let need = data.len() as u32;
        if need > self.capacity {
            let mut new_cap = self.capacity.max(1);
            while new_cap < need {
                new_cap = new_cap.saturating_mul(2);
            }
            let (buf, bg) = create_instance_storage(device, &self.instance_bgl, new_cap);
            self.instance_buf = buf;
            self.instance_bg = bg;
            self.capacity = new_cap;
        }
        if !data.is_empty() {
            queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(data));
        }
        self.instance_count = need;
    }

    /// Number of live instances in the buffer (drives the draw call).
    #[must_use]
    pub const fn instance_count(&self) -> u32 {
        self.instance_count
    }

    /// Current storage-buffer capacity (instances, not bytes).
    #[must_use]
    pub const fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Record the instance draw into the scene pass. Binds camera (0),
    /// instance storage (1), scene (2), then issues one indexed draw
    /// with `0..instance_count` as the instance range.
    ///
    /// No-op when `instance_count == 0`.
    pub fn record<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera_bg: &'a wgpu::BindGroup,
        scene_bg: &'a wgpu::BindGroup,
    ) {
        if self.instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bg, &[]);
        pass.set_bind_group(1, &self.instance_bg, &[]);
        pass.set_bind_group(2, scene_bg, &[]);
        pass.set_vertex_buffer(0, self.vbo.slice(..));
        pass.set_index_buffer(self.ibo.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..self.instance_count);
    }
}

/// Allocate a storage buffer + matching bind group for `capacity`
/// instances. Used at construction and on buffer growth.
fn create_instance_storage(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    capacity: u32,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let byte_len = u64::from(capacity) * core::mem::size_of::<InstanceGpuData>() as u64;
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instance-storage-buf"),
        size: byte_len,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("instance-storage-bg"),
        layout: bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buf.as_entire_binding(),
        }],
    });
    (buf, bg)
}

fn build_instance_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let vertex_attrs = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 12,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 24,
            shader_location: 2,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: 32,
            shader_location: 3,
        },
    ];
    let vertex_buffers = [wgpu::VertexBufferLayout {
        array_stride: core::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &vertex_attrs,
    }];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("instance-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_instance",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &vertex_buffers,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "fs_instance",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            // `CpuMesh::cube` (source of `primitive_box`) has mixed
            // face winding — some triangles are CCW, others CW. The
            // main scene pipeline also disables culling for this
            // reason. Re-enable once primitives get authored-mesh
            // replacements with consistent winding (5c.5+).
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}
