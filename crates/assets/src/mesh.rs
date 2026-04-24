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
    #[must_use]
    pub fn primitive_box() -> Self {
        let mut mesh = Self::cube(1.0, [1.0, 1.0, 1.0, 1.0]);
        mesh.label = "primitive_box".to_string();
        mesh
    }

    /// Unit-size primitive cone for instance rendering (Phase 3.6.3c.5c.2).
    ///
    /// Apex at y=+0.5, base at y=-0.5 with radius 0.5 (unit diameter).
    /// Vertex color white; per-instance data applies scale + rotation +
    /// tint. Flat-faceted sides with per-triangle normals for
    /// cel-shading band crispness; base is a single-normal fan.
    ///
    /// `sides` controls the base polygon count; default 8 gives a
    /// readable octagonal cone for tree canopies / signage cones /
    /// rooftop spires without overtessellating.
    #[must_use]
    pub fn primitive_cone(sides: u32) -> Self {
        let sides = sides.max(3);
        let mut mesh = Self::new("primitive_cone");
        let apex_y = 0.5_f32;
        let base_y = -0.5_f32;
        let radius = 0.5_f32;
        let color = [1.0_f32, 1.0, 1.0, 1.0];

        let apex = Vec3::new(0.0, apex_y, 0.0);

        // Side triangles — flat-faceted (one unique per-face normal
        // replicated across the triangle's three vertices). Winding:
        // apex → base[i] → base[i+1] is CCW as seen from outside, so
        // front-face-culling sees the outward surface correctly.
        for i in 0..sides {
            let theta0 = (i as f32) * core::f32::consts::TAU / sides as f32;
            let theta1 = ((i + 1) as f32) * core::f32::consts::TAU / sides as f32;
            let p0 = Vec3::new(radius * theta0.cos(), base_y, radius * theta0.sin());
            let p1 = Vec3::new(radius * theta1.cos(), base_y, radius * theta1.sin());
            let normal = (apex - p0).cross(p1 - p0).normalize_or_zero();

            let start = mesh.vertices.len() as u32;
            let u_left = i as f32 / sides as f32;
            let u_right = (i + 1) as f32 / sides as f32;
            for (pos, uv) in [
                (p0, Vec2::new(u_left, 1.0)),
                (p1, Vec2::new(u_right, 1.0)),
                (apex, Vec2::new((u_left + u_right) * 0.5, 0.0)),
            ] {
                mesh.vertices.push(Vertex {
                    position: [pos.x, pos.y, pos.z],
                    normal: [normal.x, normal.y, normal.z],
                    uv: [uv.x, uv.y],
                    color,
                });
            }
            mesh.indices
                .extend_from_slice(&[start, start + 1, start + 2]);
        }

        // Base disc — flat -Y normal, fan from center. Winding is
        // center → ring[(i+1) % sides] → ring[i] (reversed from the
        // sides so the normal points -Y outward).
        let base_center_idx = mesh.vertices.len() as u32;
        mesh.vertices.push(Vertex {
            position: [0.0, base_y, 0.0],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5, 0.5],
            color,
        });
        let base_ring_start = mesh.vertices.len() as u32;
        for i in 0..sides {
            let theta = (i as f32) * core::f32::consts::TAU / sides as f32;
            mesh.vertices.push(Vertex {
                position: [radius * theta.cos(), base_y, radius * theta.sin()],
                normal: [0.0, -1.0, 0.0],
                uv: [0.5 + theta.cos() * 0.5, 0.5 + theta.sin() * 0.5],
                color,
            });
        }
        for i in 0..sides {
            mesh.indices.push(base_center_idx);
            mesh.indices.push(base_ring_start + ((i + 1) % sides));
            mesh.indices.push(base_ring_start + i);
        }

        mesh
    }

    /// Unit-size primitive cylinder for instance rendering
    /// (Phase 3.6.3c.5c.2).
    ///
    /// From y=-0.5 to y=+0.5 with radius 0.5 (unit diameter, unit
    /// height). Vertex color white. Flat-faceted sides + flat caps
    /// for cel-shading. Used for pillars, towers, silos, road
    /// markers, gas-station columns.
    ///
    /// `sides` controls the polygon count; default 8 gives readable
    /// octagonal silhouette without overtessellating.
    #[must_use]
    pub fn primitive_cylinder(sides: u32) -> Self {
        let sides = sides.max(3);
        let mut mesh = Self::new("primitive_cylinder");
        let top_y = 0.5_f32;
        let bottom_y = -0.5_f32;
        let radius = 0.5_f32;
        let color = [1.0_f32, 1.0, 1.0, 1.0];

        // Side quads — flat-faceted (single normal per quad, replicated
        // on its 4 vertices). Winding:
        //   top[i] → bottom[i] → bottom[i+1] → top[i+1]
        // as seen from outside the cylinder is CCW.
        for i in 0..sides {
            let theta0 = (i as f32) * core::f32::consts::TAU / sides as f32;
            let theta1 = ((i + 1) as f32) * core::f32::consts::TAU / sides as f32;
            let t0 = Vec3::new(radius * theta0.cos(), top_y, radius * theta0.sin());
            let b0 = Vec3::new(radius * theta0.cos(), bottom_y, radius * theta0.sin());
            let t1 = Vec3::new(radius * theta1.cos(), top_y, radius * theta1.sin());
            let b1 = Vec3::new(radius * theta1.cos(), bottom_y, radius * theta1.sin());
            // Outward radial normal at the midpoint of this face.
            let mid_theta = (theta0 + theta1) * 0.5;
            let normal = Vec3::new(mid_theta.cos(), 0.0, mid_theta.sin());

            let start = mesh.vertices.len() as u32;
            let u_left = i as f32 / sides as f32;
            let u_right = (i + 1) as f32 / sides as f32;
            for (pos, uv) in [
                (t0, Vec2::new(u_left, 0.0)),
                (b0, Vec2::new(u_left, 1.0)),
                (b1, Vec2::new(u_right, 1.0)),
                (t1, Vec2::new(u_right, 0.0)),
            ] {
                mesh.vertices.push(Vertex {
                    position: [pos.x, pos.y, pos.z],
                    normal: [normal.x, normal.y, normal.z],
                    uv: [uv.x, uv.y],
                    color,
                });
            }
            // Two triangles per quad: (t0, b0, b1) + (t0, b1, t1) = CCW
            // from outside.
            mesh.indices.extend_from_slice(&[
                start,
                start + 1,
                start + 2,
                start,
                start + 2,
                start + 3,
            ]);
        }

        // Top cap (+Y normal). Fan from center. Winding:
        // center → ring[i] → ring[(i+1) % sides] gives +Y CCW.
        let top_center_idx = mesh.vertices.len() as u32;
        mesh.vertices.push(Vertex {
            position: [0.0, top_y, 0.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.5, 0.5],
            color,
        });
        let top_ring_start = mesh.vertices.len() as u32;
        for i in 0..sides {
            let theta = (i as f32) * core::f32::consts::TAU / sides as f32;
            mesh.vertices.push(Vertex {
                position: [radius * theta.cos(), top_y, radius * theta.sin()],
                normal: [0.0, 1.0, 0.0],
                uv: [0.5 + theta.cos() * 0.5, 0.5 + theta.sin() * 0.5],
                color,
            });
        }
        for i in 0..sides {
            mesh.indices.push(top_center_idx);
            mesh.indices.push(top_ring_start + i);
            mesh.indices.push(top_ring_start + ((i + 1) % sides));
        }

        // Bottom cap (-Y normal). Reversed winding:
        // center → ring[(i+1) % sides] → ring[i] gives -Y CCW.
        let bot_center_idx = mesh.vertices.len() as u32;
        mesh.vertices.push(Vertex {
            position: [0.0, bottom_y, 0.0],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5, 0.5],
            color,
        });
        let bot_ring_start = mesh.vertices.len() as u32;
        for i in 0..sides {
            let theta = (i as f32) * core::f32::consts::TAU / sides as f32;
            mesh.vertices.push(Vertex {
                position: [radius * theta.cos(), bottom_y, radius * theta.sin()],
                normal: [0.0, -1.0, 0.0],
                uv: [0.5 + theta.cos() * 0.5, 0.5 + theta.sin() * 0.5],
                color,
            });
        }
        for i in 0..sides {
            mesh.indices.push(bot_center_idx);
            mesh.indices.push(bot_ring_start + ((i + 1) % sides));
            mesh.indices.push(bot_ring_start + i);
        }

        mesh
    }

    /// Unit-size primitive half-cylinder for instance rendering
    /// (Phase 3.6.3c.5c.2.c.1).
    ///
    /// Quonset-hut / hangar-roof shape. The cylinder is split along the
    /// XZ plane (y=0): the upper half (y ≥ 0) is the curved roof, the
    /// bottom (y=0) is a flat rectangular floor. The length axis runs
    /// along Z (from z=-0.5 to z=+0.5); the rounded arc spans from
    /// x=-0.5 (arc start) up over the top (y=+0.5) to x=+0.5 (arc
    /// end). Unit diameter × unit length; per-instance scale shapes it.
    ///
    /// Faces (all CCW-from-outside):
    /// - curved arc → N flat-faceted quads along the length
    /// - flat rectangular base → one quad with -Y normal
    /// - two semicircular end caps (±Z) → fans from cap center, each
    ///   with a single axial normal
    ///
    /// `sides` controls the semicircle segmentation; default 8 gives a
    /// readable octagonal-ish arc.
    #[must_use]
    pub fn primitive_half_cylinder(sides: u32) -> Self {
        let sides = sides.max(3);
        let mut mesh = Self::new("primitive_half_cylinder");
        let half_len = 0.5_f32;
        let radius = 0.5_f32;
        let color = [1.0_f32, 1.0, 1.0, 1.0];

        // Arc angles go from 0 to π (not 0 to TAU). Theta=0 is +X;
        // theta=π is -X; theta=π/2 is +Y (top of dome).
        //
        // Curved side quads — flat-faceted. Winding per quad (CCW from
        // outside): near_top → near_bottom → far_bottom → far_top,
        // where "top" means the more-recent arc position (higher index).
        //
        // Actually we need to be careful: walking theta from 0 (at +X)
        // toward π (at -X), consecutive arc points go counterclockwise
        // as seen from +Z (looking in -Z direction). For a quad facing
        // outward radially, the visible side's winding is:
        //   near[i]  → far[i] → far[i+1] → near[i+1]
        // where near = Z=-0.5 end (closer to viewer from +Z), far = Z=+0.5.
        for i in 0..sides {
            let theta0 = (i as f32) * core::f32::consts::PI / sides as f32;
            let theta1 = ((i + 1) as f32) * core::f32::consts::PI / sides as f32;
            let n_bot_start = Vec3::new(radius * theta0.cos(), radius * theta0.sin(), -half_len);
            let n_bot_end = Vec3::new(radius * theta1.cos(), radius * theta1.sin(), -half_len);
            let f_bot_start = Vec3::new(radius * theta0.cos(), radius * theta0.sin(), half_len);
            let f_bot_end = Vec3::new(radius * theta1.cos(), radius * theta1.sin(), half_len);
            let mid_theta = (theta0 + theta1) * 0.5;
            let normal = Vec3::new(mid_theta.cos(), mid_theta.sin(), 0.0);

            let start = mesh.vertices.len() as u32;
            let u0 = i as f32 / sides as f32;
            let u1 = (i + 1) as f32 / sides as f32;
            for (pos, uv) in [
                (n_bot_start, Vec2::new(u0, 0.0)),
                (f_bot_start, Vec2::new(u0, 1.0)),
                (f_bot_end, Vec2::new(u1, 1.0)),
                (n_bot_end, Vec2::new(u1, 0.0)),
            ] {
                mesh.vertices.push(Vertex {
                    position: [pos.x, pos.y, pos.z],
                    normal: [normal.x, normal.y, normal.z],
                    uv: [uv.x, uv.y],
                    color,
                });
            }
            // Two tris per quad: (0, 1, 2) + (0, 2, 3).
            mesh.indices.extend_from_slice(&[
                start,
                start + 1,
                start + 2,
                start,
                start + 2,
                start + 3,
            ]);
        }

        // Flat rectangular base (-Y normal). Four corners at y=0,
        // spanning x=±radius, z=±half_len.
        // Winding CCW from below (outside): we want the normal to
        // point -Y, so when viewed from -Y (looking up), go CW.
        // From +Y (looking down), order is: (+X,+Z) → (-X,+Z) → (-X,-Z) → (+X,-Z)
        // Reverse for -Y facing: (+X,-Z) → (-X,-Z) → (-X,+Z) → (+X,+Z)
        let base_start = mesh.vertices.len() as u32;
        let base_normal = [0.0, -1.0, 0.0];
        for (pos, uv) in [
            (Vec3::new(radius, 0.0, -half_len), Vec2::new(1.0, 0.0)),
            (Vec3::new(-radius, 0.0, -half_len), Vec2::new(0.0, 0.0)),
            (Vec3::new(-radius, 0.0, half_len), Vec2::new(0.0, 1.0)),
            (Vec3::new(radius, 0.0, half_len), Vec2::new(1.0, 1.0)),
        ] {
            mesh.vertices.push(Vertex {
                position: [pos.x, pos.y, pos.z],
                normal: base_normal,
                uv: [uv.x, uv.y],
                color,
            });
        }
        mesh.indices.extend_from_slice(&[
            base_start,
            base_start + 1,
            base_start + 2,
            base_start,
            base_start + 2,
            base_start + 3,
        ]);

        // End caps — semicircular fans.
        //
        // Far cap at z=+half_len, normal +Z. Winding: center → arc[i]
        // → arc[i+1] is CCW as seen from +Z (outside).
        //
        // Near cap at z=-half_len, normal -Z. Reversed winding:
        // center → arc[i+1] → arc[i].
        for (z_val, normal, reversed) in [
            (half_len, [0.0_f32, 0.0, 1.0_f32], false),
            (-half_len, [0.0, 0.0, -1.0], true),
        ] {
            let center_idx = mesh.vertices.len() as u32;
            mesh.vertices.push(Vertex {
                position: [0.0, 0.0, z_val],
                normal,
                uv: [0.5, 0.5],
                color,
            });
            // Include the two base-level endpoints (x=±radius at y=0)
            // AND the arc points (sides+1 points from 0 to π).
            let arc_start = mesh.vertices.len() as u32;
            // Add the +X base corner first (theta=0).
            // Then arc points at theta = dtheta, 2*dtheta, ..., π-dtheta.
            // Then the -X base corner last (theta=π).
            for i in 0..=sides {
                let theta = (i as f32) * core::f32::consts::PI / sides as f32;
                let x = radius * theta.cos();
                let y = radius * theta.sin();
                mesh.vertices.push(Vertex {
                    position: [x, y, z_val],
                    normal,
                    uv: [0.5 + x, 0.5 - y], // arbitrary; cel shading uses normal, not UV
                    color,
                });
            }
            // Also include the base endpoints so the fan is closed.
            // Base endpoints are already at i=0 (+X, y=0) and i=sides
            // (-X, y=0) of the arc loop — perfect.
            // Fan triangles: center → arc[i] → arc[i+1] (or reversed).
            for i in 0..sides {
                mesh.indices.push(center_idx);
                let (b, c) = if reversed {
                    (arc_start + i + 1, arc_start + i)
                } else {
                    (arc_start + i, arc_start + i + 1)
                };
                mesh.indices.push(b);
                mesh.indices.push(c);
            }
        }

        mesh
    }

    /// Unit-size primitive flat quad for instance rendering
    /// (Phase 3.6.3c.5c.2.c.1).
    ///
    /// Single axis-aligned quad in the XZ plane at y=0, spanning
    /// x=[-0.5, +0.5] × z=[-0.5, +0.5]. Normal points +Y. Used for
    /// runway surfaces, tarmac / apron slabs, road segments, signage
    /// panels, anything that's essentially a 2D patch.
    ///
    /// Because the quad has zero thickness, `base_size_m.y` in a
    /// [`crate::primitive_component_doc_stub`]-style RON composition
    /// encodes anti-z-fight lift above ground rather than height —
    /// e.g. `(30.0, 0.1, 600.0)` = 30 m × 600 m runway floating 5 cm
    /// above terrain to avoid flickering where depth buffer precision
    /// tiebreaks in the terrain's favor.
    ///
    /// Only the +Y face is drawn; callers should not rotate the quad
    /// below horizontal (the backside isn't visible).
    #[must_use]
    pub fn primitive_flat_quad() -> Self {
        let mut mesh = Self::new("primitive_flat_quad");
        let half = 0.5_f32;
        let color = [1.0_f32, 1.0, 1.0, 1.0];
        let normal = [0.0_f32, 1.0, 0.0];

        // Winding CCW from +Y (above): (-X,-Z) → (+X,-Z) → (+X,+Z) → (-X,+Z).
        for (pos, uv) in [
            (Vec3::new(-half, 0.0, -half), Vec2::new(0.0, 0.0)),
            (Vec3::new(half, 0.0, -half), Vec2::new(1.0, 0.0)),
            (Vec3::new(half, 0.0, half), Vec2::new(1.0, 1.0)),
            (Vec3::new(-half, 0.0, half), Vec2::new(0.0, 1.0)),
        ] {
            mesh.vertices.push(Vertex {
                position: [pos.x, pos.y, pos.z],
                normal,
                uv: [uv.x, uv.y],
                color,
            });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        mesh
    }

    /// Unit-size primitive sphere for instance rendering
    /// (Phase 3.6.3c.5c.2).
    ///
    /// Unit diameter (radius 0.5) centered at origin. Vertex color
    /// white. UV-sphere construction with smooth per-vertex normals —
    /// unlike the faceted box/cone/cylinder, spheres benefit from
    /// smooth normals for cel-shading's banded transitions curving
    /// around the surface. Used for boulders, floats, spherical
    /// building features.
    ///
    /// `lon_segments` + `lat_segments` default to 12 and 8. Raising
    /// either costs ~quadratic vertex count; keep low for distant
    /// LODs in 5c.2+.
    #[must_use]
    pub fn primitive_sphere(lon_segments: u32, lat_segments: u32) -> Self {
        let lon = lon_segments.max(3);
        let lat = lat_segments.max(2);
        let mut mesh = Self::new("primitive_sphere");
        let radius = 0.5_f32;
        let color = [1.0_f32, 1.0, 1.0, 1.0];

        // Generate vertices in a grid: lat=0 is top pole, lat=lat is
        // bottom pole. Longitude wraps, so emit lon+1 columns with the
        // last matching the first (seamless UV + normals).
        for lat_i in 0..=lat {
            let phi = (lat_i as f32) * core::f32::consts::PI / (lat as f32);
            let y = radius * phi.cos();
            let ring_r = radius * phi.sin();
            for lon_i in 0..=lon {
                let theta = (lon_i as f32) * core::f32::consts::TAU / (lon as f32);
                let x = ring_r * theta.cos();
                let z = ring_r * theta.sin();
                let pos = Vec3::new(x, y, z);
                let normal = pos.normalize_or_zero();
                mesh.vertices.push(Vertex {
                    position: [pos.x, pos.y, pos.z],
                    normal: [normal.x, normal.y, normal.z],
                    uv: [lon_i as f32 / lon as f32, lat_i as f32 / lat as f32],
                    color,
                });
            }
        }

        // Indices: quads between latitude rings, wound CCW from
        // outside. Pole caps degenerate into triangles naturally
        // because the top/bottom rings have all-the-same y vertices.
        let row = lon + 1;
        for lat_i in 0..lat {
            for lon_i in 0..lon {
                let tl = lat_i * row + lon_i;
                let tr = lat_i * row + lon_i + 1;
                let bl = (lat_i + 1) * row + lon_i;
                let br = (lat_i + 1) * row + lon_i + 1;
                // Two triangles per quad. Winding: tl → bl → br → tr
                // is CCW from outside (looking down the radial axis
                // with +Y up).
                mesh.indices.extend_from_slice(&[tl, bl, br, tl, br, tr]);
            }
        }

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
    fn primitive_cone_has_expected_vertex_count() {
        // Sides=8: 8 side triangles × 3 unique verts = 24 side verts
        //        + 1 base center + 8 base ring = 33 verts.
        // Indices: 8 side tris × 3 + 8 base tris × 3 = 48.
        let m = CpuMesh::primitive_cone(8);
        assert_eq!(m.vertex_count(), 33);
        assert_eq!(m.index_count(), 48);
    }

    #[test]
    fn primitive_cone_apex_is_above_base() {
        let m = CpuMesh::primitive_cone(8);
        let max_y = m
            .vertices
            .iter()
            .map(|v| v.position[1])
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = m
            .vertices
            .iter()
            .map(|v| v.position[1])
            .fold(f32::INFINITY, f32::min);
        assert!((max_y - 0.5).abs() < 1e-5, "apex at y=0.5");
        assert!((min_y - -0.5).abs() < 1e-5, "base at y=-0.5");
    }

    #[test]
    fn primitive_cone_side_normals_have_outward_xz_component() {
        // Side normals should not point straight up/down — they tilt
        // outward + up. Base vertices have normal [0, -1, 0].
        let m = CpuMesh::primitive_cone(8);
        let mut side_count = 0;
        for v in &m.vertices {
            let n = v.normal;
            if n[1] > 0.0 && (n[0].powi(2) + n[2].powi(2)) > 0.01 {
                side_count += 1;
            }
        }
        // 8 sides × 3 verts (apex + 2 base) = 24 side-normal vertices.
        assert_eq!(side_count, 24);
    }

    #[test]
    fn primitive_cylinder_has_expected_vertex_count() {
        // Sides=8: 8 side quads × 4 unique verts = 32 side verts
        //        + (1 + 8) top cap + (1 + 8) bottom cap = 50 verts.
        // Indices: 8 side quads × 6 + 8 top tris × 3 + 8 bottom tris × 3 = 96.
        let m = CpuMesh::primitive_cylinder(8);
        assert_eq!(m.vertex_count(), 50);
        assert_eq!(m.index_count(), 96);
    }

    #[test]
    fn primitive_cylinder_bounds_are_unit() {
        let m = CpuMesh::primitive_cylinder(8);
        for v in &m.vertices {
            assert!(v.position[0].abs() <= 0.5 + 1e-5);
            assert!(v.position[1].abs() <= 0.5 + 1e-5);
            assert!(v.position[2].abs() <= 0.5 + 1e-5);
        }
    }

    #[test]
    fn primitive_sphere_has_smooth_normals() {
        // Every vertex's normal should equal its position direction
        // (sphere of radius 0.5, normals are position / 0.5 ≈ unit).
        let m = CpuMesh::primitive_sphere(12, 8);
        for v in &m.vertices {
            let pos = Vec3::new(v.position[0], v.position[1], v.position[2]);
            let n = Vec3::new(v.normal[0], v.normal[1], v.normal[2]);
            if pos.length() > 1e-4 {
                let expected = pos.normalize();
                assert!(
                    (n - expected).length() < 1e-4,
                    "vertex normal != position dir"
                );
            }
        }
    }

    #[test]
    fn primitive_half_cylinder_bounds() {
        // Half-cylinder spans x=[-0.5, +0.5], y=[0, +0.5], z=[-0.5, +0.5].
        let m = CpuMesh::primitive_half_cylinder(8);
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for v in &m.vertices {
            assert!(v.position[0].abs() <= 0.5 + 1e-5);
            assert!(v.position[2].abs() <= 0.5 + 1e-5);
            min_y = min_y.min(v.position[1]);
            max_y = max_y.max(v.position[1]);
        }
        assert!((min_y - 0.0).abs() < 1e-5, "bottom at y=0 (flat base)");
        assert!((max_y - 0.5).abs() < 1e-5, "peak at y=+0.5 (arc top)");
    }

    #[test]
    fn primitive_half_cylinder_has_base_normal_down() {
        // The flat base should have exactly 4 vertices whose normal
        // matches (0, -1, 0). Side / end-cap vertices have different
        // normals.
        let m = CpuMesh::primitive_half_cylinder(8);
        let base_count = m
            .vertices
            .iter()
            .filter(|v| {
                v.normal[0].abs() < 1e-5
                    && (v.normal[1] - -1.0).abs() < 1e-5
                    && v.normal[2].abs() < 1e-5
            })
            .count();
        assert_eq!(base_count, 4, "exactly 4 base-quad vertices face -Y");
    }

    #[test]
    fn primitive_flat_quad_is_unit_xz_plane() {
        let m = CpuMesh::primitive_flat_quad();
        assert_eq!(m.vertex_count(), 4);
        assert_eq!(m.index_count(), 6);
        for v in &m.vertices {
            assert!(v.position[1].abs() < 1e-5, "quad is flat at y=0");
            assert!(v.normal[0].abs() < 1e-5, "normal x is 0");
            assert!((v.normal[1] - 1.0).abs() < 1e-5, "normal y is 1");
            assert!(v.normal[2].abs() < 1e-5, "normal z is 0");
            assert!(v.position[0].abs() <= 0.5 + 1e-5);
            assert!(v.position[2].abs() <= 0.5 + 1e-5);
        }
    }

    #[test]
    fn primitive_sphere_vertex_count_grid() {
        // (lon+1) * (lat+1) = 13 * 9 = 117 verts.
        let m = CpuMesh::primitive_sphere(12, 8);
        assert_eq!(m.vertex_count(), 117);
        // Indices: lon × lat × 6 = 12 × 8 × 6 = 576.
        assert_eq!(m.index_count(), 576);
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
