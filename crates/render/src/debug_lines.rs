//! Cel-shaded debug-line rendering: triangulated quads with a dark outline band behind
//! a slightly thinner colored fill. Pipeline topology is `TriangleList`.
//!
//! Phase 2.4.2c infrastructure. The first consumer is the bottom-right orientation
//! widget; later phases reuse this for HUD ladders (pitch ladder, bank indicator ticks),
//! cockpit instrument face lines, flight-path vector, and (with a second depth-aware
//! pipeline variant) wireframe collision volumes.
//!
//! # Cel outline
//!
//! Each segment is rendered as two quads drawn in two distinct draw calls within the
//! same render pass:
//! 1. A wider dark-grey *outline* quad (painter's order: drawn first, behind).
//! 2. A narrower colored *fill* quad (drawn second, on top).
//!
//! The outline band extending past the fill reads as a cel-style border. The outline
//! thickness is intentionally set a hair thinner than the main scene's sobel outline
//! so the widget doesn't visually compete with scene edges.
//!
//! # Pipeline shape
//!
//! The pipeline reads a single `camera`-shaped uniform at `@group(0) @binding(0)`. For
//! the widget use case the app writes a *widget view-proj* matrix into a dedicated
//! uniform buffer (see `Renderer::widget_camera_*`); vectors are in widget-local
//! `[-1, 1]` space and mapped to a screen-space rectangle.
//!
//! Depth is disabled; the widget is drawn last and overlays everything. When a future
//! consumer needs depth-aware 3D geometry in the main scene, add a second pipeline
//! with `depth_stencil: Some(...)`.

#![deny(unsafe_code)]

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

/// One vertex of a debug line/quad: position + RGB color.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DebugLineVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

/// CPU-side accumulator.
///
/// Each segment produces one outline quad (6 verts) and one fill quad (6 verts); the
/// two vecs are drawn in two separate draw calls so all outlines land behind all fills
/// regardless of per-segment push order.
///
/// The app clears + repopulates this each frame. Construction costs are amortized; the
/// `Vec`s grow to a stable capacity after the first few frames and stay there.
#[derive(Clone, Debug, Default)]
pub struct DebugLineBatch {
    /// Triangle-list vertices for the dark outline quads (drawn first, behind).
    pub outline_vertices: Vec<DebugLineVertex>,
    /// Triangle-list vertices for the colored fill quads (drawn second, on top).
    pub fill_vertices: Vec<DebugLineVertex>,
}

/// Outline color (dark grey, not pure black -- matches the scene's cel outline feel
/// without getting lost against a dark rim).
pub const OUTLINE_COLOR: [f32; 3] = [0.05, 0.05, 0.08];

/// Quad half-widths, in **widget-local units** (the widget cube spans `[-1, 1]³`; 2.0
/// full units = `WIDGET_PX` pixels, so 1 unit ≈ 90 px at the default widget size).
///
/// These values yield ~2.5 px fill and ~4 px outline at the default 180 px widget
/// size. The outline extends ~0.75 px past the fill on each side, which reads as a
/// crisp cel band without overwhelming the widget interior. Scale proportionally if
/// `WIDGET_PX` in `renderer.rs` changes.
pub const FILL_HALF_W: f32 = 0.014;
pub const OUTLINE_HALF_W: f32 = 0.022;

impl DebugLineBatch {
    /// New empty batch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset without freeing storage. Call at the start of each frame.
    pub fn clear(&mut self) {
        self.outline_vertices.clear();
        self.fill_vertices.clear();
    }

    /// True iff no segments have been pushed this frame.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.outline_vertices.is_empty() && self.fill_vertices.is_empty()
    }

    /// Total vertex count across outline and fill. Each segment contributes 12 vertices.
    #[must_use]
    pub fn len_vertices(&self) -> usize {
        self.outline_vertices.len() + self.fill_vertices.len()
    }

    /// Push a line segment from `a` to `b` in `color`. Emits one outline quad and one
    /// fill quad. Widening is applied in the widget's **XY plane** -- safe for the
    /// Phase 2.4.2c widget because it uses an orthographic projection looking down +Z,
    /// so widget-local XY maps 1:1 to screen XY.
    ///
    /// When a future consumer uses this with a non-orthographic projection (e.g., a
    /// depth-aware world-space wireframe pass), the widening will need to happen in
    /// clip space inside a dedicated vertex shader; the batch API changes at that
    /// point. Not worth building now.
    pub fn push_segment(&mut self, a: Vec3, b: Vec3, color: [f32; 3]) {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = dx.hypot(dy);
        if len < 1e-6 {
            return;
        }
        // Unit perpendicular in the widget's XY plane.
        let perp_x = -dy / len;
        let perp_y = dx / len;
        push_quad(
            &mut self.outline_vertices,
            a,
            b,
            perp_x,
            perp_y,
            OUTLINE_HALF_W,
            OUTLINE_COLOR,
        );
        push_quad(
            &mut self.fill_vertices,
            a,
            b,
            perp_x,
            perp_y,
            FILL_HALF_W,
            color,
        );
    }

    /// Push a vector as `(origin, origin + direction)`. Convenience for the gizmo
    /// builder.
    pub fn push_vector(&mut self, origin: Vec3, direction: Vec3, color: [f32; 3]) {
        self.push_segment(origin, origin + direction, color);
    }
}

fn push_quad(
    verts: &mut Vec<DebugLineVertex>,
    a: Vec3,
    b: Vec3,
    perp_x: f32,
    perp_y: f32,
    half_w: f32,
    color: [f32; 3],
) {
    let ox = perp_x * half_w;
    let oy = perp_y * half_w;
    let a_left = [a.x + ox, a.y + oy, a.z];
    let a_right = [a.x - ox, a.y - oy, a.z];
    let b_left = [b.x + ox, b.y + oy, b.z];
    let b_right = [b.x - ox, b.y - oy, b.z];
    // Two triangles forming the quad. CCW winding when viewed from +Z (out of screen).
    verts.push(DebugLineVertex {
        position: a_left,
        color,
    });
    verts.push(DebugLineVertex {
        position: a_right,
        color,
    });
    verts.push(DebugLineVertex {
        position: b_left,
        color,
    });
    verts.push(DebugLineVertex {
        position: a_right,
        color,
    });
    verts.push(DebugLineVertex {
        position: b_right,
        color,
    });
    verts.push(DebugLineVertex {
        position: b_left,
        color,
    });
}

/// GPU-side quad pipeline + dynamic vertex buffers for outline and fill.
pub struct DebugLinesRenderer {
    pipeline: wgpu::RenderPipeline,
    outline_vbo: wgpu::Buffer,
    outline_capacity: u64,
    fill_vbo: wgpu::Buffer,
    fill_capacity: u64,
}

impl DebugLinesRenderer {
    /// Build the pipeline. The caller-supplied camera BGL is reused for the widget's
    /// dedicated uniform so one pipeline serves both scene-camera and widget-camera
    /// draws.
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        camera_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowplan-debug-lines-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("debug_lines.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("debug-lines-pipeline-layout"),
            bind_group_layouts: &[camera_bgl],
            push_constant_ranges: &[],
        });
        let vertex_attrs = wgpu::vertex_attr_array![
            0 => Float32x3, // position
            1 => Float32x3, // color
        ];
        let vertex_buffers = [wgpu::VertexBufferLayout {
            array_stride: core::mem::size_of::<DebugLineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &vertex_attrs,
        }];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("debug-lines-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &vertex_buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // Culling off: segment winding depends on endpoint order and we don't
                // want the builder to care about that.
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let initial_capacity: u64 = 512; // ~85 segments before first growth
        let outline_vbo = create_empty_vbo(device, "debug-lines-outline-vbo", initial_capacity);
        let fill_vbo = create_empty_vbo(device, "debug-lines-fill-vbo", initial_capacity);
        Self {
            pipeline,
            outline_vbo,
            outline_capacity: initial_capacity,
            fill_vbo,
            fill_capacity: initial_capacity,
        }
    }

    /// Upload both vertex lists to their VBOs, growing if necessary.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, batch: &DebugLineBatch) {
        upload_or_grow(
            device,
            queue,
            &mut self.outline_vbo,
            &mut self.outline_capacity,
            &batch.outline_vertices,
            "debug-lines-outline-vbo-grown",
        );
        upload_or_grow(
            device,
            queue,
            &mut self.fill_vbo,
            &mut self.fill_capacity,
            &batch.fill_vertices,
            "debug-lines-fill-vbo-grown",
        );
    }

    /// Issue draws for the outline (first) and fill (second). Caller must have bound
    /// the camera BGL at `@group(0)`.
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, batch: &DebugLineBatch) {
        pass.set_pipeline(&self.pipeline);
        let outline_count = batch.outline_vertices.len() as u32;
        if outline_count > 0 {
            pass.set_vertex_buffer(0, self.outline_vbo.slice(..));
            pass.draw(0..outline_count, 0..1);
        }
        let fill_count = batch.fill_vertices.len() as u32;
        if fill_count > 0 {
            pass.set_vertex_buffer(0, self.fill_vbo.slice(..));
            pass.draw(0..fill_count, 0..1);
        }
    }
}

fn create_empty_vbo(device: &wgpu::Device, label: &str, capacity_verts: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: capacity_verts * core::mem::size_of::<DebugLineVertex>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn upload_or_grow(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    vbo: &mut wgpu::Buffer,
    capacity: &mut u64,
    verts: &[DebugLineVertex],
    grown_label: &str,
) {
    let needed = verts.len() as u64;
    if needed == 0 {
        return;
    }
    if needed > *capacity {
        let new_capacity = needed.next_power_of_two().max(*capacity * 2);
        *vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(grown_label),
            contents: bytemuck::cast_slice(verts),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        *capacity = new_capacity;
        return;
    }
    queue.write_buffer(vbo, 0, bytemuck::cast_slice(verts));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_starts_empty() {
        let b = DebugLineBatch::new();
        assert!(b.is_empty());
        assert_eq!(b.len_vertices(), 0);
    }

    #[test]
    fn push_segment_emits_twelve_vertices() {
        let mut b = DebugLineBatch::new();
        b.push_segment(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), [1.0, 0.0, 0.0]);
        // One outline quad (6) + one fill quad (6).
        assert_eq!(b.len_vertices(), 12);
        assert_eq!(b.outline_vertices.len(), 6);
        assert_eq!(b.fill_vertices.len(), 6);
    }

    #[test]
    fn push_segment_with_zero_length_is_noop() {
        let mut b = DebugLineBatch::new();
        b.push_segment(Vec3::ZERO, Vec3::ZERO, [1.0, 0.0, 0.0]);
        assert!(b.is_empty());
    }

    #[test]
    fn fill_quad_is_narrower_than_outline() {
        let mut b = DebugLineBatch::new();
        // Horizontal segment along +X. Perpendicular is ±Y.
        b.push_segment(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), [1.0, 0.0, 0.0]);
        // First outline vertex is at (ox * half_w, oy * half_w, 0) with perp = (0, 1).
        // So outline_y should match +OUTLINE_HALF_W, fill_y should match +FILL_HALF_W.
        let outline_top_y = b.outline_vertices[0].position[1];
        let fill_top_y = b.fill_vertices[0].position[1];
        assert!(
            (outline_top_y - OUTLINE_HALF_W).abs() < 1e-5,
            "outline_top_y = {outline_top_y}",
        );
        assert!(
            (fill_top_y - FILL_HALF_W).abs() < 1e-5,
            "fill_top_y = {fill_top_y}",
        );
        assert!(outline_top_y > fill_top_y, "outline must extend past fill");
    }

    #[test]
    fn clear_preserves_capacity() {
        let mut b = DebugLineBatch::new();
        for _ in 0..32 {
            b.push_segment(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), [1.0, 1.0, 1.0]);
        }
        let o_cap = b.outline_vertices.capacity();
        let f_cap = b.fill_vertices.capacity();
        b.clear();
        assert!(b.is_empty());
        assert!(b.outline_vertices.capacity() >= o_cap);
        assert!(b.fill_vertices.capacity() >= f_cap);
    }
}
