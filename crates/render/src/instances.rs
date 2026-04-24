//! Feature-instance renderer (Phase 3.6.3c.5c.1 + 5c.2.a).
//!
//! Draws multiple primitive meshes (box / cone / cylinder / sphere)
//! replicated per-instance from per-primitive GPU storage buffers.
//! Per-instance data = position, rotation around world Y, per-axis
//! scale, baked color — computed at bake time (crate `flowplan-world`,
//! `feature_instances`) and streamed to the renderer by `game_app`'s
//! active-cell-set loop.
//!
//! # Pipeline overview
//!
//! One `wgpu::RenderPipeline` shared across primitives (vertex layout +
//! shader are identical). Per primitive kind, the renderer owns:
//!
//! - Vertex buffer: mesh verts uploaded once at construction.
//! - Index buffer: mesh indices uploaded once.
//! - Storage buffer: growable; holds the live instance list for this
//!   primitive kind. Bound to `@group(1) @binding(0)` when drawing.
//! - Draw call: `draw_indexed(mesh.indices, 0, 0..instance_count)`.
//!   Vertex shader reads `instance_id = @builtin(instance_index)` and
//!   indexes into `instances[instance_id]`.
//!
//! `record` issues one draw call per primitive kind with non-zero
//! instance count (typically 4 draw calls max for 5c.2.a).
//!
//! # Grouping responsibility
//!
//! Caller groups instances by `PrimitiveKind` and calls
//! `upload_kind(kind, data)` once per kind per rebuild. Keeps the
//! renderer simple (no per-frame instance-by-kind re-partition) and
//! lets the caller's grouping match its own iteration order.

#![deny(unsafe_code)]

use bytemuck::{Pod, Zeroable};
use flowplan_assets::{CpuMesh, Vertex};
use flowplan_world::feature_instances::PrimitiveKind;
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

/// Per-primitive GPU resources — mesh buffers + instance storage.
/// Held inside the renderer; the caller never sees this directly.
struct PrimitiveSlot {
    vbo: wgpu::Buffer,
    ibo: wgpu::Buffer,
    index_count: u32,
    instance_buf: wgpu::Buffer,
    instance_bg: wgpu::BindGroup,
    /// Capacity in instances (not bytes). Grows via `upload_kind`.
    capacity: u32,
    /// Live instance count driving the draw range.
    instance_count: u32,
}

/// Renderer for instanced feature primitives. Shares a single pipeline
/// across primitives; owns a [`PrimitiveSlot`] per [`PrimitiveKind`].
pub struct InstanceRenderer {
    pipeline: wgpu::RenderPipeline,
    instance_bgl: wgpu::BindGroupLayout,
    /// Parallel to [`PrimitiveKind::ALL`] — index via
    /// [`Self::kind_index`] so adding a primitive is a single-location
    /// change (extend `PrimitiveKind::ALL` + pass the matching mesh).
    slots: Vec<PrimitiveSlot>,
}

/// Initial capacity per-primitive in instances. Sized for the 5c.1
/// active-set upper bound (~25 cells × ~8 houses). Grows via doubling.
const INITIAL_CAPACITY: u32 = 512;

/// Baked-in WGSL source; exposed to lib-test so the shader-presence
/// sanity tests can keep the stale-build failure mode out of the build.
pub const INSTANCES_WGSL_SOURCE: &str = include_str!("instances.wgsl");

impl InstanceRenderer {
    /// Construct the renderer: compile shader, build pipeline, upload
    /// one mesh + allocate one empty storage buffer per primitive kind.
    ///
    /// `meshes` must have one entry per variant of [`PrimitiveKind::ALL`]
    /// in the same order. Callers use [`Self::build_primitive_meshes`]
    /// or construct manually.
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        camera_bgl: &wgpu::BindGroupLayout,
        scene_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        meshes: &[&CpuMesh],
    ) -> Self {
        assert_eq!(
            meshes.len(),
            PrimitiveKind::ALL.len(),
            "expected one mesh per PrimitiveKind variant"
        );

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

        let slots = meshes
            .iter()
            .map(|mesh| {
                let vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("{}-vbo", mesh.label)),
                    contents: bytemuck::cast_slice(&mesh.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let ibo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("{}-ibo", mesh.label)),
                    contents: bytemuck::cast_slice(&mesh.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                let (instance_buf, instance_bg) =
                    create_instance_storage(device, &instance_bgl, INITIAL_CAPACITY);
                PrimitiveSlot {
                    vbo,
                    ibo,
                    index_count: mesh.indices.len() as u32,
                    instance_buf,
                    instance_bg,
                    capacity: INITIAL_CAPACITY,
                    instance_count: 0,
                }
            })
            .collect();

        Self {
            pipeline,
            instance_bgl,
            slots,
        }
    }

    /// Build the default primitive mesh set in the canonical order
    /// matching `PrimitiveKind::ALL`. Returned `Vec<CpuMesh>` lives
    /// long enough for the caller to pass `&[&CpuMesh]` to
    /// [`Self::new`].
    ///
    /// Default polygon counts tuned for readable silhouettes at flyby
    /// distance without overtessellating: 8-sided cone + cylinder +
    /// half-cylinder; 12×8 UV sphere; 2-triangle flat quad.
    #[must_use]
    pub fn build_primitive_meshes() -> Vec<CpuMesh> {
        vec![
            CpuMesh::primitive_box(),
            CpuMesh::primitive_cone(8),
            CpuMesh::primitive_cylinder(8),
            CpuMesh::primitive_sphere(12, 8),
            CpuMesh::primitive_half_cylinder(8),
            CpuMesh::primitive_flat_quad(),
        ]
    }

    /// Replace the active-set instance list for a single primitive
    /// kind. Grows that slot's storage buffer if needed (doubles
    /// capacity until the upload fits). Uploads the full buffer in
    /// one `queue.write_buffer` call.
    ///
    /// Call once per kind per rebuild. An empty slice clears the
    /// kind's draw (no-op on record).
    pub fn upload_kind(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        kind: PrimitiveKind,
        data: &[InstanceGpuData],
    ) {
        let slot_idx = Self::kind_index(kind);
        let slot = &mut self.slots[slot_idx];
        let need = data.len() as u32;
        if need > slot.capacity {
            let mut new_cap = slot.capacity.max(1);
            while new_cap < need {
                new_cap = new_cap.saturating_mul(2);
            }
            let (buf, bg) = create_instance_storage(device, &self.instance_bgl, new_cap);
            slot.instance_buf = buf;
            slot.instance_bg = bg;
            slot.capacity = new_cap;
        }
        if !data.is_empty() {
            queue.write_buffer(&slot.instance_buf, 0, bytemuck::cast_slice(data));
        }
        slot.instance_count = need;
    }

    /// Total live instance count across all primitive kinds. HUD /
    /// telemetry consumer.
    #[must_use]
    pub fn instance_count(&self) -> u32 {
        self.slots.iter().map(|s| s.instance_count).sum()
    }

    /// Live instance count for a single primitive kind. Useful for
    /// per-kind diagnostics.
    #[must_use]
    pub fn instance_count_for(&self, kind: PrimitiveKind) -> u32 {
        self.slots[Self::kind_index(kind)].instance_count
    }

    /// Current storage capacity (instances) for a primitive kind.
    #[must_use]
    pub fn capacity_for(&self, kind: PrimitiveKind) -> u32 {
        self.slots[Self::kind_index(kind)].capacity
    }

    /// Record all non-empty primitive-kind draw calls into the scene
    /// pass. Binds camera (0), per-kind instance storage (1), scene
    /// (2); issues one indexed draw per kind.
    ///
    /// Pipeline is bound once (all primitives share it). Kinds with
    /// `instance_count == 0` are skipped (no draw call submitted).
    pub fn record<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        camera_bg: &'a wgpu::BindGroup,
        scene_bg: &'a wgpu::BindGroup,
    ) {
        // Early-out if every slot is empty.
        if self.slots.iter().all(|s| s.instance_count == 0) {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bg, &[]);
        pass.set_bind_group(2, scene_bg, &[]);
        for slot in &self.slots {
            if slot.instance_count == 0 {
                continue;
            }
            pass.set_bind_group(1, &slot.instance_bg, &[]);
            pass.set_vertex_buffer(0, slot.vbo.slice(..));
            pass.set_index_buffer(slot.ibo.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..slot.index_count, 0, 0..slot.instance_count);
        }
    }

    /// Map a `PrimitiveKind` to its index in `PrimitiveKind::ALL` (and
    /// therefore into `self.slots`). Single point of truth; grep-
    /// findable when extending the enum — the compiler's exhaustive
    /// match will flag any new variant added without a slot.
    const fn kind_index(kind: PrimitiveKind) -> usize {
        match kind {
            PrimitiveKind::Box => 0,
            PrimitiveKind::Cone => 1,
            PrimitiveKind::Cylinder => 2,
            PrimitiveKind::Sphere => 3,
            PrimitiveKind::HalfCylinder => 4,
            PrimitiveKind::FlatQuad => 5,
        }
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
            // `primitive_box` still uses `CpuMesh::cube` which has
            // mixed face winding (some CCW, others CW). Cone /
            // cylinder / sphere were authored CCW-from-outside in
            // 5c.2.a and would back-cull cleanly on their own, but
            // the shared pipeline has to accommodate the box too.
            // Re-enable `Face::Back` once the box mesh is re-authored
            // (tracked in `simple_primitives_render_direction` memory
            // follow-up).
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
