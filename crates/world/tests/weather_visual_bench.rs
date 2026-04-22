//! Weather visual-bench integration test (3.2.1 Layer W1).
//!
//! Emits PNG artifacts per weather field at three crop scales (full /
//! quadrant / player-local) for multi-scale visual inspection. See
//! [`TEST_PLAN.md § World-Gen Visual Bench`](../../../TEST_PLAN.md#world-gen-visual-bench)
//! for the pattern.
//!
//! # Running
//!
//! Default: tests here are `#[ignore]` so `cargo test --workspace` doesn't
//! pay the PNG-rendering cost on every run.
//!
//! Opt-in: `cargo test -p flowplan-world --test weather_visual_bench -- --ignored`
//!
//! # Output
//!
//! PNGs written to `target/world-gen-plots/weather/<seed>/`. Committed
//! reference artifacts live at `docs/world-gen-plots/weather/<seed>/`.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
// HSV conversion uses canonical single-letter names (c, x, m, r, g, b).
#![allow(clippy::many_single_char_names)]
// Test harness: panicking on I/O error is the appropriate failure mode.
#![allow(clippy::expect_used)]

use std::path::Path;

use flowplan_world::terrain::Terrain;
use flowplan_world::weather::{
    cell_state, frontal, humidity, pressure, temperature, turbulence, voronoi, wind,
};
use image::{ImageBuffer, Rgb};

// Silence unused-crate-dependencies lint for transitive deps the integration
// test target sees but doesn't itself name directly.
use flowplan_core as _;
use glam as _;

/// Full-world-scenario extent (m). 500 km — representative small region.
const FULL_EXTENT_M: f32 = 500_000.0;

/// Player-local crop extent (m). Matches ~10 km streaming disk outer radius.
const PLAYER_EXTENT_M: f32 = 10_000.0;

/// PNG resolution (px per side).
const RESOLUTION: u32 = 1024;

/// Canonical seed used by all visual-bench scenarios.
const CANONICAL_SEED: u64 = 0xF10_1A11_u64;

#[derive(Copy, Clone)]
struct Crop {
    name: &'static str,
    extent_m: f32,
    center_x_m: f32,
    center_z_m: f32,
}

impl Crop {
    const FULL: Self = Self {
        name: "full",
        extent_m: FULL_EXTENT_M,
        center_x_m: 0.0,
        center_z_m: 0.0,
    };

    const QUADRANT: Self = Self {
        name: "quadrant",
        extent_m: FULL_EXTENT_M / 2.0,
        // Top-right of a centered-at-origin full extent.
        center_x_m: FULL_EXTENT_M / 4.0,
        center_z_m: -FULL_EXTENT_M / 4.0,
    };

    const PLAYER: Self = Self {
        name: "player",
        extent_m: PLAYER_EXTENT_M,
        center_x_m: 0.0,
        center_z_m: 0.0,
    };

    fn world_coord(self, px: u32, py: u32) -> (f32, f32) {
        let u = (px as f32 + 0.5) / RESOLUTION as f32;
        let v = (py as f32 + 0.5) / RESOLUTION as f32;
        let x = self.center_x_m + (u - 0.5) * self.extent_m;
        // Image y grows down; world z grows south. Flip v so north is at
        // the top of the image.
        let z = self.center_z_m + (0.5 - v) * self.extent_m;
        (x, z)
    }
}

const CROPS: &[Crop] = &[Crop::FULL, Crop::QUADRANT, Crop::PLAYER];

/// Map normalized `[0, 1]` → RGB via a perceptually-flat cool→warm ramp.
fn ramp_rgb(t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let (r, g, b) = if t < 1.0 / 3.0 {
        let u = t * 3.0;
        (0.0, u, 1.0) // blue → cyan
    } else if t < 2.0 / 3.0 {
        let u = (t - 1.0 / 3.0) * 3.0;
        (u, 1.0, 1.0 - u) // cyan → yellow
    } else {
        let u = (t - 2.0 / 3.0) * 3.0;
        (1.0, 1.0 - u, 0.0) // yellow → red
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

fn render_field<F: Fn(f32, f32) -> f32 + Sync>(
    crop: Crop,
    value_range: (f32, f32),
    sample: F,
) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
    let (lo, hi) = value_range;
    let span = hi - lo;
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = crop.world_coord(px, py);
            let value = sample(x, z);
            let normalized = ((value - lo) / span).clamp(0.0, 1.0);
            let rgb = ramp_rgb(normalized);
            img.put_pixel(px, py, Rgb(rgb));
        }
    }
    img
}

fn output_dir() -> std::path::PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| std::path::PathBuf::from("target"), Into::into);
    target
        .join("world-gen-plots/weather")
        .join(format!("{CANONICAL_SEED:x}"))
}

fn write_png(img: &ImageBuffer<Rgb<u8>, Vec<u8>>, dir: &Path, filename: &str) {
    std::fs::create_dir_all(dir).expect("create output dir");
    let path = dir.join(filename);
    img.save(&path)
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!("wrote {}", path.display());
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_pressure_field_artifacts() {
    let dir = output_dir();
    let value_range = (
        pressure::BASELINE_HPA - pressure::SYNOPTIC_AMPLITUDE_HPA,
        pressure::BASELINE_HPA + pressure::SYNOPTIC_AMPLITUDE_HPA,
    );
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            pressure::sea_level_pressure(CANONICAL_SEED, x, z, 0.0)
        });
        write_png(&img, &dir, &format!("pressure_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_temperature_field_artifacts() {
    let dir = output_dir();
    let value_range = (
        temperature::BASELINE_KELVIN
            - temperature::CLIMATE_AMPLITUDE_K
            - temperature::SYNOPTIC_AMPLITUDE_K
            - temperature::DIURNAL_AMPLITUDE_K,
        temperature::BASELINE_KELVIN
            + temperature::CLIMATE_AMPLITUDE_K
            + temperature::SYNOPTIC_AMPLITUDE_K
            + temperature::DIURNAL_AMPLITUDE_K,
    );
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            // Noon of day 0 — captures the diurnal peak.
            temperature::sea_level_temperature(CANONICAL_SEED, x, z, 12.0 * 3_600.0)
        });
        write_png(&img, &dir, &format!("temperature_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_humidity_field_artifacts() {
    let dir = output_dir();
    let value_range = (0.0, 1.0);
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            humidity::sea_level_humidity(CANONICAL_SEED, x, z, 0.0)
        });
        write_png(&img, &dir, &format!("humidity_{}.png", crop.name));
    }
}

/// HSV → RGB. Hue in radians ∈ `(-π, π]`; sat/val in `[0, 1]`.
fn hsv_to_rgb(hue_rad: f32, sat: f32, val: f32) -> [u8; 3] {
    let hue_deg = (hue_rad.to_degrees() + 360.0) % 360.0;
    let sector = hue_deg / 60.0;
    let c = val * sat;
    let x = c * (1.0 - ((sector % 2.0) - 1.0).abs());
    let m = val - c;
    let (r, g, b) = match sector as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    ]
}

/// Render a wind field plot: hue = direction, value = magnitude / `max_mag`.
fn render_wind(crop: Crop, max_mag: f32) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = crop.world_coord(px, py);
            let w = wind::wind_at(CANONICAL_SEED, x, 0.0, z, 0.0);
            let mag = libm::sqrtf(w.x * w.x + w.z * w.z);
            let dir = libm::atan2f(w.z, w.x);
            let value = (mag / max_mag).clamp(0.0, 1.0);
            let rgb = hsv_to_rgb(dir, 1.0, value);
            img.put_pixel(px, py, Rgb(rgb));
        }
    }
    img
}

/// Draw a line segment into the image via Bresenham's algorithm.
fn draw_line(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    (x0, y0): (i32, i32),
    (x1, y1): (i32, i32),
    color: [u8; 3],
) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    let (w, h) = (RESOLUTION as i32, RESOLUTION as i32);
    loop {
        if (0..w).contains(&x) && (0..h).contains(&y) {
            img.put_pixel(x as u32, y as u32, Rgb(color));
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// Map world coord to pixel coord within a crop. Returns (px, py) or `None` if outside.
fn world_to_pixel(crop: Crop, x_m: f32, z_m: f32) -> Option<(i32, i32)> {
    let u = (x_m - crop.center_x_m) / crop.extent_m + 0.5;
    let v = 0.5 - (z_m - crop.center_z_m) / crop.extent_m;
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return None;
    }
    Some((
        (u * RESOLUTION as f32) as i32,
        (v * RESOLUTION as f32) as i32,
    ))
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_wind_field_artifacts() {
    let dir = output_dir();
    // Upper bound on synoptic surface wind — chosen to match test-asserted
    // plausible range (~25 m/s).
    let max_mag = 25.0;
    for &crop in CROPS {
        let img = render_wind(crop, max_mag);
        write_png(&img, &dir, &format!("wind_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_turbulence_field_artifacts() {
    let dir = output_dir();
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let value_range = (0.0, turbulence::MAX_TURBULENCE_INTENSITY);
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            // Sample at ground level — terrain-turbulence peaks near surface.
            turbulence::turbulence_at(CANONICAL_SEED, &terrain, x, 100.0, z, 0.0)
        });
        write_png(&img, &dir, &format!("turbulence_{}.png", crop.name));
    }
}

/// Hash a `CellId` to a stable RGB color.
fn cell_color(cell_id: voronoi::CellId) -> [u8; 3] {
    #[allow(clippy::cast_sign_loss)]
    let mut h = (cell_id.ix as u32 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    #[allow(clippy::cast_sign_loss)]
    let h2 = (cell_id.iz as u32 as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ h2).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    // Extract 3 bytes for RGB.
    let r = ((h >> 16) & 0xff) as u8;
    let g = ((h >> 32) & 0xff) as u8;
    let b = ((h >> 48) & 0xff) as u8;
    [r, g, b]
}

/// Canonical palette for cell types — mnemonic weather-map conventions.
fn cell_type_color(t: cell_state::CellType) -> [u8; 3] {
    match t {
        cell_state::CellType::ClearAir => [180, 200, 230], // pale sky blue
        cell_state::CellType::Cumulus => [240, 240, 240],  // white
        cell_state::CellType::Nimbus => [70, 70, 90],      // dark slate
        cell_state::CellType::Stratus => [170, 170, 180],  // light gray
        cell_state::CellType::Squall => [220, 180, 40],    // amber
        cell_state::CellType::Frontal => [200, 50, 80],    // magenta-red
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_cell_type_artifacts() {
    let dir = output_dir();
    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let (x, z) = crop.world_coord(px, py);
                let lookup = voronoi::cell_at(CANONICAL_SEED, x, z);
                let state = cell_state::initial_state(CANONICAL_SEED, lookup.cell_id, 0.0);
                let mut color = cell_type_color(state.cell_type);
                // Feather cell edges 500 m toward black to reveal partition.
                let edge_pixels = (500.0 / crop.extent_m) * RESOLUTION as f32;
                if lookup.distance_to_edge < 500.0 && edge_pixels > 1.0 {
                    let t = lookup.distance_to_edge / 500.0;
                    for c in &mut color {
                        *c = (f32::from(*c) * t) as u8;
                    }
                }
                img.put_pixel(px, py, Rgb(color));
            }
        }
        write_png(&img, &dir, &format!("cell_type_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_voronoi_partition_artifacts() {
    let dir = output_dir();
    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let (x, z) = crop.world_coord(px, py);
                let lookup = voronoi::cell_at(CANONICAL_SEED, x, z);
                let mut color = cell_color(lookup.cell_id);
                // Darken within ~500 m of a cell edge to reveal the partition
                // structure; crops smaller than the cell size benefit most from
                // this edge overlay.
                let edge_pixels = (500.0 / crop.extent_m) * RESOLUTION as f32;
                if lookup.distance_to_edge < 500.0 && edge_pixels > 1.0 {
                    let t = lookup.distance_to_edge / 500.0;
                    for c in &mut color {
                        *c = (f32::from(*c) * t) as u8;
                    }
                }
                img.put_pixel(px, py, Rgb(color));
            }
        }
        write_png(&img, &dir, &format!("voronoi_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_frontal_graph_artifacts() {
    let dir = output_dir();
    // Underlay the pressure field; overlay frontal segments in black.
    let value_range = (
        pressure::BASELINE_HPA - pressure::SYNOPTIC_AMPLITUDE_HPA,
        pressure::BASELINE_HPA + pressure::SYNOPTIC_AMPLITUDE_HPA,
    );
    for &crop in CROPS {
        let mut img = render_field(crop, value_range, |x, z| {
            pressure::sea_level_pressure(CANONICAL_SEED, x, z, 0.0)
        });
        let segs = frontal::frontal_segments(
            CANONICAL_SEED,
            crop.center_x_m,
            crop.center_z_m,
            crop.extent_m / 2.0,
            // Spacing scales with crop so each plot has ~25 × 25 cells.
            crop.extent_m / 25.0,
            0.0,
        );
        for s in &segs {
            let start = world_to_pixel(crop, s.start.x, s.start.y);
            let end = world_to_pixel(crop, s.end.x, s.end.y);
            if let (Some(p0), Some(p1)) = (start, end) {
                draw_line(&mut img, p0, p1, [0, 0, 0]);
            }
        }
        write_png(&img, &dir, &format!("frontal_{}.png", crop.name));
    }
}
