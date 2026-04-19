//! CPU-side mesh representation and builders.
//!
//! `Vertex` is `repr(C)` POD so it can be uploaded to a GPU vertex buffer via
//! `bytemuck::cast_slice` without copying. `CpuMesh` holds owned vertex + index buffers.
//!
//! Phase 1 builders: `cube`, `terrain_patch`. Future phases add glTF import.

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};

/// A single vertex: position, normal, uv, and a flat-shade color.
///
/// `#[repr(C)]` layout is a soft contract with the render pipeline's `vertex_attr_array`;
/// any change here requires a matching change in the render crate.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

impl Vertex {
    /// Stride in bytes. Used by the render crate to configure the vertex buffer layout.
    pub const STRIDE: wgpu_stride::Count = wgpu_stride::Count::new(core::mem::size_of::<Self>());
}

/// A CPU-resident mesh: typed vertex array and 32-bit index buffer.
///
/// Allocated once at asset build time; the render crate uploads to the GPU and retains
/// the wgpu buffers. Phase 1 meshes are trivially small and live for the lifetime of
/// the app; Phase 2+ will use the handle table to manage lifetimes.
#[derive(Clone, Debug)]
pub struct CpuMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// A human-readable name, used by the render crate for wgpu debug labels.
    pub label: String,
}

impl CpuMesh {
    /// Construct an empty mesh with a label.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            label: label.into(),
        }
    }

    /// Build a unit cube centered at origin, color-tinted as given.
    #[must_use]
    pub fn cube(size: f32, color: [f32; 4]) -> Self {
        let half = size * 0.5;
        let mut mesh = Self::new("cube");

        // Six faces. For each: normal, tangent_u, tangent_v defining the face plane.
        let faces: [(Vec3, Vec3, Vec3); 6] = [
            (Vec3::Y, Vec3::X, Vec3::Z),         // top +Y
            (Vec3::NEG_Y, Vec3::X, Vec3::NEG_Z), // bottom -Y
            (Vec3::Z, Vec3::X, Vec3::Y),         // back +Z
            (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y), // front -Z
            (Vec3::X, Vec3::NEG_Z, Vec3::Y),     // right +X
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),     // left -X
        ];
        for (normal, tangent_u, tangent_v) in faces {
            let start = mesh.vertices.len() as u32;
            let corners = [
                (Vec2::new(0.0, 0.0), -tangent_u - tangent_v),
                (Vec2::new(1.0, 0.0), tangent_u - tangent_v),
                (Vec2::new(1.0, 1.0), tangent_u + tangent_v),
                (Vec2::new(0.0, 1.0), -tangent_u + tangent_v),
            ];
            for (uv, corner) in corners {
                let pos = normal * half + corner * half;
                mesh.vertices.push(Vertex {
                    position: [pos.x, pos.y, pos.z],
                    normal: [normal.x, normal.y, normal.z],
                    uv: [uv.x, uv.y],
                    color,
                });
            }
            mesh.indices.extend_from_slice(&[
                start,
                start + 1,
                start + 2,
                start,
                start + 2,
                start + 3,
            ]);
        }
        mesh
    }

    /// Build a flat grid patch for the terrain, sampled from a provided height function.
    ///
    /// The patch is `cells_per_side` cells wide, spanning `[-half_extent, +half_extent]`
    /// in world X/Z. Height is sampled via the closure.
    pub fn terrain_patch(
        cells_per_side: u32,
        half_extent: f32,
        sample_height: impl Fn(f32, f32) -> f32,
    ) -> Self {
        let mut mesh = Self::new("terrain");
        let cells = cells_per_side;
        let verts_per_side = cells + 1;
        let step = (half_extent * 2.0) / cells as f32;

        // Build vertices.
        for iz in 0..verts_per_side {
            for ix in 0..verts_per_side {
                let x = -half_extent + ix as f32 * step;
                let z = -half_extent + iz as f32 * step;
                let y = sample_height(x, z);

                // Approximate normal via central differences on the height function.
                let eps = step * 0.5;
                let dy_dx = (sample_height(x + eps, z) - sample_height(x - eps, z)) / (2.0 * eps);
                let dy_dz = (sample_height(x, z + eps) - sample_height(x, z - eps)) / (2.0 * eps);
                let normal = Vec3::new(-dy_dx, 1.0, -dy_dz).normalize_or(Vec3::Y);

                // Gentle color ramp: green at sea level -> brown at altitude.
                let tier = (y / 20.0).clamp(0.0, 1.0);
                let color = [
                    0.35 + 0.4 * tier,
                    0.55 - 0.2 * tier,
                    0.22 - 0.15 * tier,
                    1.0,
                ];

                mesh.vertices.push(Vertex {
                    position: [x, y, z],
                    normal: [normal.x, normal.y, normal.z],
                    uv: [ix as f32 / cells as f32, iz as f32 / cells as f32],
                    color,
                });
            }
        }

        // Build indices (two triangles per cell).
        for iz in 0..cells {
            for ix in 0..cells {
                let row = verts_per_side;
                let tl = iz * row + ix;
                let tr = iz * row + ix + 1;
                let bl = (iz + 1) * row + ix;
                let br = (iz + 1) * row + ix + 1;
                mesh.indices.extend_from_slice(&[tl, bl, tr, tr, bl, br]);
            }
        }
        mesh
    }

    /// Returns the vertex count.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// Returns the index count.
    #[must_use]
    pub fn index_count(&self) -> usize {
        self.indices.len()
    }

    /// Minimum Y coordinate across all vertices. Returns 0.0 for an empty mesh.
    #[must_use]
    pub fn bbox_min_y(&self) -> f32 {
        if self.vertices.is_empty() {
            return 0.0;
        }
        self.vertices
            .iter()
            .map(|v| v.position[1])
            .fold(f32::INFINITY, f32::min)
    }

    /// Translate every vertex by `offset`. Normals are unchanged (rotation/scale-only
    /// transforms would require normal-matrix handling; translation does not).
    pub fn translate(&mut self, offset: [f32; 3]) {
        for v in &mut self.vertices {
            v.position[0] += offset[0];
            v.position[1] += offset[1];
            v.position[2] += offset[2];
        }
    }
}

// Compile-time sanity on Vertex layout. Kept tight so the render crate can rely on it.
mod wgpu_stride {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Count(pub usize);

    impl Count {
        pub const fn new(n: usize) -> Self {
            Self(n)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_has_six_faces_24_vertices_36_indices() {
        let m = CpuMesh::cube(2.0, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(m.vertex_count(), 24);
        assert_eq!(m.index_count(), 36);
    }

    #[test]
    fn cube_positions_within_bounds() {
        let m = CpuMesh::cube(2.0, [1.0, 1.0, 1.0, 1.0]);
        for v in &m.vertices {
            for axis in v.position {
                assert!(axis.abs() <= 1.0 + 1e-5);
            }
        }
    }

    #[test]
    fn terrain_patch_sizes_correct() {
        let m = CpuMesh::terrain_patch(4, 10.0, |_x, _z| 0.0);
        // 5x5 verts = 25
        assert_eq!(m.vertex_count(), 25);
        // 4x4 cells * 6 indices = 96
        assert_eq!(m.index_count(), 96);
    }

    #[test]
    fn terrain_patch_flat_normals_up() {
        let m = CpuMesh::terrain_patch(2, 4.0, |_x, _z| 0.0);
        for v in &m.vertices {
            // Flat terrain -> normals point straight up.
            assert!((v.normal[1] - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn vertex_stride_matches_size_of() {
        assert_eq!(Vertex::STRIDE.0, core::mem::size_of::<Vertex>());
    }

    #[test]
    fn bbox_min_y_of_cube_is_minus_half_size() {
        let m = CpuMesh::cube(2.0, [1.0, 1.0, 1.0, 1.0]);
        assert!((m.bbox_min_y() - -1.0).abs() < 1e-6);
    }

    #[test]
    fn bbox_min_y_empty_mesh() {
        let m = CpuMesh::new("empty");
        assert!((m.bbox_min_y() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn translate_shifts_positions() {
        let mut m = CpuMesh::cube(2.0, [1.0, 1.0, 1.0, 1.0]);
        m.translate([0.0, 2.0, 0.0]);
        assert!((m.bbox_min_y() - 1.0).abs() < 1e-6);
        // Normals untouched by translation.
        for v in &m.vertices {
            let n2 =
                v.normal[0] * v.normal[0] + v.normal[1] * v.normal[1] + v.normal[2] * v.normal[2];
            assert!((n2 - 1.0).abs() < 1e-5);
        }
    }
}
