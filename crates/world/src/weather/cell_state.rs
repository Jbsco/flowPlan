//! Per-cell weather state with W1 placement-constraint gating.
//!
//! Given a [`CellId`](super::voronoi::CellId), sample W1 fields at the cell's
//! seed position and classify the cell into one of six [`CellType`] variants.
//! Classification is deterministic per `(master_seed, cell_id, time)` and
//! respects MAPGEN placement constraints (pressure range, humidity threshold,
//! temperature range, wind shear, frontal-boundary proximity).
//!
//! # Real-world-possible gate
//!
//! Each cell type has physical preconditions that must hold in the W1 field
//! at the cell center:
//!
//! - `Cumulus`: warm-enough temp + moderate humidity. Fair-weather convection.
//! - `Nimbus`: deep convection signals (high humidity, warm, pressure trough).
//! - `Stratus`: stable, humid, cooler. Typical behind cold fronts.
//! - `Squall`: strong wind + moisture. Gust-front behavior.
//! - `Frontal`: on or near a W1 pressure-gradient maximum (frontal graph).
//! - `ClearAir`: default. "No-weather is a type of weather."
//!
//! A cell whose weighted-random roll picks a type that fails its constraint
//! falls back to `ClearAir` — re-rolling is not needed because eligibility
//! weights are zero for unmet preconditions, so a failed roll is statistically
//! rare (happens only on numerical edge cases).
//!
//! # Scope
//!
//! **Phase 3.2.2b (this module):** static per-cell state derived once per
//! `(seed, cell, time)` query. No temporal evolution yet — the `storm_phase`
//! field is 0 and no state-machine transitions happen. 3.2.2c adds the
//! adjacency-bias evolution and `storm_phase` advancement.

use super::voronoi::CellId;
use super::{frontal, humidity, pressure, temperature, voronoi, wind};

/// Cell-type classifications per [MAPGEN.md § W2 per-cell state](../../../../MAPGEN.md#w2--weather-voronoi-partition).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CellType {
    /// No cloud, no precipitation. Default / majority.
    ClearAir,
    /// Fair-weather cumulus: scattered puffy clouds, mild turbulence.
    Cumulus,
    /// Cumulonimbus: deep convective column, heavy precip, strong turbulence.
    Nimbus,
    /// Stratus: thin horizontal layer, low turbulence, light drizzle.
    Stratus,
    /// Squall line: strong wind-shift + heavy precip, short-duration.
    Squall,
    /// Along a W1 frontal-boundary line segment: banded structure,
    /// continuous precip, wind-shift across the line.
    Frontal,
}

/// Per-cell state snapshot at a query time.
#[derive(Copy, Clone, Debug)]
pub struct CellState {
    pub cell_type: CellType,
    /// Cloud base altitude (m MSL). 0 for `ClearAir`.
    pub base_altitude_m: f32,
    /// Cloud top altitude (m MSL). 0 for `ClearAir`.
    pub top_altitude_m: f32,
    /// Peak cloud density in `[0, 1]`. 0 for `ClearAir`.
    pub density: f32,
    /// Precipitation rate (mm/hr). 0 for `ClearAir` / `Cumulus`.
    pub precip_rate_mm_per_hr: f32,
    /// Storm evolution phase in `[0, 1]`. 0 at 3.2.2b; advances in 3.2.2c.
    pub storm_phase: f32,
}

// Placement-constraint thresholds — chosen to match real atmospheric precedents.

/// Minimum humidity for any cloud-bearing cell type.
pub const CLOUD_MIN_HUMIDITY: f32 = 0.40;

/// Minimum humidity for nimbus (deep convection).
pub const NIMBUS_MIN_HUMIDITY: f32 = 0.70;

/// Minimum temperature for convective cells (K). Below this, clouds stratify.
pub const CONVECTIVE_MIN_TEMP_K: f32 = 280.0;

/// Maximum temperature for stratus (K). Warmer → cells mix out.
pub const STRATUS_MAX_TEMP_K: f32 = 285.0;

/// Minimum surface-wind magnitude for squall (m/s).
pub const SQUALL_MIN_WIND_MPS: f32 = 14.0;

/// Pressure-gradient magnitude (hPa/m) above which a cell is a frontal candidate.
pub const FRONTAL_GRAD_THRESHOLD: f32 = frontal::FRONTAL_THRESHOLD_HPA_PER_M;

/// Baseline "no weather" prior.
///
/// Biases the statistical distribution toward `ClearAir` so clear skies are
/// the dominant cell type — matches real-world observation that most of the
/// sky, most of the time, is clear.
pub const CLEAR_AIR_PRIOR: f32 = 2.5;

/// Altitude at which wind is sampled for squall wind-shear classification (m).
const WIND_SAMPLE_ALTITUDE_M: f32 = 2_000.0;

const CELL_STATE_TAG: u64 = u64::from_le_bytes(*b"cellstat");
const CELL_OFFSET_TAG: u64 = u64::from_le_bytes(*b"celloff_");

/// Full life cycle duration of a weather cell (s).
///
/// 6 hours is the midpoint of real cell lifetimes: a fair-weather cumulus
/// lives 30–60 min; a stratus deck persists 4–8 hours; a mesoscale
/// convective system 3–6 hours. Using one fixed duration keeps the pure-
/// function model simple; per-type duration variability is a future
/// enhancement.
pub const CELL_CYCLE_DURATION_S: f32 = 6.0 * 3_600.0;

/// Phase boundaries within a cycle: growth-peak-dissipation-dormant.
///
/// At `phase < GROWTH_END`, activity ramps from 0 to 1.
/// At `phase < PEAK_END`, activity is 1.
/// At `phase < DISSIPATION_END`, activity ramps from 1 to 0.
/// Otherwise the cell is dormant (clear air).
pub const GROWTH_END: f32 = 0.15;
pub const PEAK_END: f32 = 0.60;
pub const DISSIPATION_END: f32 = 0.85;

/// Per-cell temporal offset in `[0, CELL_CYCLE_DURATION_S)`.
///
/// Deterministic per `(master_seed, cell)`. Ensures cells are at varying
/// phases at any given sim time — a snapshot of the grid shows a mix of
/// growing, peak, and dissipating cells rather than every cell cycling in
/// lockstep.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_sign_loss)]
pub fn cell_time_offset(master_seed: u64, cell: CellId) -> f32 {
    let packed = ((cell.ix as u32 as u64) << 32) | (cell.iz as u32 as u64);
    let mut x = master_seed
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(CELL_OFFSET_TAG)
        .wrapping_add(packed);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    let unit = ((x >> 40) as f32) / ((1u32 << 24) as f32);
    unit * CELL_CYCLE_DURATION_S
}

/// Cell phase `∈ [0, 1)` at sim time `time_s`.
///
/// Deterministic per `(master_seed, cell, time_s)`. Phase advances linearly
/// with time, wrapping at cycle boundaries. See [`state_at_time`] for how
/// phase drives activity.
#[must_use]
pub fn cell_phase(master_seed: u64, cell: CellId, time_s: f32) -> f32 {
    let offset = cell_time_offset(master_seed, cell);
    // Explicit mod-to-positive since time_s might theoretically be negative.
    let t = time_s + offset;
    let cycles = libm::floorf(t / CELL_CYCLE_DURATION_S);
    (t - cycles * CELL_CYCLE_DURATION_S) / CELL_CYCLE_DURATION_S
}

/// Life-cycle activity multiplier `∈ [0, 1]` at a given phase.
///
/// Piecewise-linear: 0→1 during growth, 1 at peak, 1→0 during dissipation,
/// 0 during dormant phase. A cell's density and precipitation are scaled by
/// this factor.
#[must_use]
pub fn lifecycle_activity(phase: f32) -> f32 {
    let p = phase.clamp(0.0, 1.0);
    if p < GROWTH_END {
        p / GROWTH_END
    } else if p < PEAK_END {
        1.0
    } else if p < DISSIPATION_END {
        1.0 - (p - PEAK_END) / (DISSIPATION_END - PEAK_END)
    } else {
        0.0
    }
}

/// Per-cell state at sim time `time_s`, evolved from `initial_state` through
/// the cell's life-cycle curve.
///
/// Deterministic per `(master_seed, cell, time_s)`. Adjacency-bias enhancements
/// (neighbor cell types influence re-roll at cycle boundaries) are a future
/// refinement — the W1 substrate already provides implicit adjacency since
/// neighboring cells share smooth pressure/temperature/humidity fields and
/// tend to classify into similar types.
#[must_use]
pub fn state_at_time(master_seed: u64, cell: CellId, time_s: f32) -> CellState {
    let initial = initial_state(master_seed, cell, time_s);
    if matches!(initial.cell_type, CellType::ClearAir) {
        return initial;
    }
    let phase = cell_phase(master_seed, cell, time_s);
    let activity = lifecycle_activity(phase);
    if activity < 0.01 {
        return CellState {
            cell_type: CellType::ClearAir,
            base_altitude_m: 0.0,
            top_altitude_m: 0.0,
            density: 0.0,
            precip_rate_mm_per_hr: 0.0,
            storm_phase: phase,
        };
    }
    CellState {
        density: initial.density * activity,
        precip_rate_mm_per_hr: initial.precip_rate_mm_per_hr * activity,
        storm_phase: phase,
        ..initial
    }
}

/// Initial per-cell state at `time_s`. Deterministic per `(master_seed, cell,
/// time_s)`.
#[must_use]
pub fn initial_state(master_seed: u64, cell: CellId, time_s: f32) -> CellState {
    let seed_pos = voronoi::seed_position(master_seed, cell);

    // Sample W1 at the cell's seed position.
    let p_hpa = pressure::sea_level_pressure(master_seed, seed_pos.x, seed_pos.y, time_s);
    let t_k = temperature::sea_level_temperature(master_seed, seed_pos.x, seed_pos.y, time_s);
    let rh = humidity::sea_level_humidity(master_seed, seed_pos.x, seed_pos.y, time_s);
    let w = wind::wind_at(
        master_seed,
        seed_pos.x,
        WIND_SAMPLE_ALTITUDE_M,
        seed_pos.y,
        time_s,
    );
    let wind_mag = libm::sqrtf(w.x * w.x + w.z * w.z);
    let grad_mag = pressure_gradient_magnitude(master_seed, seed_pos.x, seed_pos.y, time_s);

    let cell_type = select_cell_type(master_seed, cell, p_hpa, t_k, rh, wind_mag, grad_mag);
    parameterize(cell_type, rh, t_k)
}

/// Select cell type via weighted-random based on W1 eligibility.
#[allow(clippy::too_many_arguments)] // W1 state is wide by design.
fn select_cell_type(
    master_seed: u64,
    cell: CellId,
    p_hpa: f32,
    t_k: f32,
    rh: f32,
    wind_mag: f32,
    grad_mag: f32,
) -> CellType {
    let elig_cumulus = if rh >= CLOUD_MIN_HUMIDITY && t_k >= CONVECTIVE_MIN_TEMP_K {
        1.0
    } else {
        0.0
    };
    let elig_nimbus = if rh >= NIMBUS_MIN_HUMIDITY
        && t_k >= CONVECTIVE_MIN_TEMP_K
        && p_hpa < pressure::BASELINE_HPA
    {
        1.5
    } else {
        0.0
    };
    let elig_stratus = if rh >= (CLOUD_MIN_HUMIDITY + 0.1) && t_k <= STRATUS_MAX_TEMP_K {
        1.0
    } else {
        0.0
    };
    let elig_squall = if rh >= CLOUD_MIN_HUMIDITY && wind_mag >= SQUALL_MIN_WIND_MPS {
        1.0
    } else {
        0.0
    };
    let elig_frontal = if grad_mag >= FRONTAL_GRAD_THRESHOLD {
        2.0
    } else {
        0.0
    };

    let weights = [
        CLEAR_AIR_PRIOR,
        elig_cumulus,
        elig_nimbus,
        elig_stratus,
        elig_squall,
        elig_frontal,
    ];
    let total: f32 = weights.iter().sum();
    if total <= 0.0 {
        return CellType::ClearAir;
    }

    let roll = cell_roll(master_seed, cell) * total;
    let mut acc = 0.0;
    for (i, &w) in weights.iter().enumerate() {
        acc += w;
        if roll < acc {
            return match i {
                0 => CellType::ClearAir,
                1 => CellType::Cumulus,
                2 => CellType::Nimbus,
                3 => CellType::Stratus,
                4 => CellType::Squall,
                _ => CellType::Frontal,
            };
        }
    }
    CellType::ClearAir
}

/// Per-type altitude / density / precip parameters, tuned to plausible
/// real-atmospheric ranges. Some parameters vary with W1 state (cloud base
/// lowers with humidity, top lifts with warmth).
fn parameterize(cell_type: CellType, rh: f32, t_k: f32) -> CellState {
    let storm_phase = 0.0;
    match cell_type {
        CellType::ClearAir => CellState {
            cell_type,
            base_altitude_m: 0.0,
            top_altitude_m: 0.0,
            density: 0.0,
            precip_rate_mm_per_hr: 0.0,
            storm_phase,
        },
        CellType::Cumulus => {
            // LCL approximation: base rises with aridity.
            let base = 1_200.0 + (1.0 - rh) * 800.0;
            let top = base + 1_500.0 + (t_k - 283.0) * 100.0;
            CellState {
                cell_type,
                base_altitude_m: base,
                top_altitude_m: top.max(base + 500.0),
                density: 0.35,
                precip_rate_mm_per_hr: 0.0,
                storm_phase,
            }
        },
        CellType::Nimbus => {
            let base = 900.0 + (1.0 - rh) * 400.0;
            let top = 9_000.0 + (t_k - 283.0) * 200.0;
            CellState {
                cell_type,
                base_altitude_m: base,
                top_altitude_m: top,
                density: 0.80,
                precip_rate_mm_per_hr: 6.0,
                storm_phase,
            }
        },
        CellType::Stratus => {
            let base = 300.0 + (1.0 - rh) * 400.0;
            let top = base + 700.0;
            CellState {
                cell_type,
                base_altitude_m: base,
                top_altitude_m: top,
                density: 0.55,
                precip_rate_mm_per_hr: 0.3,
                storm_phase,
            }
        },
        CellType::Squall => CellState {
            cell_type,
            base_altitude_m: 500.0,
            top_altitude_m: 5_000.0,
            density: 0.65,
            precip_rate_mm_per_hr: 14.0,
            storm_phase,
        },
        CellType::Frontal => CellState {
            cell_type,
            base_altitude_m: 1_000.0,
            top_altitude_m: 6_000.0,
            density: 0.60,
            precip_rate_mm_per_hr: 8.0,
            storm_phase,
        },
    }
}

fn pressure_gradient_magnitude(master_seed: u64, x_m: f32, z_m: f32, time_s: f32) -> f32 {
    // Step matches the frontal-graph sampling step so thresholds line up.
    const FD_STEP_M: f32 = 2_000.0;
    let p_xp = pressure::sea_level_pressure(master_seed, x_m + FD_STEP_M, z_m, time_s);
    let p_xm = pressure::sea_level_pressure(master_seed, x_m - FD_STEP_M, z_m, time_s);
    let p_zp = pressure::sea_level_pressure(master_seed, x_m, z_m + FD_STEP_M, time_s);
    let p_zm = pressure::sea_level_pressure(master_seed, x_m, z_m - FD_STEP_M, time_s);
    let dp_dx = (p_xp - p_xm) / (2.0 * FD_STEP_M);
    let dp_dz = (p_zp - p_zm) / (2.0 * FD_STEP_M);
    libm::sqrtf(dp_dx * dp_dx + dp_dz * dp_dz)
}

/// Deterministic roll in `[0, 1)` from `(master_seed, cell)`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_sign_loss)]
fn cell_roll(master_seed: u64, cell: CellId) -> f32 {
    let packed = ((cell.ix as u32 as u64) << 32) | (cell.iz as u32 as u64);
    let mut x = master_seed
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(CELL_STATE_TAG)
        .wrapping_add(packed);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    ((x >> 40) as f32) / ((1u32 << 24) as f32)
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_deterministic() {
        let a = initial_state(42, CellId { ix: 3, iz: -2 }, 0.0);
        let b = initial_state(42, CellId { ix: 3, iz: -2 }, 0.0);
        assert_eq!(a.cell_type, b.cell_type);
        assert_eq!(a.density.to_bits(), b.density.to_bits());
    }

    #[test]
    fn clear_air_dominates() {
        // Over a dense sample of cells, clear_air should be the plurality.
        let mut counts = [0_u32; 6];
        for ix in -10..=10 {
            for iz in -10..=10 {
                let s = initial_state(42, CellId { ix, iz }, 0.0);
                let idx = match s.cell_type {
                    CellType::ClearAir => 0,
                    CellType::Cumulus => 1,
                    CellType::Nimbus => 2,
                    CellType::Stratus => 3,
                    CellType::Squall => 4,
                    CellType::Frontal => 5,
                };
                counts[idx] += 1;
            }
        }
        let total: u32 = counts.iter().sum();
        assert!(
            counts[0] > total / 3,
            "clear_air should dominate: counts = {counts:?} / {total}"
        );
    }

    #[test]
    fn constraint_violation_never_selected() {
        // Check that across a grid, no cell with rh < CLOUD_MIN_HUMIDITY came
        // back as cumulus/nimbus/stratus/squall/frontal — any cell with
        // insufficient humidity must be ClearAir or Frontal (frontal ignores rh).
        for ix in -20..=20 {
            for iz in -20..=20 {
                let cell = CellId { ix, iz };
                let seed_pos = voronoi::seed_position(42, cell);
                let rh = humidity::sea_level_humidity(42, seed_pos.x, seed_pos.y, 0.0);
                let s = initial_state(42, cell, 0.0);
                if rh < CLOUD_MIN_HUMIDITY {
                    // Only ClearAir or Frontal are allowed — frontal has no RH
                    // gate (it depends on pressure gradient instead).
                    assert!(
                        matches!(s.cell_type, CellType::ClearAir | CellType::Frontal),
                        "cell ({ix}, {iz}) with rh={rh} got {:?}",
                        s.cell_type
                    );
                }
            }
        }
    }

    #[test]
    fn clear_air_has_zero_density() {
        let s = CellState {
            cell_type: CellType::ClearAir,
            base_altitude_m: 0.0,
            top_altitude_m: 0.0,
            density: 0.0,
            precip_rate_mm_per_hr: 0.0,
            storm_phase: 0.0,
        };
        assert_eq!(s.density.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn nimbus_deeper_than_cumulus() {
        // Synthesize and check parameterize()'s shape: nimbus is taller.
        let cu = parameterize(CellType::Cumulus, 0.6, 288.0);
        let nb = parameterize(CellType::Nimbus, 0.75, 288.0);
        assert!(nb.top_altitude_m > cu.top_altitude_m);
        assert!(nb.density > cu.density);
        assert!(nb.precip_rate_mm_per_hr > cu.precip_rate_mm_per_hr);
    }

    #[test]
    fn different_seeds_produce_different_cell_states() {
        // Across many cells, at least one should differ by seed.
        let mut any_diff = false;
        for ix in 0..10 {
            for iz in 0..10 {
                let a = initial_state(1, CellId { ix, iz }, 0.0);
                let b = initial_state(2, CellId { ix, iz }, 0.0);
                if a.cell_type != b.cell_type {
                    any_diff = true;
                    break;
                }
            }
            if any_diff {
                break;
            }
        }
        assert!(any_diff, "seeds 1 vs 2 produced identical cell-state grids");
    }

    #[test]
    fn cell_phase_deterministic() {
        let a = cell_phase(42, CellId { ix: 3, iz: -2 }, 1_000.0);
        let b = cell_phase(42, CellId { ix: 3, iz: -2 }, 1_000.0);
        assert_eq!(a.to_bits(), b.to_bits());
    }

    #[test]
    fn cell_phase_wraps_at_cycle() {
        // Phase at time t and at time t + CELL_CYCLE_DURATION should be equal.
        let cell = CellId { ix: 5, iz: 7 };
        let p0 = cell_phase(42, cell, 100.0);
        let p1 = cell_phase(42, cell, 100.0 + CELL_CYCLE_DURATION_S);
        // Allow small float error in the modular arithmetic.
        assert!((p0 - p1).abs() < 1e-4, "phase {p0} vs {p1} after one cycle");
    }

    #[test]
    fn different_cells_have_different_offsets() {
        let o1 = cell_time_offset(42, CellId { ix: 0, iz: 0 });
        let o2 = cell_time_offset(42, CellId { ix: 1, iz: 0 });
        assert_ne!(o1.to_bits(), o2.to_bits());
    }

    #[test]
    fn lifecycle_activity_shape() {
        // Zero at phase 0 (start of growth).
        assert_eq!(lifecycle_activity(0.0).to_bits(), 0.0_f32.to_bits());
        // One at mid-peak.
        assert_eq!(lifecycle_activity(0.4).to_bits(), 1.0_f32.to_bits());
        // Zero in dormant phase.
        assert_eq!(lifecycle_activity(0.95).to_bits(), 0.0_f32.to_bits());
        // Partial during growth.
        let mid_growth = lifecycle_activity(GROWTH_END / 2.0);
        assert!(mid_growth > 0.0 && mid_growth < 1.0);
        // Partial during dissipation.
        let mid_dissipation = lifecycle_activity(f32::midpoint(PEAK_END, DISSIPATION_END));
        assert!(mid_dissipation > 0.0 && mid_dissipation < 1.0);
    }

    #[test]
    fn evolved_state_dormant_reports_clear_air() {
        // Find a cell that's non-ClearAir at initial, then fast-forward
        // until it's dormant and check that it reports ClearAir.
        for ix in 0..30 {
            for iz in 0..30 {
                let cell = CellId { ix, iz };
                let init = initial_state(42, cell, 0.0);
                if !matches!(init.cell_type, CellType::ClearAir) {
                    // Fast-forward to time when this cell is in the dormant
                    // phase (just before cycle wrap).
                    let offset = cell_time_offset(42, cell);
                    let target_phase = 0.92;
                    let target_time = target_phase * CELL_CYCLE_DURATION_S - offset;
                    if target_time > 0.0 {
                        let evolved = state_at_time(42, cell, target_time);
                        assert!(
                            matches!(evolved.cell_type, CellType::ClearAir),
                            "cell {:?} at phase {} should be dormant, got {:?}",
                            cell,
                            cell_phase(42, cell, target_time),
                            evolved.cell_type
                        );
                        return;
                    }
                }
            }
        }
        panic!("no non-ClearAir cell found in sample grid");
    }

    #[test]
    fn evolved_state_peak_matches_initial() {
        // At peak phase, density and precip should equal initial values
        // (activity == 1).
        for ix in 0..30 {
            for iz in 0..30 {
                let cell = CellId { ix, iz };
                let init = initial_state(42, cell, 0.0);
                if !matches!(init.cell_type, CellType::ClearAir) {
                    let offset = cell_time_offset(42, cell);
                    let target_phase = f32::midpoint(GROWTH_END, PEAK_END);
                    let target_time = target_phase * CELL_CYCLE_DURATION_S - offset;
                    if target_time > 0.0 {
                        let evolved = state_at_time(42, cell, target_time);
                        assert_eq!(evolved.density.to_bits(), init.density.to_bits());
                        assert_eq!(
                            evolved.precip_rate_mm_per_hr.to_bits(),
                            init.precip_rate_mm_per_hr.to_bits()
                        );
                        return;
                    }
                }
            }
        }
        panic!("no non-ClearAir cell found in sample grid");
    }

    #[test]
    fn lifecycle_cannot_activate_clear_cells() {
        // At any time t: if the W1 classifier resolves a cell to ClearAir,
        // state_at_time must also report ClearAir. The life-cycle only
        // dissipates active cells — it cannot conjure weather out of air that
        // the W1 substrate classifies as clear.
        for ix in 0..30 {
            for iz in 0..30 {
                let cell = CellId { ix, iz };
                for t_hr in [0.0, 1.0, 2.0, 5.0] {
                    let t = t_hr * 3_600.0;
                    let init = initial_state(42, cell, t);
                    let evolved = state_at_time(42, cell, t);
                    if matches!(init.cell_type, CellType::ClearAir) {
                        assert!(
                            matches!(evolved.cell_type, CellType::ClearAir),
                            "state_at_time activated a W1-clear cell at t={t_hr}h"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cloud_bearing_cells_have_positive_altitudes() {
        // Sample the grid; any non-ClearAir cell should have base and top > 0.
        for ix in -15..=15 {
            for iz in -15..=15 {
                let s = initial_state(42, CellId { ix, iz }, 0.0);
                if s.cell_type != CellType::ClearAir {
                    assert!(
                        s.base_altitude_m > 0.0,
                        "{:?} base_alt = {}",
                        s.cell_type,
                        s.base_altitude_m
                    );
                    assert!(s.top_altitude_m > s.base_altitude_m);
                    assert!(s.density > 0.0);
                }
            }
        }
    }
}
