//! flowPlan asset pipeline.
//!
//! Phase 1 scope: POD vertex and mesh types, and CPU-side builders for the placeholder
//! cube (vehicle) and a terrain patch derived from `flowplan_world::Terrain`. No runtime
//! glTF import yet; that's Phase 2.

#![forbid(unsafe_code)]
// Mesh generation intentionally casts usize<->u32 for buffer lengths and u32<->f32 for
// normalized UVs and grid positions. The values involved fit within safe ranges for the
// sizes we produce (64k verts at most in Phase 1).
#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

pub mod gltf_load;
pub mod mesh;

pub use gltf_load::{load_mesh, GltfLoadError};
pub use mesh::{CpuMesh, Vertex};
