//! glTF 2.0 mesh loader.
//!
//! Phase 2 Milestone 2.3 sub-step 1: parse a `.glb` or embedded-buffer `.gltf` byte slice
//! into a `CpuMesh`. Takes the first mesh's first primitive; rejects non-triangle
//! topologies. Missing optional attributes fall back to sensible defaults so a trivially
//! authored mesh (position-only) still renders.
//!
//! The handle-table integration arrives in a later sub-step; for now loading is done
//! eagerly at startup by `flowplan-app`.

use crate::mesh::{CpuMesh, Vertex};

/// Errors produced by [`load_mesh`].
#[derive(thiserror::Error, Debug)]
pub enum GltfLoadError {
    /// Upstream glTF parser rejected the byte slice.
    #[error("glTF parse failed")]
    Parse(#[from] gltf::Error),
    /// Document contains no `meshes[]` entries.
    #[error("glTF has no meshes")]
    NoMesh,
    /// Document's first mesh contains no `primitives[]` entries.
    #[error("glTF first mesh has no primitives")]
    NoPrimitive,
    /// Primitive is missing the required `POSITION` attribute.
    #[error("primitive missing required POSITION attribute")]
    NoPosition,
    /// Primitive uses a topology that this Phase 2 loader does not support. Phase 2
    /// accepts only `Triangles`; strip / fan / line / point handling is deferred.
    #[error("primitive topology is not TriangleList (got {0:?})")]
    UnsupportedTopology(gltf::mesh::Mode),
}

/// Load the first mesh + first primitive of a glTF document into a [`CpuMesh`].
///
/// Supports both `.glb` (self-contained binary) and embedded-buffer `.gltf`. External
/// `.bin` / image references are not resolved; stick to `.glb` for shipping assets.
///
/// Attribute fallback when missing:
/// - `NORMAL`     -> `(0, 1, 0)` on every vertex (caller may recompute if needed)
/// - `TEXCOORD_0` -> `(0, 0)`
/// - `COLOR_0`    -> `(1, 1, 1, 1)`
/// - index buffer -> sequential `0..vertex_count` (non-indexed primitive)
pub fn load_mesh(bytes: &[u8]) -> Result<CpuMesh, GltfLoadError> {
    let (doc, buffers, _images) = gltf::import_slice(bytes)?;
    let mesh = doc.meshes().next().ok_or(GltfLoadError::NoMesh)?;
    let primitive = mesh.primitives().next().ok_or(GltfLoadError::NoPrimitive)?;
    if primitive.mode() != gltf::mesh::Mode::Triangles {
        return Err(GltfLoadError::UnsupportedTopology(primitive.mode()));
    }
    let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or(GltfLoadError::NoPosition)?
        .collect();
    let vertex_count = positions.len();

    let normals: Vec<[f32; 3]> = reader
        .read_normals()
        .map_or_else(|| vec![[0.0, 1.0, 0.0]; vertex_count], Iterator::collect);

    let uvs: Vec<[f32; 2]> = reader.read_tex_coords(0).map_or_else(
        || vec![[0.0, 0.0]; vertex_count],
        |t| t.into_f32().collect(),
    );

    let colors: Vec<[f32; 4]> = reader.read_colors(0).map_or_else(
        || vec![[1.0, 1.0, 1.0, 1.0]; vertex_count],
        |c| c.into_rgba_f32().collect(),
    );

    let indices: Vec<u32> = reader.read_indices().map_or_else(
        || (0..vertex_count as u32).collect(),
        |i| i.into_u32().collect(),
    );

    let label = mesh.name().unwrap_or("gltf-mesh").to_string();
    let mut out = CpuMesh::new(label);
    out.vertices.reserve(vertex_count);
    for i in 0..vertex_count {
        out.vertices.push(Vertex {
            position: positions[i],
            normal: normals[i],
            uv: uvs[i],
            color: colors[i],
        });
    }
    out.indices = indices;
    Ok(out)
}
