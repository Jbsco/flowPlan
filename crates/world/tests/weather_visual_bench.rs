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
// Test harness: panicking on I/O error is the appropriate failure mode.
#![allow(clippy::expect_used)]

use std::path::Path;

use flowplan_world::weather::{humidity, pressure, temperature};
use image::{ImageBuffer, Rgb};

// Silence unused-crate-dependencies lint for transitive deps the integration
// test target sees but doesn't itself use.
use flowplan_core as _;
use glam as _;
use libm as _;

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
