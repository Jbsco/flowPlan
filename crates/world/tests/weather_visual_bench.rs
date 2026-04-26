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
    self, cell_state, frontal, humidity, pressure, temperature, turbulence, voronoi, wind,
};
use image::{ImageBuffer, Rgb};

// Silence unused-crate-dependencies lint for transitive deps the integration
// test target sees but doesn't itself name directly.
use bincode as _;
use flowplan_core as _;
use glam as _;
use rayon as _;
use ron as _;
use serde as _;
use tracing as _;

/// Full-world-scenario extent (m). 500 km — representative small region.
const FULL_EXTENT_M: f32 = 500_000.0;

/// Continental-scale extent (m). 2000 km — previews Option A world-scale weather
/// seamlessness; emitted only for the synoptic overlay as a cheap "does this
/// stay continuous?" sanity check.
const CONTINENTAL_EXTENT_M: f32 = 2_000_000.0;

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

    const CONTINENTAL: Self = Self {
        name: "continental",
        extent_m: CONTINENTAL_EXTENT_M,
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

/// Synoptic-overlay wind-arrow grid density (arrows per side).
const ARROW_COUNT: u32 = 24;

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

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_humidity_terrain_field_artifacts() {
    // Humidity with orographic + rain-shadow coupling applied. Compare
    // side-by-side against `humidity_*.png` to see windward boosts / lee
    // desiccation over mountainous terrain.
    let dir = output_dir();
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let value_range = (0.0, 1.0);
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            humidity::sea_level_humidity_with_terrain(CANONICAL_SEED, &terrain, x, z, 0.0)
        });
        write_png(&img, &dir, &format!("humidity_terrain_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_cloud_density_field_artifacts() {
    // Cloud density sampled at 2 km altitude (typical cumulus mid-layer).
    // Dark = clear air / below cell base / above top; bright = inside a
    // cloud column.
    let dir = output_dir();
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let value_range = (0.0, 1.0);
    let sample_altitude = 2_000.0;
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            weather::deltas::cloud_density_at(CANONICAL_SEED, &terrain, x, sample_altitude, z, 0.0)
        });
        write_png(&img, &dir, &format!("cloud_density_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_precipitation_field_artifacts() {
    // Precipitation rate at ground (50 m AGL — "below cloud base" for any
    // precipitating cell). Dark = dry; bright = heavy rain.
    let dir = output_dir();
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let value_range = (0.0, 15.0); // 0 to heavy-rain reference
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            weather::deltas::precipitation_rate_at(CANONICAL_SEED, &terrain, x, 50.0, z, 0.0)
        });
        write_png(&img, &dir, &format!("precipitation_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_visibility_field_artifacts() {
    // Horizontal visibility at 500 m AGL. Bright = clear baseline ~50 km;
    // dark = inside cloud / heavy precip. Clamped to the cloud-interior floor.
    let dir = output_dir();
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let value_range = (
        weather::deltas::CLOUD_INTERIOR_VISIBILITY_M,
        weather::deltas::BASELINE_VISIBILITY_M,
    );
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            weather::deltas::visibility_at(CANONICAL_SEED, &terrain, x, 500.0, z, 0.0)
        });
        write_png(&img, &dir, &format!("visibility_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_turbulence_composed_field_artifacts() {
    // W1 + W3 composed turbulence. Comparable to `turbulence_*.png`
    // (W1 baseline); active weather cells add visible peaks.
    let dir = output_dir();
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let value_range = (0.0, turbulence::MAX_TURBULENCE_INTENSITY);
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            weather::deltas::turbulence_at_with_weather(CANONICAL_SEED, &terrain, x, 500.0, z, 0.0)
        });
        write_png(
            &img,
            &dir,
            &format!("turbulence_composed_{}.png", crop.name),
        );
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_orographic_modifier_field_artifacts() {
    // The raw multiplicative modifier — centered at 1.0. Blue (<1) = lee-side
    // dry bias, red (>1) = windward-side moist bias. Gray (≈1) = no terrain
    // effect (low elevation, flat, or calm wind).
    let dir = output_dir();
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let value_range = (
        1.0 - weather::orographic::MAX_BIAS,
        1.0 + weather::orographic::MAX_BIAS,
    );
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            weather::orographic::terrain_moisture_modifier(CANONICAL_SEED, &terrain, x, z, 0.0)
        });
        write_png(
            &img,
            &dir,
            &format!("orographic_modifier_{}.png", crop.name),
        );
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

/// Shared cell-type rendering helper: resolves each pixel to a cell state and
/// renders via the weather-map palette with edge feathering. `time_s` selects
/// between "initial" (pass 0.0) and "evolved" (pass a future time) semantics —
/// `state_source` decides which API to query.
fn render_cell_type_map(
    crop: Crop,
    time_s: f32,
    state_source: fn(u64, voronoi::CellId, f32) -> cell_state::CellState,
) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = crop.world_coord(px, py);
            let lookup = voronoi::cell_at(CANONICAL_SEED, x, z);
            let state = state_source(CANONICAL_SEED, lookup.cell_id, time_s);
            let mut color = cell_type_color(state.cell_type);
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
    img
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_cell_type_artifacts() {
    let dir = output_dir();
    for &crop in CROPS {
        let img = render_cell_type_map(crop, 0.0, cell_state::initial_state);
        write_png(&img, &dir, &format!("cell_type_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_cell_type_evolved_artifacts() {
    // Snapshot at t = 3 hours (half a cycle). Cells with different temporal
    // offsets will be in different life-cycle phases, and some initially-active
    // cells will have dissipated (showing clear air through the Voronoi tint).
    let dir = output_dir();
    let time_3h = 3.0 * 3_600.0;
    for &crop in CROPS {
        let img = render_cell_type_map(crop, time_3h, cell_state::state_at_time);
        write_png(&img, &dir, &format!("cell_type_evolved_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_cell_phase_artifacts() {
    // Phase field at t=0: each cell's current position in its life-cycle.
    // Dark = start of growth, bright = peak, dark-red = dissipating, black
    // = dormant. Reveals the temporal offset distribution across cells.
    let dir = output_dir();
    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let (x, z) = crop.world_coord(px, py);
                let lookup = voronoi::cell_at(CANONICAL_SEED, x, z);
                let init = cell_state::initial_state(CANONICAL_SEED, lookup.cell_id, 0.0);
                let color = if matches!(init.cell_type, cell_state::CellType::ClearAir) {
                    // W1-clear cells: always dark gray (no lifecycle).
                    [40, 40, 45]
                } else {
                    let phase = cell_state::cell_phase(CANONICAL_SEED, lookup.cell_id, 0.0);
                    let activity = cell_state::lifecycle_activity(phase);
                    ramp_rgb(activity)
                };
                img.put_pixel(px, py, Rgb(color));
            }
        }
        write_png(&img, &dir, &format!("cell_phase_{}.png", crop.name));
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

/// Alpha-blend `src` over `dst` at `alpha` ∈ `[0, 1]`.
fn blend(dst: [u8; 3], src: [u8; 3], alpha: f32) -> [u8; 3] {
    let a = alpha.clamp(0.0, 1.0);
    [
        (f32::from(dst[0]) * (1.0 - a) + f32::from(src[0]) * a) as u8,
        (f32::from(dst[1]) * (1.0 - a) + f32::from(src[1]) * a) as u8,
        (f32::from(dst[2]) * (1.0 - a) + f32::from(src[2]) * a) as u8,
    ]
}

/// Desaturate an RGB color toward mid-gray by `factor` ∈ `[0, 1]`. 0 = no
/// change; 1 = fully gray.
fn desaturate(rgb: [u8; 3], factor: f32) -> [u8; 3] {
    let f = factor.clamp(0.0, 1.0);
    let gray =
        (f32::from(rgb[0]) * 0.30 + f32::from(rgb[1]) * 0.59 + f32::from(rgb[2]) * 0.11) as u8;
    [
        (f32::from(rgb[0]) * (1.0 - f) + f32::from(gray) * f) as u8,
        (f32::from(rgb[1]) * (1.0 - f) + f32::from(gray) * f) as u8,
        (f32::from(rgb[2]) * (1.0 - f) + f32::from(gray) * f) as u8,
    ]
}

/// Draw a Bresenham line, painting a `thickness × thickness` square at each
/// step for visibility on complex backgrounds.
fn draw_line_thick(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    (x0, y0): (i32, i32),
    (x1, y1): (i32, i32),
    color: [u8; 3],
    thickness: i32,
) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    let (w, h) = (RESOLUTION as i32, RESOLUTION as i32);
    let half = thickness / 2;
    loop {
        for dx2 in -half..=half {
            for dy2 in -half..=half {
                let (px, py) = (x + dx2, y + dy2);
                if (0..w).contains(&px) && (0..h).contains(&py) {
                    img.put_pixel(px as u32, py as u32, Rgb(color));
                }
            }
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

/// Draw an arrow from `origin` to `tip`, with a short arrowhead back from the
/// tip at ±30° from the shaft direction.
fn draw_arrow(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    origin: (i32, i32),
    tip: (i32, i32),
    color: [u8; 3],
) {
    draw_line(img, origin, tip, color);
    let dx = (tip.0 - origin.0) as f32;
    let dy = (tip.1 - origin.1) as f32;
    let len = libm::sqrtf(dx * dx + dy * dy);
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (len * 0.35).min(6.0);
    // ±30° rotation of the reversed unit vector.
    let c30 = 0.866;
    let s30 = 0.5;
    let (rx1, ry1) = (-ux * c30 - (-uy) * s30, -uy * c30 + (-ux) * s30);
    let (rx2, ry2) = (-ux * c30 + (-uy) * s30, -uy * c30 - (-ux) * s30);
    let head_end_1 = (tip.0 + (rx1 * head) as i32, tip.1 + (ry1 * head) as i32);
    let head_end_2 = (tip.0 + (rx2 * head) as i32, tip.1 + (ry2 * head) as i32);
    draw_line(img, tip, head_end_1, color);
    draw_line(img, tip, head_end_2, color);
}

/// Dewpoint depression (temperature minus dewpoint) at sea level, Kelvin.
///
/// Uses a Magnus-formula approximation: `Td = (b · α) / (a - α)` with
/// `α = ln(RH) + a·T_C / (b + T_C)`, `a = 17.625`, `b = 243.04`. Returns
/// `T - Td` — small values (< 3 K) indicate saturation risk / fog; large
/// values (> 15 K) indicate arid air.
fn dewpoint_spread_k(t_k: f32, rh: f32) -> f32 {
    let t_c = t_k - 273.15;
    let rh_clamped = rh.clamp(0.01, 1.0);
    let a = 17.625_f32;
    let b = 243.04_f32;
    let alpha = libm::logf(rh_clamped) + a * t_c / (b + t_c);
    let td_c = (b * alpha) / (a - alpha);
    t_c - td_c
}

/// Lifting condensation level (m AGL) via Espy's rule: `LCL ≈ 125 × spread`.
///
/// Physical meaning: altitude at which a rising parcel reaches saturation.
/// 0 m = fog at ground; 1–2 km = typical cumulus base; 3 km+ = dry convective
/// air (high-desert or elevated boundary layer).
fn lcl_altitude_m(t_k: f32, rh: f32) -> f32 {
    125.0 * dewpoint_spread_k(t_k, rh)
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_dewpoint_spread_artifacts() {
    // 0 K (saturated, fog) → 20 K (arid). Blue = moist, red = dry.
    let dir = output_dir();
    let value_range = (0.0, 20.0);
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            let t = temperature::sea_level_temperature(CANONICAL_SEED, x, z, 12.0 * 3_600.0);
            let rh = humidity::sea_level_humidity(CANONICAL_SEED, x, z, 0.0);
            dewpoint_spread_k(t, rh)
        });
        write_png(&img, &dir, &format!("dewpoint_spread_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_lcl_altitude_artifacts() {
    // 0 m (ground-level condensation = fog) → 4 km (arid convective air).
    // Blue = low cloud base (stratus country), red = high base (desert-like).
    let dir = output_dir();
    let value_range = (0.0, 4_000.0);
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| {
            let t = temperature::sea_level_temperature(CANONICAL_SEED, x, z, 12.0 * 3_600.0);
            let rh = humidity::sea_level_humidity(CANONICAL_SEED, x, z, 0.0);
            lcl_altitude_m(t, rh)
        });
        write_png(&img, &dir, &format!("lcl_altitude_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_moisture_overlay_artifacts() {
    // Moisture-focused composite: dewpoint-spread ramp as background,
    // cell-type tint over cloud-bearing cells, frontal segments, wind arrows.
    let dir = output_dir();
    let spread_lo = 0.0;
    let spread_hi = 20.0;
    let spread_span = spread_hi - spread_lo;

    let arrow_stride = RESOLUTION / ARROW_COUNT;
    let arrow_len_px = (arrow_stride as f32) * 0.45;

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);

        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let (x, z) = crop.world_coord(px, py);
                let t = temperature::sea_level_temperature(CANONICAL_SEED, x, z, 12.0 * 3_600.0);
                let rh = humidity::sea_level_humidity(CANONICAL_SEED, x, z, 0.0);
                let spread = dewpoint_spread_k(t, rh);
                // Invert the ramp so moist (low spread) is blue, dry is red.
                let spread_rgb = desaturate(
                    ramp_rgb(((spread - spread_lo) / spread_span).clamp(0.0, 1.0)),
                    0.45,
                );

                let lookup = voronoi::cell_at(CANONICAL_SEED, x, z);
                let state = cell_state::initial_state(CANONICAL_SEED, lookup.cell_id, 0.0);
                let mut out = if matches!(state.cell_type, cell_state::CellType::ClearAir) {
                    spread_rgb
                } else {
                    blend(spread_rgb, cell_type_color(state.cell_type), 0.40)
                };

                let edge_pixels = (300.0 / crop.extent_m) * RESOLUTION as f32;
                if lookup.distance_to_edge < 300.0 && edge_pixels > 1.0 {
                    let t_edge = lookup.distance_to_edge / 300.0;
                    for c in &mut out {
                        *c = (f32::from(*c) * (0.5 + 0.5 * t_edge)) as u8;
                    }
                }

                img.put_pixel(px, py, Rgb(out));
            }
        }

        let segs = frontal::frontal_segments(
            CANONICAL_SEED,
            crop.center_x_m,
            crop.center_z_m,
            crop.extent_m / 2.0,
            crop.extent_m / 25.0,
            0.0,
        );
        for s in &segs {
            if let (Some(p0), Some(p1)) = (
                world_to_pixel(crop, s.start.x, s.start.y),
                world_to_pixel(crop, s.end.x, s.end.y),
            ) {
                draw_line_thick(&mut img, p0, p1, [255, 255, 255], 5);
                draw_line_thick(&mut img, p0, p1, [0, 0, 0], 2);
            }
        }

        let max_mag = 25.0;
        for ay in 0..ARROW_COUNT {
            for ax in 0..ARROW_COUNT {
                let px = ax * arrow_stride + arrow_stride / 2;
                let py = ay * arrow_stride + arrow_stride / 2;
                let (x, z) = crop.world_coord(px, py);
                let w = wind::wind_at(CANONICAL_SEED, x, 2_000.0, z, 0.0);
                let mag = libm::sqrtf(w.x * w.x + w.z * w.z);
                let scale = (mag / max_mag).clamp(0.1, 1.0) * arrow_len_px;
                let tip_px = (
                    px as i32 + (w.x * scale / mag.max(0.01)) as i32,
                    py as i32 - (w.z * scale / mag.max(0.01)) as i32,
                );
                draw_arrow(&mut img, (px as i32, py as i32), tip_px, [255, 255, 255]);
                draw_arrow(&mut img, (px as i32, py as i32), tip_px, [30, 30, 30]);
            }
        }

        write_png(&img, &dir, &format!("moisture_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_synoptic_overlay_artifacts() {
    // Weather-map-style composite: pressure field (desaturated) as background,
    // cell-type tint in active cells, Voronoi edges, frontal segments, and
    // sparse wind arrows. Continental crop (2000 km) appended as a
    // seamlessness preview for the pinned Option A toroidal world.
    let dir = output_dir();
    let p_lo = pressure::BASELINE_HPA - pressure::SYNOPTIC_AMPLITUDE_HPA;
    let p_hi = pressure::BASELINE_HPA + pressure::SYNOPTIC_AMPLITUDE_HPA;
    let p_span = p_hi - p_lo;

    // Arrow grid density — 24 × 24 arrows per image (~43 px spacing).
    let arrow_stride = RESOLUTION / ARROW_COUNT;
    let arrow_len_px = (arrow_stride as f32) * 0.45;

    let synoptic_crops: &[Crop] = &[Crop::FULL, Crop::QUADRANT, Crop::PLAYER, Crop::CONTINENTAL];
    for &crop in synoptic_crops {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);

        // Background: desaturated pressure + cell-type tint + Voronoi edge.
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let (x, z) = crop.world_coord(px, py);
                let p = pressure::sea_level_pressure(CANONICAL_SEED, x, z, 0.0);
                let pressure_rgb =
                    desaturate(ramp_rgb(((p - p_lo) / p_span).clamp(0.0, 1.0)), 0.55);

                let lookup = voronoi::cell_at(CANONICAL_SEED, x, z);
                let state = cell_state::initial_state(CANONICAL_SEED, lookup.cell_id, 0.0);
                let mut out = if matches!(state.cell_type, cell_state::CellType::ClearAir) {
                    pressure_rgb
                } else {
                    // Blend cell-type color over pressure background at 40% alpha.
                    blend(pressure_rgb, cell_type_color(state.cell_type), 0.40)
                };

                // Subtle Voronoi-edge darkening (only where cells are large
                // enough on-screen to distinguish).
                let edge_pixels = (300.0 / crop.extent_m) * RESOLUTION as f32;
                if lookup.distance_to_edge < 300.0 && edge_pixels > 1.0 {
                    let t = lookup.distance_to_edge / 300.0;
                    for c in &mut out {
                        *c = (f32::from(*c) * (0.5 + 0.5 * t)) as u8;
                    }
                }

                img.put_pixel(px, py, Rgb(out));
            }
        }

        // Frontal segments: white halo + black core for readability against
        // any background.
        let segs = frontal::frontal_segments(
            CANONICAL_SEED,
            crop.center_x_m,
            crop.center_z_m,
            crop.extent_m / 2.0,
            crop.extent_m / 25.0,
            0.0,
        );
        for s in &segs {
            if let (Some(p0), Some(p1)) = (
                world_to_pixel(crop, s.start.x, s.start.y),
                world_to_pixel(crop, s.end.x, s.end.y),
            ) {
                draw_line_thick(&mut img, p0, p1, [255, 255, 255], 5);
                draw_line_thick(&mut img, p0, p1, [0, 0, 0], 2);
            }
        }

        // Wind arrows on a sparse regular grid. Arrow length scales with
        // magnitude so calm regions show short stubs, jet-stream cores show
        // long arrows.
        let max_mag = 25.0;
        for ay in 0..ARROW_COUNT {
            for ax in 0..ARROW_COUNT {
                let px = ax * arrow_stride + arrow_stride / 2;
                let py = ay * arrow_stride + arrow_stride / 2;
                let (x, z) = crop.world_coord(px, py);
                // Sample wind aloft (2 km) so surface friction isn't the
                // dominant visual signal.
                let w = wind::wind_at(CANONICAL_SEED, x, 2_000.0, z, 0.0);
                let mag = libm::sqrtf(w.x * w.x + w.z * w.z);
                let scale = (mag / max_mag).clamp(0.1, 1.0) * arrow_len_px;
                // World axes: +x east, +z north. Image +y is south.
                let tip_px = (
                    px as i32 + (w.x * scale / mag.max(0.01)) as i32,
                    py as i32 - (w.z * scale / mag.max(0.01)) as i32,
                );
                // Halo + core.
                draw_arrow(&mut img, (px as i32, py as i32), tip_px, [255, 255, 255]);
                draw_arrow(&mut img, (px as i32, py as i32), tip_px, [30, 30, 30]);
            }
        }

        write_png(&img, &dir, &format!("synoptic_{}.png", crop.name));
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
