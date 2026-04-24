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

    /// Unit-size primitive box for instance rendering (Phase 3.6.3c.5c.1).
    ///
    /// Dimensions: 1 m × 1 m × 1 m, centered at origin, vertex color
    /// white. The instance shader reads (position, scale, `rotation_y`,
    /// color) from a storage buffer and applies them per draw; the
    /// mesh itself is uploaded once and reused for every box instance.
    ///
    /// 5c.1 MVP: this is the only instance primitive. 5c.2+ will add
    /// cone / cylinder / sphere / etc.
    #[must_use]
    pub fn primitive_box() -> Self {
        let mut mesh = Self::cube(1.0, [1.0, 1.0, 1.0, 1.0]);
        mesh.label = "primitive_box".to_string();
        mesh
    }

    /// Build a flat grid patch for the terrain, sampled from a provided height function.
    ///
    /// The patch is `cells_per_side` cells wide, spanning `[-half_extent, +half_extent]`
    /// in world X/Z (centered at world origin). Height is sampled via the closure.
    /// For a patch centered at an arbitrary world position (used by the
    /// Phase 3.6.3c.5b streaming-render layer), see [`Self::terrain_patch_at`].
    pub fn terrain_patch(
        cells_per_side: u32,
        half_extent: f32,
        sample_height: impl Fn(f32, f32) -> f32 + Sync,
    ) -> Self {
        Self::terrain_patch_at(cells_per_side, half_extent, (0.0, 0.0), sample_height)
    }

    /// Build a flat grid patch for the terrain at an arbitrary world center.
    ///
    /// Vertex positions are in **world coordinates** (no model-matrix
    /// translation needed). The patch spans `[center.x - half_extent,
    /// center.x + half_extent]` × `[center.z - half_extent, center.z +
    /// half_extent]`. Height is sampled via the closure at world coords.
    ///
    /// Used by the streaming-render layer (Phase 3.6.3c.5b — see memory
    /// `streaming_render_architecture` + MAPGEN § Render streaming).
    /// Built in parallel via `rayon::par_iter` because `cells_per_side`
    /// can be 1000+ for a 10 km × 20 m-cell patch (~1 M vertices) and
    /// the closure (`terrain.height(x, z)`) is the dominant cost.
    pub fn terrain_patch_at(
        cells_per_side: u32,
        half_extent: f32,
        center: (f32, f32),
        sample_height: impl Fn(f32, f32) -> f32 + Sync,
    ) -> Self {
        // Square mesh = disc with radius so large it clips nothing.
        // Using f32::INFINITY triggers NaN via multiplication, so use a
        // large finite value (1e9 m ≫ any realistic world extent).
        Self::terrain_patch_disc_at(
            cells_per_side,
            half_extent,
            f32::MAX * 0.5,
            center,
            sample_height,
        )
    }

    /// Build a terrain mesh where vertices outside `disc_radius` of the
    /// center are skipped (height not sampled) and triangles with any
    /// out-of-disc vertex are not emitted. Net result: a circular mesh
    /// inscribed in the `2 × half_extent` square bounding box.
    ///
    /// Saves the expensive `sample_height` calls for out-of-disc
    /// vertices (~22% of the square's area at `disc_radius = half_extent`)
    /// while keeping the indexing logic rectangular (in-disc vertex
    /// indices are the same as if the full grid were emitted). Verts
    /// outside the disc are still emitted as dummy positions (at y=0)
    /// because the index remap cost would exceed the savings — they're
    /// just never referenced by any emitted triangle, so GPU ignores
    /// them beyond the memory footprint.
    ///
    /// Used by the streaming-render layer for 20 km-radius disc mode
    /// (3.6.3c.5b-followup). See `streaming_render_architecture` memory.
    pub fn terrain_patch_disc_at(
        cells_per_side: u32,
        half_extent: f32,
        disc_radius: f32,
        center: (f32, f32),
        sample_height: impl Fn(f32, f32) -> f32 + Sync,
    ) -> Self {
        use rayon::prelude::*;

        let mut mesh = Self::new("terrain");
        let cells = cells_per_side;
        let verts_per_side = cells + 1;
        let step = (half_extent * 2.0) / cells as f32;
        let total_verts = (verts_per_side as usize) * (verts_per_side as usize);
        let disc_radius_sq = disc_radius * disc_radius;

        mesh.vertices = (0..total_verts)
            .into_par_iter()
            .map(|i| {
                let ix = (i % verts_per_side as usize) as u32;
                let iz = (i / verts_per_side as usize) as u32;
                let x = center.0 - half_extent + ix as f32 * step;
                let z = center.1 - half_extent + iz as f32 * step;
                // Skip expensive height() sampling for out-of-disc verts.
                let dx = x - center.0;
                let dz = z - center.1;
                if dx * dx + dz * dz > disc_radius_sq {
                    return Vertex {
                        position: [x, 0.0, z],
                        normal: [0.0, 1.0, 0.0],
                        uv: [ix as f32 / cells as f32, iz as f32 / cells as f32],
                        color: [0.0, 0.0, 0.0, 0.0],
                    };
                }

                let y = sample_height(x, z);
                let eps = step * 0.5;
                let dy_dx = (sample_height(x + eps, z) - sample_height(x - eps, z)) / (2.0 * eps);
                let dy_dz = (sample_height(x, z + eps) - sample_height(x, z - eps)) / (2.0 * eps);
                let normal = Vec3::new(-dy_dx, 1.0, -dy_dz).normalize_or(Vec3::Y);

                let color = terrain_color_for_height(y);

                Vertex {
                    position: [x, y, z],
                    normal: [normal.x, normal.y, normal.z],
                    uv: [ix as f32 / cells as f32, iz as f32 / cells as f32],
                    color,
                }
            })
            .collect();

        // Emit triangles only where ALL FOUR corner vertices of the cell
        // are inside the disc. Cells straddling the boundary get
        // dropped; the mesh's visible edge is a blocky approximation of
        // a circle but with small cells (20-30 m at typical config)
        // reads as smooth at the 20 km view distance.
        mesh.indices
            .reserve((cells as usize) * (cells as usize) * 6);
        let in_disc = |x_w: f32, z_w: f32| -> bool {
            let dx = x_w - center.0;
            let dz = z_w - center.1;
            dx * dx + dz * dz <= disc_radius_sq
        };
        for iz in 0..cells {
            for ix in 0..cells {
                let x0 = center.0 - half_extent + ix as f32 * step;
                let x1 = x0 + step;
                let z0 = center.1 - half_extent + iz as f32 * step;
                let z1 = z0 + step;
                if !(in_disc(x0, z0) && in_disc(x1, z0) && in_disc(x0, z1) && in_disc(x1, z1)) {
                    continue;
                }
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

    /// Build a flat water grid at `y = 0` with uniform color and upward normals.
    ///
    /// Tessellated (not a single quad) so Milestone 2.2.2 can apply Gerstner wave
    /// displacement in the vertex shader without re-authoring the mesh.
    #[must_use]
    pub fn water_grid(cells_per_side: u32, half_extent: f32, color: [f32; 4]) -> Self {
        Self::water_grid_at(cells_per_side, half_extent, (0.0, 0.0), color)
    }

    /// Build a flat water grid at `y = 0` centered at an arbitrary world
    /// position. Vertex positions in world coordinates (no model-matrix
    /// translation needed). Used by the streaming-render layer
    /// (Phase 3.6.3c.5b) to keep water aligned with the active camera.
    /// Build a flat water grid clipped to a disc of `disc_radius`.
    /// Same pattern as [`Self::terrain_patch_disc_at`] — rectangular
    /// vertex grid, triangles only emitted inside the disc.
    #[must_use]
    pub fn water_grid_disc_at(
        cells_per_side: u32,
        half_extent: f32,
        disc_radius: f32,
        center: (f32, f32),
        color: [f32; 4],
    ) -> Self {
        let mut mesh = Self::new("water");
        let cells = cells_per_side.max(1);
        let verts_per_side = cells + 1;
        let step = (half_extent * 2.0) / cells as f32;
        let disc_radius_sq = disc_radius * disc_radius;

        // All vertices are emitted (positions are cheap); out-of-disc
        // verts are just never indexed by any triangle.
        for iz in 0..verts_per_side {
            for ix in 0..verts_per_side {
                let x = center.0 - half_extent + ix as f32 * step;
                let z = center.1 - half_extent + iz as f32 * step;
                mesh.vertices.push(Vertex {
                    position: [x, 0.0, z],
                    normal: [0.0, 1.0, 0.0],
                    uv: [ix as f32 / cells as f32, iz as f32 / cells as f32],
                    color,
                });
            }
        }

        // Emit only cells fully inside the disc.
        let in_disc = |x_w: f32, z_w: f32| -> bool {
            let dx = x_w - center.0;
            let dz = z_w - center.1;
            dx * dx + dz * dz <= disc_radius_sq
        };
        for iz in 0..cells {
            for ix in 0..cells {
                let x0 = center.0 - half_extent + ix as f32 * step;
                let x1 = x0 + step;
                let z0 = center.1 - half_extent + iz as f32 * step;
                let z1 = z0 + step;
                if !(in_disc(x0, z0) && in_disc(x1, z0) && in_disc(x0, z1) && in_disc(x1, z1)) {
                    continue;
                }
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

    #[must_use]
    pub fn water_grid_at(
        cells_per_side: u32,
        half_extent: f32,
        center: (f32, f32),
        color: [f32; 4],
    ) -> Self {
        let mut mesh = Self::new("water");
        let cells = cells_per_side.max(1);
        let verts_per_side = cells + 1;
        let step = (half_extent * 2.0) / cells as f32;

        for iz in 0..verts_per_side {
            for ix in 0..verts_per_side {
                let x = center.0 - half_extent + ix as f32 * step;
                let z = center.1 - half_extent + iz as f32 * step;
                mesh.vertices.push(Vertex {
                    position: [x, 0.0, z],
                    normal: [0.0, 1.0, 0.0],
                    uv: [ix as f32 / cells as f32, iz as f32 / cells as f32],
                    color,
                });
            }
        }

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

    /// Axis-aligned bounding box: `(min, max)` in the mesh's local space. Each is
    /// `[x, y, z]`. Returns `([0, 0, 0], [0, 0, 0])` for an empty mesh.
    #[must_use]
    pub fn bbox(&self) -> ([f32; 3], [f32; 3]) {
        if self.vertices.is_empty() {
            return ([0.0; 3], [0.0; 3]);
        }
        let mut mn = [f32::INFINITY; 3];
        let mut mx = [f32::NEG_INFINITY; 3];
        for v in &self.vertices {
            for axis in 0..3 {
                if v.position[axis] < mn[axis] {
                    mn[axis] = v.position[axis];
                }
                if v.position[axis] > mx[axis] {
                    mx[axis] = v.position[axis];
                }
            }
        }
        (mn, mx)
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

/// Terrain color ramp used by [`CpuMesh::terrain_patch`].
///
/// Five smoothly-blended tiers by height. Tuned for the Phase 2.2.4 mountain-scale
/// terrain envelope (±375 m max, per ref1/2/3 reference photos):
/// - below 6 m -- sandy beige (beach / shoreline approach)
/// - 6 to 60 m -- field green (lowland grass)
/// - 60 to 180 m -- forest green (wooded slopes)
/// - 180 to 320 m -- rocky brown (bare ridge / cliff)
/// - above 320 m -- snow white (peaks / glaciers)
fn terrain_color_for_height(y: f32) -> [f32; 4] {
    const SAND: [f32; 3] = [0.80, 0.72, 0.54];
    const FIELD: [f32; 3] = [0.50, 0.62, 0.32];
    const FOREST: [f32; 3] = [0.25, 0.40, 0.20];
    const ROCK: [f32; 3] = [0.52, 0.44, 0.34];
    const SNOW: [f32; 3] = [0.95, 0.96, 0.98];
    let rgb = if y < 6.0 {
        let t = ((y + 4.0) / 10.0).clamp(0.0, 1.0);
        lerp3(SAND, FIELD, t)
    } else if y < 60.0 {
        FIELD
    } else if y < 180.0 {
        let t = ((y - 60.0) / 120.0).clamp(0.0, 1.0);
        lerp3(FIELD, FOREST, t)
    } else if y < 320.0 {
        let t = ((y - 180.0) / 140.0).clamp(0.0, 1.0);
        lerp3(FOREST, ROCK, t)
    } else {
        let t = ((y - 320.0) / 80.0).clamp(0.0, 1.0);
        lerp3(ROCK, SNOW, t)
    };
    [rgb[0], rgb[1], rgb[2], 1.0]
}

#[inline]
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
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
    fn water_grid_has_expected_counts() {
        let m = CpuMesh::water_grid(4, 10.0, [0.1, 0.3, 0.55, 1.0]);
        // 5x5 verts = 25
        assert_eq!(m.vertex_count(), 25);
        // 4x4 cells * 6 indices = 96
        assert_eq!(m.index_count(), 96);
        // All verts at y=0, normal up.
        for v in &m.vertices {
            assert!((v.position[1] - 0.0).abs() < f32::EPSILON);
            assert!((v.normal[1] - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn bbox_of_cube_is_symmetric() {
        let m = CpuMesh::cube(2.0, [1.0; 4]);
        let (mn, mx) = m.bbox();
        for axis in 0..3 {
            assert!((mn[axis] - -1.0).abs() < 1e-6);
            assert!((mx[axis] - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn bbox_empty_mesh() {
        let m = CpuMesh::new("empty");
        let (mn, mx) = m.bbox();
        for axis in 0..3 {
            assert!(mn[axis].abs() < f32::EPSILON);
            assert!(mx[axis].abs() < f32::EPSILON);
        }
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
