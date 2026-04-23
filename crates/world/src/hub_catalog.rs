//! Hub catalog bake (Phase 3.6.1e).
//!
//! Pre-baked, read-only catalog of ~55K hubs committed at world-load
//! time. Each [`HubEntry`] carries a stable [`HubId`] + approximate
//! world coords + tier + operational tags + region id + optional
//! display name. Mission systems (Phase 4.5+) reference hubs by id;
//! 2D hub-map (4.8) renders approximate coords; Voronoi late-binding
//! (3.6.2) resolves approximate coords to actual cell positions when
//! streaming arrives.
//!
//! # Architecture
//!
//! Five tiers (see [`HubTier`]) seeded separately with per-tier
//! density, min-spacing, and constraint rules. Placement uses
//! rejection-sampled dart-throwing — deterministic per master seed,
//! blue-noise-like distribution because spacing-rejection kills
//! clusters.
//!
//! Placement constraints per tier (checked at dart time):
//! - Biome: some tiers (Major, Regional) prefer coastal / low-altitude
//!   biomes; Remote / Isolated accept any
//! - Altitude: Major/Regional need flat low-altitude; Alpine tarn +
//!   barn-find tier-5 accept high altitudes
//! - Water adjacency: water-required tiers reject placements without
//!   a lake / river / sea within N km
//! - Min spacing: tier-n hub rejects placement too close to another
//!   tier-n hub (preserves the "regional spread" feel)
//!
//! # Region Voronoi
//!
//! Regions are their own Voronoi layer at ~300 km average spacing
//! (~280 seeds). Power-diagram weights sampled from a biome-coherence
//! field produce **3.7-4.8× linear size variation**: seeds in biome
//! cores get large weights → big regions (California/Texas-scale,
//! ~400 K km²); seeds at biome transitions get small weights → tight
//! regions (Austria-scale, ~30 K km²). Multi-biome regions emerge
//! naturally when a transition-zone seed's cell spans biome edges.
//!
//! `region_id_at(x, z)` does power-diagram nearest-seed lookup: O(N)
//! over 280 seeds, fast enough even at per-pixel query rate.
//!
//! # Operational tags
//!
//! Each hub carries [`HubOperationalTags`] bits describing what gear
//! the destination demands (high-altitude ops, snow landing, water-
//! only, STOL required, etc.). Populated at placement from biome +
//! altitude + water adjacency + tier. Mission matching (4.5) filters
//! on these so grayed-mission UX shows "requires STOL gear" hints.
//!
//! # Determinism
//!
//! Pure function of `(master_seed, terrain, hydrology)`. Bit-identical
//! per seed. Folded into the world-bake hash contract.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::too_long_first_doc_paragraph)]

use flowplan_core::{wrap_coord, WORLD_EXTENT_M};

use crate::biome::{biome_at, biome_weights_at, BiomeClass};
use crate::hydrology::{HydrologyGraph, WaterClass};
use crate::noise::fbm_2d;
use crate::terrain::Terrain;

// ═══════════════════════════════════════════════════════════════════════════
// Public types
// ═══════════════════════════════════════════════════════════════════════════

/// Stable cross-session hub identifier. Assigned in seed order at bake
/// time; preserved across re-bakes of the same master seed.
pub type HubId = u32;

/// Stable cross-session region identifier. 0-based index into the
/// region-seed catalog; `region_id_at(x, z)` resolves world coords
/// to a region via power-diagram lookup.
pub type RegionId = u32;

/// Hub tier taxonomy per [`FEATURES.md § Hubs & Landing Areas`](../../../FEATURES.md#hubs--landing-areas).
/// Drives layout, runway count, mission density, aircraft availability,
/// placement constraints.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HubTier {
    /// Major — 2-3 runways, full services, control tower, seaplane
    /// base. 1-2 per region. Authored tier-1 layouts land in 3.6.3.
    Major,
    /// Regional — quonset hangars, one runway (maybe two), no tower.
    /// 3-5 per region.
    Regional,
    /// Local — dirt strip, tie-down, fuel pump. 10-20 per region.
    Local,
    /// Remote — fishing camp / ranger cabin / clearing. 30+ per region.
    Remote,
    /// Isolated — uncharted, barn-find territory. ~500 per region;
    /// discovery-gated (no map entry until landed).
    Isolated,
}

impl HubTier {
    /// All tiers in placement order (heaviest-first so higher-tier
    /// spacing rejections apply to lower-tier candidates).
    pub const ALL: [Self; 5] = [
        Self::Major,
        Self::Regional,
        Self::Local,
        Self::Remote,
        Self::Isolated,
    ];
}

/// Operational tags describing what gear + skills a hub destination
/// demands. Bitfield packed into `u32`. Set at placement time from
/// biome + altitude + water adjacency + tier.
///
/// Mission filtering (Phase 4.5) reads these to match aircraft loadout:
/// a mission to a `HIGH_ALTITUDE_OPS | SNOW_LANDING` hub greys out if
/// the player's fleet has no skis / turbocharged engines.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct HubOperationalTags(u32);

impl HubOperationalTags {
    /// Destination sits above 1500 m — density-altitude planning matters.
    pub const HIGH_ALTITUDE_OPS: Self = Self(1 << 0);
    /// Alpine or subarctic biome + cold season → snow/ski landing likely.
    pub const SNOW_LANDING: Self = Self(1 << 1);
    /// No land surface adjacent — floats required.
    pub const WATER_ONLY: Self = Self(1 << 2);
    /// No water adjacent — wheels only.
    pub const LAND_ONLY: Self = Self(1 << 3);
    /// Short surface — tundra tires / STOL technique wanted.
    pub const STOL_REQUIRED: Self = Self(1 << 4);
    /// Coastal boreal / humid biomes → fog + low-ceiling common.
    pub const LOW_VIS_COMMON: Self = Self(1 << 5);
    /// Archipelago / island-chain destination → efficient powerplant
    /// + long-range tanks recommended.
    pub const LONG_RANGE_DESTINATION: Self = Self(1 << 6);
    /// Remote / isolated — fuel planning critical, no refuel available.
    pub const REMOTE_NO_REFUEL: Self = Self(1 << 7);

    /// Empty bitfield.
    pub const NONE: Self = Self(0);

    /// Raw bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// True if all bits in `other` are set in `self`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// True if any bit in `other` is set in `self`.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }
}

impl core::ops::BitOr for HubOperationalTags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for HubOperationalTags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Runway-naming style convention per tier.
///
/// - `Formal`: full "Runway 27L" numbering per FEATURES § Runway
///   authoring rules. Used by Major / Regional tiers with paved or
///   prepared runways.
/// - `Informal`: casual references like "Access Road 27" or "The
///   north strip" — the magnetic heading still drives the number when
///   a number is used, but the surface isn't called "Runway X". Used
///   by Local / Remote tiers with dirt roads, clearings, lakeshores.
/// - `NoRunway`: no runway reference at all — the tier-5 isolated
///   "pristine cove" case. Surface described purely by terrain.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum RunwayNamingStyle {
    /// Paved / prepared runway: "Runway 09/27", "Runway 18L".
    Formal,
    /// Dirt road / clearing / lakeshore: "Access Road 27", "north strip".
    Informal,
    /// No runway reference; terrain-described surface.
    NoRunway,
}

/// Hub catalog entry.
#[derive(Clone, Debug)]
pub struct HubEntry {
    /// Stable cross-session id.
    pub id: HubId,
    /// Approximate world coords (meter-precision not guaranteed pre-
    /// Voronoi-resolution; meter-precision after 3.6.2 resolve step).
    pub approx_world_pos: (f32, f32),
    /// Hub tier.
    pub tier: HubTier,
    /// Biome class at hub position (sampled at bake time).
    pub parent_biome: BiomeClass,
    /// Region id containing this hub.
    pub region_id: RegionId,
    /// Operational tags.
    pub operational_tags: HubOperationalTags,
    /// Runway-naming style convention.
    pub runway_naming_style: RunwayNamingStyle,
    /// Procedurally-generated display name (None for tier-5 isolated;
    /// authored-override path arrives later).
    pub display_name: Option<String>,
}

/// Region Voronoi seed entry.
#[derive(Copy, Clone, Debug)]
pub struct RegionSeed {
    /// Stable region id.
    pub id: RegionId,
    /// Seed position in world coords.
    pub position: (f32, f32),
    /// Power-diagram weight (m²). Higher weight = larger cell.
    pub weight_m2: f32,
    /// Biome class at seed position (informs region naming + size).
    pub primary_biome: BiomeClass,
}

/// Hub catalog baked from terrain + hydrology at world-load time.
/// Read-only after construction; safe to share across threads.
#[derive(Clone, Debug)]
pub struct HubCatalog {
    /// Master seed used for the bake.
    seed: u64,
    /// Full hub list, ordered by id.
    hubs: Vec<HubEntry>,
    /// Region seed catalog for power-diagram lookup.
    region_seeds: Vec<RegionSeed>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Bake-time constants
// ═══════════════════════════════════════════════════════════════════════════

/// Target region seed count for a 5000 km world (~300 km average
/// spacing). Produces Austria-to-California size range via power-
/// diagram weights.
pub const REGION_SEED_COUNT: usize = 280;

/// Power-diagram weight amplitude (m²) for region Voronoi.
///
/// Raised to (300 km)² = 9 × 10¹⁰ to produce visible size variation
/// (~2-3× linear, ~4-9× area) between biome-core regions and
/// transition-zone regions. Combined with region-area-weighted
/// dart acceptance (see [`REGION_DENSITY_SCALE_MIN`] /
/// [`REGION_DENSITY_SCALE_MAX`]), this gives the real-geography
/// feel of "big regions have more hubs per km² — one / two very
/// large, several medium, many small — like real countries/states".
///
/// Initial calibration at 1.6e11 = (400 km)² produced a 460× hub-
/// count ratio (too extreme). 4e10 = (200 km)² was too flat. 9e10
/// is the middle ground.
pub const REGION_WEIGHT_AMPLITUDE_M2: f32 = 6.5e10; // ≈ (255 km)²

/// Minimum region-size density scale applied to per-dart acceptance
/// probability. A small region (low weight) gets this fraction of
/// uniform density. Prevents tiny transition-zone regions from being
/// completely empty while still letting them have fewer hubs.
///
/// Bumped from 0.4 → 0.6 after the initial (300 km)² amplitude +
/// 0.4 min-scale produced a 485× hub-count ratio (one region had a
/// single hub). 0.6 floor ensures small regions still have ~20 +
/// hubs worth of Isolated density + a handful of larger tiers.
pub const REGION_DENSITY_SCALE_MIN: f32 = 0.6;

/// Maximum region-size density scale. Biggest region (high weight)
/// gets this multiplier on uniform density. Models real-world urban
/// concentration — big regions have a "capital" effect where hub
/// density rises faster than area.
pub const REGION_DENSITY_SCALE_MAX: f32 = 2.0;

/// Region weight-field fBm wavelength (m). 1500 km — coarser than
/// per-region spacing so weight varies gradually across seeds.
pub const REGION_WEIGHT_FIELD_WAVELENGTH_M: f32 = flowplan_core::WORLD_EXTENT_M / 3.0;

/// Per-tier target hub count (totals across all regions).
pub const TIER_COUNT_MAJOR: usize = 150;
/// Per-tier target hub count (regional).
pub const TIER_COUNT_REGIONAL: usize = 400;
/// Per-tier target hub count (local).
pub const TIER_COUNT_LOCAL: usize = 1_500;
/// Per-tier target hub count (remote).
pub const TIER_COUNT_REMOTE: usize = 3_500;
/// Per-tier target hub count (isolated).
pub const TIER_COUNT_ISOLATED: usize = 50_000;

/// Per-tier minimum inter-hub spacing (m). Within the same tier, hubs
/// reject placement closer than this.
const MIN_SPACING_MAJOR_M: f32 = 200_000.0;
const MIN_SPACING_REGIONAL_M: f32 = 80_000.0;
const MIN_SPACING_LOCAL_M: f32 = 20_000.0;
const MIN_SPACING_REMOTE_M: f32 = 6_000.0;
const MIN_SPACING_ISOLATED_M: f32 = 1_000.0;

/// Retry budget multiplier per tier — `target_count × this` dart
/// attempts before giving up on filling to target.
const PLACEMENT_RETRY_MULTIPLIER: usize = 4;

/// Water-adjacency search radius (m). A hub with water-adjacency
/// requirement rejects placement without water-class cell within this
/// distance.
const WATER_ADJACENCY_RADIUS_M: f32 = 15_000.0;

/// Octave-offset base for region weight field (reserved block
/// `[720..740)` in the project's octave-offset convention, disjoint
/// from magnetic-variation `[700..720)` + biome primitives `[500..700)`).
const REGION_WEIGHT_OCTAVE_OFFSET: u32 = 720;

/// Octave-offset base for hub placement jitter PRNG mixing. Reserved
/// block `[740..760)` in the project's octave-offset convention —
/// unused at present but kept to document the reservation for future
/// placement-heuristic work that wants its own Perlin field.
#[allow(dead_code)]
const HUB_PLACEMENT_OCTAVE_OFFSET: u32 = 740;

const HUB_CATALOG_FIELD_TAG: u64 = u64::from_le_bytes(*b"hubcatal");
const REGION_SEEDS_FIELD_TAG: u64 = u64::from_le_bytes(*b"regionse");

/// Splitmix64-inspired seed mixer (matches pattern in weather / magnetic_variation).
const fn mix_seed(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_add(b).wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Deterministic u32 hash (splitmix-style). Used for per-candidate
/// jitter in hub-placement dart throwing.
const fn hash_u32(a: u64, b: u64) -> u32 {
    let mixed = mix_seed(a, b);
    (mixed & 0xFFFF_FFFF) as u32
}

/// Pseudo-random f32 in [0, 1) from a u32 hash input.
fn hash_to_unit(a: u64, b: u64) -> f32 {
    let h = hash_u32(a, b);
    (h as f32) / (u32::MAX as f32 + 1.0)
}

// ═══════════════════════════════════════════════════════════════════════════
// Region Voronoi (power-diagram nearest-seed)
// ═══════════════════════════════════════════════════════════════════════════

/// Bake the region Voronoi seed catalog. Uses a regular grid with
/// per-cell jitter for blue-noise-like distribution. Per-seed weight
/// sampled from Perlin-fBm at `REGION_WEIGHT_FIELD_WAVELENGTH_M`.
fn bake_region_seeds(master_seed: u64) -> Vec<RegionSeed> {
    let seed = mix_seed(master_seed, REGION_SEEDS_FIELD_TAG);
    // Grid cell count ~ sqrt(REGION_SEED_COUNT). 17×17 = 289 cells,
    // closest to 280 target. Each cell contributes one jittered seed.
    let grid_n: usize = 17;
    let cell_size_m = WORLD_EXTENT_M / grid_n as f32;
    let mut seeds: Vec<RegionSeed> = Vec::with_capacity(grid_n * grid_n);
    let jitter_fraction = 0.4_f32; // keep seed inside central 80% of cell

    for iz in 0..grid_n {
        for ix in 0..grid_n {
            let cell_center_x = (ix as f32 + 0.5) * cell_size_m;
            let cell_center_z = (iz as f32 + 0.5) * cell_size_m;
            let jx_hash = hash_to_unit(seed, (iz as u64) * 1031 + (ix as u64) * 17);
            let jz_hash = hash_to_unit(seed, (iz as u64) * 2053 + (ix as u64) * 23);
            let jitter_x = (jx_hash - 0.5) * 2.0 * jitter_fraction * cell_size_m;
            let jitter_z = (jz_hash - 0.5) * 2.0 * jitter_fraction * cell_size_m;
            let x = wrap_coord(cell_center_x + jitter_x);
            let z = wrap_coord(cell_center_z + jitter_z);

            // Power-diagram weight: Perlin-fBm at ~1500 km × amplitude.
            // Bias by biome coherence — seeds in biome cores (strongest
            // single-biome weight) get boosted, seeds at biome transitions
            // get suppressed. This produces larger homogeneous regions +
            // tighter transition-zone regions.
            let weight_noise = fbm_2d(
                seed,
                REGION_WEIGHT_OCTAVE_OFFSET,
                x,
                z,
                REGION_WEIGHT_FIELD_WAVELENGTH_M,
                3,
            );
            // biome_weights_at returns 4-component BiomeWeights; max
            // component = "strongest biome" (0.25 minimum, 1.0 maximum).
            // Shift to [0, 1] after subtracting 0.25 baseline.
            let weights = biome_weights_at(master_seed, x, z);
            let max_bw = weights
                .high_desert
                .max(weights.rocky_alpine)
                .max(weights.coastal_boreal)
                .max(weights.mediterranean);
            let biome_coherence = ((max_bw - 0.25) / 0.75).clamp(0.0, 1.0);
            // Combine: Perlin noise [-1, 1] provides half; biome
            // coherence [0, 1] provides the other half. Maps to weight
            // scalar [0, 1], then to amplitude.
            let weight_scalar =
                (0.5 * weight_noise + 0.5 * biome_coherence * 2.0 - 0.5).clamp(0.0, 1.0);
            let weight_m2 = REGION_WEIGHT_AMPLITUDE_M2 * weight_scalar;

            let primary_biome = biome_at(master_seed, x, z);
            seeds.push(RegionSeed {
                id: seeds.len() as u32,
                position: (x, z),
                weight_m2,
                primary_biome,
            });
        }
    }
    seeds
}

/// Power-diagram nearest-seed lookup with toroidal wrap. Returns the
/// region id whose `d² = ||p - s||² - weight` is lowest for query
/// position `(x, z)`.
fn region_id_at_impl(seeds: &[RegionSeed], x: f32, z: f32) -> RegionId {
    let mut best_id: RegionId = 0;
    let mut best_score = f32::INFINITY;
    let qx = wrap_coord(x);
    let qz = wrap_coord(z);
    let half_w = WORLD_EXTENT_M * 0.5;
    for seed in seeds {
        let (sx, sz) = seed.position;
        let mut dx = (qx - sx).abs();
        let mut dz = (qz - sz).abs();
        // Toroidal-closest path on each axis.
        if dx > half_w {
            dx = WORLD_EXTENT_M - dx;
        }
        if dz > half_w {
            dz = WORLD_EXTENT_M - dz;
        }
        let d2 = dx * dx + dz * dz - seed.weight_m2;
        if d2 < best_score {
            best_score = d2;
            best_id = seed.id;
        }
    }
    best_id
}

// ═══════════════════════════════════════════════════════════════════════════
// Hub placement
// ═══════════════════════════════════════════════════════════════════════════

/// Per-tier placement parameters.
#[derive(Copy, Clone, Debug)]
struct TierParams {
    target_count: usize,
    min_spacing_m: f32,
    requires_water: bool,            // reject placement without water nearby
    max_sink_altitude_m: f32,        // reject if sink elevation above this
    min_sink_altitude_m: f32,        // reject if sink elevation below this
    allow_isolated_high_peaks: bool, // tier-5 allows alpine tarns
}

impl TierParams {
    const fn for_tier(tier: HubTier) -> Self {
        match tier {
            HubTier::Major => Self {
                target_count: TIER_COUNT_MAJOR,
                min_spacing_m: MIN_SPACING_MAJOR_M,
                requires_water: true,
                max_sink_altitude_m: 500.0, // paved runway needs flat low ground
                min_sink_altitude_m: 0.0,
                allow_isolated_high_peaks: false,
            },
            HubTier::Regional => Self {
                target_count: TIER_COUNT_REGIONAL,
                min_spacing_m: MIN_SPACING_REGIONAL_M,
                requires_water: false,
                max_sink_altitude_m: 1_500.0,
                min_sink_altitude_m: 0.0,
                allow_isolated_high_peaks: false,
            },
            HubTier::Local => Self {
                target_count: TIER_COUNT_LOCAL,
                min_spacing_m: MIN_SPACING_LOCAL_M,
                requires_water: false,
                max_sink_altitude_m: 2_500.0,
                min_sink_altitude_m: 0.0,
                allow_isolated_high_peaks: false,
            },
            HubTier::Remote => Self {
                target_count: TIER_COUNT_REMOTE,
                min_spacing_m: MIN_SPACING_REMOTE_M,
                requires_water: false,
                max_sink_altitude_m: 3_500.0,
                min_sink_altitude_m: 0.0,
                allow_isolated_high_peaks: true,
            },
            HubTier::Isolated => Self {
                target_count: TIER_COUNT_ISOLATED,
                min_spacing_m: MIN_SPACING_ISOLATED_M,
                requires_water: false,
                max_sink_altitude_m: 4_500.0, // world max — can be alpine tarn
                min_sink_altitude_m: -100.0,  // world min — can be weird low spots
                allow_isolated_high_peaks: true,
            },
        }
    }
}

/// Runway-naming style convention by tier.
const fn runway_style_for_tier(tier: HubTier) -> RunwayNamingStyle {
    match tier {
        HubTier::Major | HubTier::Regional => RunwayNamingStyle::Formal,
        HubTier::Local | HubTier::Remote => RunwayNamingStyle::Informal,
        HubTier::Isolated => RunwayNamingStyle::NoRunway,
    }
}

/// Spatial-hash acceleration structure for min-spacing rejection.
///
/// Hubs bucket into grid cells of side `min_spacing_m`; candidate
/// rejection checks the 3×3 neighborhood (9 cells) for pre-existing
/// hubs within the spacing radius. Makes placement O(N) instead of
/// O(N²).
struct SpatialHash {
    cell_size_m: f32,
    grid_n: usize,
    buckets: Vec<Vec<u32>>, // flat; indexed by iz*grid_n + ix; Vec<u32> stores hub ids
}

impl SpatialHash {
    fn new(cell_size_m: f32) -> Self {
        let grid_n = ((WORLD_EXTENT_M / cell_size_m).ceil() as usize).max(1);
        let buckets = vec![Vec::new(); grid_n * grid_n];
        Self {
            cell_size_m,
            grid_n,
            buckets,
        }
    }

    fn cell_index(&self, x: f32, z: f32) -> (usize, usize) {
        let xw = wrap_coord(x);
        let zw = wrap_coord(z);
        let ix = ((xw / self.cell_size_m) as usize).min(self.grid_n - 1);
        let iz = ((zw / self.cell_size_m) as usize).min(self.grid_n - 1);
        (ix, iz)
    }

    fn insert(&mut self, hub_id: u32, x: f32, z: f32) {
        let (ix, iz) = self.cell_index(x, z);
        self.buckets[iz * self.grid_n + ix].push(hub_id);
    }

    /// Return true if any inserted hub is within `min_distance_m` of
    /// (x, z), using toroidal-closest path.
    fn any_within(&self, positions: &[(f32, f32)], x: f32, z: f32, min_distance_m: f32) -> bool {
        let (ix, iz) = self.cell_index(x, z);
        let min_d2 = min_distance_m * min_distance_m;
        let half_w = WORLD_EXTENT_M * 0.5;
        // Check the 3x3 neighborhood (± search radius at most one
        // cell — grid cell size IS min spacing, so neighbors cover).
        // Search radius if min_spacing_m < cell_size, widen search.
        let radius_cells = ((min_distance_m / self.cell_size_m).ceil() as i64 + 1).max(1);
        for dz in -radius_cells..=radius_cells {
            for dx in -radius_cells..=radius_cells {
                let nx = ((ix as i64 + dx).rem_euclid(self.grid_n as i64)) as usize;
                let nz = ((iz as i64 + dz).rem_euclid(self.grid_n as i64)) as usize;
                for &hid in &self.buckets[nz * self.grid_n + nx] {
                    let (px, pz) = positions[hid as usize];
                    let mut dx_m = (x - px).abs();
                    let mut dz_m = (z - pz).abs();
                    if dx_m > half_w {
                        dx_m = WORLD_EXTENT_M - dx_m;
                    }
                    if dz_m > half_w {
                        dz_m = WORLD_EXTENT_M - dz_m;
                    }
                    if dx_m * dx_m + dz_m * dz_m < min_d2 {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Compute operational tags for a hub at `(x, z)` given its tier,
/// biome, altitude, and water classification.
fn compute_operational_tags(
    tier: HubTier,
    biome: BiomeClass,
    altitude_m: f32,
    has_water_adjacency: bool,
) -> HubOperationalTags {
    let mut tags = HubOperationalTags::NONE;

    if altitude_m > 1_500.0 {
        tags |= HubOperationalTags::HIGH_ALTITUDE_OPS;
    }

    // Snow landing: alpine biome at any altitude + other biomes at
    // high altitude. Seasonal in practice; here we just tag "could
    // be snow".
    if matches!(biome, BiomeClass::RockyAlpine) || altitude_m > 2_000.0 {
        tags |= HubOperationalTags::SNOW_LANDING;
    }

    if has_water_adjacency && altitude_m <= 10.0 {
        // Near sea level with water adjacency — some tier-5 isolated
        // hubs might only have water access (pristine cove), no land
        // surface. Heuristic: very low altitude + water + isolated/
        // remote = water-only likely.
        if matches!(tier, HubTier::Isolated | HubTier::Remote) {
            tags |= HubOperationalTags::WATER_ONLY;
        }
    } else if !has_water_adjacency {
        tags |= HubOperationalTags::LAND_ONLY;
    }

    // STOL required: remote + isolated tiers have short surfaces.
    if matches!(tier, HubTier::Remote | HubTier::Isolated) {
        tags |= HubOperationalTags::STOL_REQUIRED;
    }

    // Low-vis common: humid biomes.
    if matches!(biome, BiomeClass::CoastalBoreal) {
        tags |= HubOperationalTags::LOW_VIS_COMMON;
    }

    // Long-range destination: archipelago biome implies inter-island hops.
    if matches!(biome, BiomeClass::MediterraneanArchipelago) {
        tags |= HubOperationalTags::LONG_RANGE_DESTINATION;
    }

    // Remote no refuel: tiers 4-5.
    if matches!(tier, HubTier::Remote | HubTier::Isolated) {
        tags |= HubOperationalTags::REMOTE_NO_REFUEL;
    }

    tags
}

/// Dart-throwing placement for one tier. Returns Vec of (position,
/// altitude_m, biome, has_water_adjacency) for accepted candidates.
///
/// **Region-weighted acceptance**: dart acceptance probability scales
/// with the target region's power-diagram weight, modeling real-
/// geography urban concentration. Big regions (high weight, biome-
/// core cells) get up to [`REGION_DENSITY_SCALE_MAX`] × uniform
/// density; small regions (low weight, transition-zone cells) drop
/// to [`REGION_DENSITY_SCALE_MIN`] × uniform. One big region ends
/// up with "one/two large hubs + several medium + many small" pattern
/// like a real country/state.
#[allow(clippy::type_complexity)]
fn place_tier(
    tier: HubTier,
    master_seed: u64,
    terrain: &Terrain,
    hydrology: &HydrologyGraph,
    region_seeds: &[RegionSeed],
) -> Vec<(f32, f32, f32, BiomeClass, bool)> {
    let params = TierParams::for_tier(tier);
    let seed = mix_seed(master_seed, HUB_CATALOG_FIELD_TAG + tier as u64);

    let mut accepted: Vec<(f32, f32, f32, BiomeClass, bool)> =
        Vec::with_capacity(params.target_count);
    let mut positions: Vec<(f32, f32)> = Vec::with_capacity(params.target_count);
    let mut hash = SpatialHash::new(params.min_spacing_m);

    // Retries bumped to compensate for region-density rejection —
    // small regions reject ~(1 - MIN/MAX) of darts.
    let attempts = params.target_count * PLACEMENT_RETRY_MULTIPLIER * 2;
    let mut attempt: u64 = 0;
    while accepted.len() < params.target_count && attempt < attempts as u64 {
        let r_x = hash_to_unit(seed, attempt * 1_000_003);
        let r_z = hash_to_unit(seed, attempt * 1_000_033 + 17);
        let r_density = hash_to_unit(seed, attempt * 1_000_037 + 29);
        attempt += 1;
        let x = r_x * WORLD_EXTENT_M;
        let z = r_z * WORLD_EXTENT_M;

        // Region-density acceptance. accept_prob = density_scale /
        // MAX, so biggest regions accept ~100% of darts, smallest
        // accept MIN/MAX ≈ 18%.
        let region_id = region_id_at_impl(region_seeds, x, z);
        let region_weight = region_seeds[region_id as usize].weight_m2;
        let normalized = (region_weight / REGION_WEIGHT_AMPLITUDE_M2).clamp(0.0, 1.0);
        let density_scale = REGION_DENSITY_SCALE_MIN
            + (REGION_DENSITY_SCALE_MAX - REGION_DENSITY_SCALE_MIN) * normalized;
        let accept_prob = density_scale / REGION_DENSITY_SCALE_MAX;
        if r_density > accept_prob {
            continue;
        }

        // Altitude check — via mipmap-aware terrain sampling so we
        // get the coarse-scale elevation, not a random fine-scale spike.
        let altitude_m = terrain.height_at_scale(x, z, 2_500.0);
        if altitude_m < params.min_sink_altitude_m || altitude_m > params.max_sink_altitude_m {
            continue;
        }
        // Explicit reject: don't place ANY tier on sea-level cells
        // (pixel below 0) unless it's an "isolated water-only" case.
        if altitude_m < 0.0 && !params.allow_isolated_high_peaks {
            continue;
        }

        // Water adjacency check: does the hydrology cell at (x, z)
        // contain water, OR any cell within WATER_ADJACENCY_RADIUS_M?
        let has_water = has_water_within(hydrology, x, z, WATER_ADJACENCY_RADIUS_M);
        if params.requires_water && !has_water {
            continue;
        }

        // Min-spacing check against already-placed same-tier hubs.
        if hash.any_within(&positions, x, z, params.min_spacing_m) {
            continue;
        }

        let biome = biome_at(master_seed, x, z);
        accepted.push((x, z, altitude_m, biome, has_water));
        let idx = positions.len() as u32;
        positions.push((x, z));
        hash.insert(idx, x, z);
    }
    accepted
}

/// True if `(x, z)` or any cell within `radius_m` is classified as a
/// water surface by the hydrology graph.
fn has_water_within(hydrology: &HydrologyGraph, x: f32, z: f32, radius_m: f32) -> bool {
    // Sample at the query point + 8-compass offsets at `radius_m`.
    let offsets: [(f32, f32); 9] = [
        (0.0, 0.0),
        (radius_m, 0.0),
        (-radius_m, 0.0),
        (0.0, radius_m),
        (0.0, -radius_m),
        (radius_m, radius_m),
        (-radius_m, radius_m),
        (radius_m, -radius_m),
        (-radius_m, -radius_m),
    ];
    for (dx, dz) in offsets {
        if let Some(surface) = hydrology.water_surface_at(x + dx, z + dz) {
            if !matches!(surface.class, WaterClass::DryChannel) {
                return true;
            }
        }
    }
    false
}

// ═══════════════════════════════════════════════════════════════════════════
// Bake + query API
// ═══════════════════════════════════════════════════════════════════════════

impl HubCatalog {
    /// Bake the hub catalog for the given seed + terrain + hydrology.
    ///
    /// Cost: ~1 s on a modern CPU (~55 K dart-throws with spatial-hash
    /// acceleration). Memory: ~11 MB (55 K hubs × ~200 bytes incl.
    /// procedural display name).
    #[must_use]
    pub fn bake(master_seed: u64, terrain: &Terrain, hydrology: &HydrologyGraph) -> Self {
        let region_seeds = bake_region_seeds(master_seed);
        let mut hubs: Vec<HubEntry> = Vec::new();

        for tier in HubTier::ALL {
            let placed = place_tier(tier, master_seed, terrain, hydrology, &region_seeds);
            for (x, z, altitude_m, biome, has_water) in placed {
                let id = hubs.len() as u32;
                let region_id = region_id_at_impl(&region_seeds, x, z);
                let operational_tags = compute_operational_tags(tier, biome, altitude_m, has_water);
                let runway_naming_style = runway_style_for_tier(tier);
                let display_name = crate::hub_names::generate_name(master_seed, id, biome, tier);
                hubs.push(HubEntry {
                    id,
                    approx_world_pos: (x, z),
                    tier,
                    parent_biome: biome,
                    region_id,
                    operational_tags,
                    runway_naming_style,
                    display_name,
                });
            }
        }

        Self {
            seed: master_seed,
            hubs,
            region_seeds,
        }
    }

    /// Master seed used to bake this catalog.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// All hubs.
    #[must_use]
    pub fn hubs(&self) -> &[HubEntry] {
        &self.hubs
    }

    /// Total hub count (all tiers).
    #[must_use]
    pub fn hub_count(&self) -> usize {
        self.hubs.len()
    }

    /// Look up a hub by id.
    #[must_use]
    pub fn hub(&self, id: HubId) -> Option<&HubEntry> {
        self.hubs.get(id as usize)
    }

    /// Count hubs at a specific tier.
    #[must_use]
    pub fn hub_count_by_tier(&self, tier: HubTier) -> usize {
        self.hubs.iter().filter(|h| h.tier == tier).count()
    }

    /// All region seeds.
    #[must_use]
    pub fn region_seeds(&self) -> &[RegionSeed] {
        &self.region_seeds
    }

    /// Number of region seeds.
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.region_seeds.len()
    }

    /// Region id at world coordinate `(x, z)` via power-diagram lookup.
    #[must_use]
    pub fn region_id_at(&self, x: f32, z: f32) -> RegionId {
        region_id_at_impl(&self.region_seeds, x, z)
    }

    /// Return hubs within `radius_m` of `(center_x, center_z)`.
    /// Linear scan — O(N). Fine for typical query sizes; spatial-hash
    /// acceleration can land in a later phase if call sites demand.
    #[must_use]
    #[allow(clippy::cast_possible_wrap)]
    pub fn hubs_in_radius(&self, center_x: f32, center_z: f32, radius_m: f32) -> Vec<&HubEntry> {
        let r2 = radius_m * radius_m;
        let half_w = WORLD_EXTENT_M * 0.5;
        let cx = wrap_coord(center_x);
        let cz = wrap_coord(center_z);
        self.hubs
            .iter()
            .filter(|h| {
                let (hx, hz) = h.approx_world_pos;
                let mut dx = (cx - hx).abs();
                let mut dz = (cz - hz).abs();
                if dx > half_w {
                    dx = WORLD_EXTENT_M - dx;
                }
                if dz > half_w {
                    dz = WORLD_EXTENT_M - dz;
                }
                dx * dx + dz * dz <= r2
            })
            .collect()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::cast_precision_loss, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn tier_ordering_placement_heaviest_first() {
        // Placement iterates HubTier::ALL which must place Major first
        // so lower-tier rejections respect major-tier spacing.
        assert_eq!(HubTier::ALL[0], HubTier::Major);
        assert_eq!(HubTier::ALL[4], HubTier::Isolated);
    }

    #[test]
    fn operational_tags_bitfield_arithmetic() {
        let mut t = HubOperationalTags::NONE;
        t |= HubOperationalTags::HIGH_ALTITUDE_OPS;
        t |= HubOperationalTags::SNOW_LANDING;
        assert!(t.contains(HubOperationalTags::HIGH_ALTITUDE_OPS));
        assert!(t.intersects(HubOperationalTags::SNOW_LANDING));
        assert!(!t.contains(HubOperationalTags::WATER_ONLY));
    }

    #[test]
    fn runway_style_matches_tier_convention() {
        assert_eq!(
            runway_style_for_tier(HubTier::Major),
            RunwayNamingStyle::Formal
        );
        assert_eq!(
            runway_style_for_tier(HubTier::Regional),
            RunwayNamingStyle::Formal
        );
        assert_eq!(
            runway_style_for_tier(HubTier::Local),
            RunwayNamingStyle::Informal
        );
        assert_eq!(
            runway_style_for_tier(HubTier::Remote),
            RunwayNamingStyle::Informal
        );
        assert_eq!(
            runway_style_for_tier(HubTier::Isolated),
            RunwayNamingStyle::NoRunway
        );
    }

    // The following integration tests bake the full catalog against
    // the canonical seed + real terrain + real hydrology. Bake is
    // expensive (~5-10 s debug, ~2 s release).

    fn bake_canonical() -> HubCatalog {
        let terrain = Terrain::from_seed(0x0F10_1A11_u64);
        let hydrology = HydrologyGraph::bake(0x0F10_1A11_u64, &terrain);
        HubCatalog::bake(0x0F10_1A11_u64, &terrain, &hydrology)
    }

    #[test]
    fn bake_produces_hubs() {
        let c = bake_canonical();
        assert!(
            c.hub_count() > 10_000,
            "expected > 10K hubs in canonical bake; got {}",
            c.hub_count()
        );
    }

    #[test]
    fn bake_per_tier_counts_in_range() {
        let c = bake_canonical();
        // Major + Regional are the most constraint-limited so can fall
        // well below their targets if the world doesn't have enough
        // flat low-altitude coastal ground. Just assert > 0.
        assert!(c.hub_count_by_tier(HubTier::Major) > 0);
        assert!(c.hub_count_by_tier(HubTier::Regional) > 0);
        // Local / Remote / Isolated are less constrained and should
        // hit large fractions of target.
        assert!(c.hub_count_by_tier(HubTier::Local) > 500);
        assert!(c.hub_count_by_tier(HubTier::Remote) > 1_000);
        assert!(c.hub_count_by_tier(HubTier::Isolated) > 20_000);
    }

    #[test]
    fn bake_region_seeds_count() {
        let c = bake_canonical();
        assert_eq!(c.region_count(), 17 * 17);
    }

    #[test]
    fn bake_is_deterministic() {
        let terrain = Terrain::from_seed(12_345);
        let hydrology = HydrologyGraph::bake(12_345, &terrain);
        let a = HubCatalog::bake(12_345, &terrain, &hydrology);
        let b = HubCatalog::bake(12_345, &terrain, &hydrology);
        assert_eq!(a.hub_count(), b.hub_count());
        for (ha, hb) in a.hubs().iter().zip(b.hubs()).take(1000) {
            assert_eq!(ha.id, hb.id);
            assert_eq!(ha.tier, hb.tier);
            assert_eq!(
                ha.approx_world_pos.0.to_bits(),
                hb.approx_world_pos.0.to_bits()
            );
            assert_eq!(
                ha.approx_world_pos.1.to_bits(),
                hb.approx_world_pos.1.to_bits()
            );
            assert_eq!(ha.region_id, hb.region_id);
        }
    }

    #[test]
    fn bake_hubs_within_altitude_envelope() {
        let c = bake_canonical();
        for h in c.hubs().iter().take(5_000) {
            let params = TierParams::for_tier(h.tier);
            let altitude = Terrain::from_seed(c.seed()).height_at_scale(
                h.approx_world_pos.0,
                h.approx_world_pos.1,
                2_500.0,
            );
            assert!(
                altitude >= params.min_sink_altitude_m - 10.0
                    && altitude <= params.max_sink_altitude_m + 10.0,
                "hub {} (tier {:?}) at altitude {altitude} outside tier envelope [{}, {}]",
                h.id,
                h.tier,
                params.min_sink_altitude_m,
                params.max_sink_altitude_m
            );
        }
    }

    #[test]
    fn bake_major_hubs_water_adjacent() {
        let c = bake_canonical();
        let terrain = Terrain::from_seed(c.seed());
        let hydrology = HydrologyGraph::bake(c.seed(), &terrain);
        for h in c.hubs().iter().filter(|h| h.tier == HubTier::Major) {
            assert!(
                has_water_within(
                    &hydrology,
                    h.approx_world_pos.0,
                    h.approx_world_pos.1,
                    WATER_ADJACENCY_RADIUS_M,
                ),
                "major hub {} lacks water adjacency",
                h.id
            );
        }
    }

    #[test]
    fn region_id_at_deterministic() {
        let c = bake_canonical();
        let r1 = c.region_id_at(123_456.0, -789_012.0);
        let r2 = c.region_id_at(123_456.0, -789_012.0);
        assert_eq!(r1, r2);
    }

    #[test]
    fn region_id_wraps_toroidally() {
        let c = bake_canonical();
        let r1 = c.region_id_at(100_000.0, 100_000.0);
        let r2 = c.region_id_at(100_000.0 + WORLD_EXTENT_M, 100_000.0);
        assert_eq!(r1, r2);
    }

    #[test]
    fn hubs_in_radius_returns_reasonable_count() {
        let c = bake_canonical();
        // At 100 km radius around (0, 0), expect 5-500 hubs (depends
        // on where the origin lands).
        let nearby = c.hubs_in_radius(0.0, 0.0, 100_000.0);
        assert!(
            nearby.len() < 10_000,
            "hubs_in_radius returned {} — suspiciously high",
            nearby.len()
        );
    }

    #[test]
    fn bake_stats_sanity() {
        // Print catalog stats for visual-bench tuning feedback.
        let c = bake_canonical();
        eprintln!(
            "hub catalog stats: seed={:x} total_hubs={} regions={}",
            c.seed(),
            c.hub_count(),
            c.region_count()
        );
        for tier in HubTier::ALL {
            let count = c.hub_count_by_tier(tier);
            let target = TierParams::for_tier(tier).target_count;
            eprintln!(
                "  {:?}: {count} / {target} ({:.1}%)",
                tier,
                100.0 * count as f32 / target as f32
            );
        }
        // Operational-tag distribution.
        let mut high_alt = 0_u32;
        let mut water_only = 0_u32;
        let mut land_only = 0_u32;
        let mut stol = 0_u32;
        let mut snow = 0_u32;
        for h in c.hubs() {
            if h.operational_tags
                .contains(HubOperationalTags::HIGH_ALTITUDE_OPS)
            {
                high_alt += 1;
            }
            if h.operational_tags.contains(HubOperationalTags::WATER_ONLY) {
                water_only += 1;
            }
            if h.operational_tags.contains(HubOperationalTags::LAND_ONLY) {
                land_only += 1;
            }
            if h.operational_tags
                .contains(HubOperationalTags::STOL_REQUIRED)
            {
                stol += 1;
            }
            if h.operational_tags
                .contains(HubOperationalTags::SNOW_LANDING)
            {
                snow += 1;
            }
        }
        eprintln!(
            "  tags: high_altitude={high_alt} water_only={water_only} \
             land_only={land_only} stol={stol} snow={snow}"
        );
        // Sample names.
        eprintln!("  sample names:");
        for tier in [
            HubTier::Major,
            HubTier::Regional,
            HubTier::Local,
            HubTier::Remote,
        ] {
            let samples: Vec<String> = c
                .hubs()
                .iter()
                .filter(|h| h.tier == tier)
                .take(3)
                .filter_map(|h| h.display_name.clone())
                .collect();
            eprintln!("    {tier:?}: {samples:?}");
        }
        // Region size variation — count hubs per region to infer
        // spread.
        let mut per_region: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for h in c.hubs() {
            *per_region.entry(h.region_id).or_insert(0) += 1;
        }
        let mut counts: Vec<u32> = per_region.values().copied().collect();
        counts.sort_unstable();
        let smallest = counts.first().copied().unwrap_or(0);
        let largest = counts.last().copied().unwrap_or(0);
        let ratio = if smallest > 0 {
            largest as f32 / smallest as f32
        } else {
            f32::INFINITY
        };
        eprintln!(
            "  region hub-count: min={smallest} max={largest} ratio={ratio:.1}x \
             (target: 4-20x for Austria/California-scale variation)"
        );
    }

    /// Toroidal distance between two world points (returns metres).
    fn toroidal_distance_m(a: (f32, f32), b: (f32, f32)) -> f32 {
        let half_w = WORLD_EXTENT_M * 0.5;
        let mut dx = (a.0 - b.0).abs();
        let mut dz = (a.1 - b.1).abs();
        if dx > half_w {
            dx = WORLD_EXTENT_M - dx;
        }
        if dz > half_w {
            dz = WORLD_EXTENT_M - dz;
        }
        dx.hypot(dz)
    }

    #[test]
    fn nearest_neighbor_distance_per_tier_in_band() {
        // For each tier: compute nearest same-tier neighbor distance
        // for every hub. Two invariants:
        //
        // 1. `nn_min >= min_spacing_m` — algorithm guarantee from
        //    dart-throwing rejection. Same check as `min_spacing_*`
        //    but via a different path.
        // 2. `nn_mean` sits in a plausible band derived from target
        //    density: expected NN for uniformly-distributed N points
        //    in area A is ~0.5 × sqrt(A/N) (Clark-Evans). Blue-noise
        //    rejection pushes it a bit higher. Upper bound at 3× the
        //    uniform-random expected value catches dropouts /
        //    under-population.
        let c = bake_canonical();
        let world_area_m2 = WORLD_EXTENT_M * WORLD_EXTENT_M;
        for tier in HubTier::ALL {
            let params = TierParams::for_tier(tier);
            let hubs: Vec<&HubEntry> = c.hubs().iter().filter(|h| h.tier == tier).collect();
            if hubs.len() < 2 {
                continue;
            }
            let mut nn_sum = 0.0_f64;
            let mut nn_min = f32::INFINITY;
            let mut nn_max = 0.0_f32;
            for (i, a) in hubs.iter().enumerate() {
                let mut nearest = f32::INFINITY;
                for (j, b) in hubs.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    let d = toroidal_distance_m(a.approx_world_pos, b.approx_world_pos);
                    if d < nearest {
                        nearest = d;
                    }
                }
                nn_sum += f64::from(nearest);
                if nearest < nn_min {
                    nn_min = nearest;
                }
                if nearest > nn_max {
                    nn_max = nearest;
                }
            }
            let nn_mean = (nn_sum / hubs.len() as f64) as f32;
            // Invariant 1: algorithm guarantee.
            assert!(
                nn_min >= params.min_spacing_m - 1.0,
                "{:?} nn_min {} < min_spacing {}",
                tier,
                nn_min,
                params.min_spacing_m
            );
            // Invariant 2: density-derived upper bound. Expected NN
            // for uniform random ~ 0.5 × sqrt(area / count). Bound at
            // 4× that (tolerates dropouts up to 75% placement failure).
            let expected_nn = 0.5 * (world_area_m2 / hubs.len() as f32).sqrt();
            let plausible_upper = expected_nn * 4.0;
            assert!(
                nn_mean >= params.min_spacing_m,
                "{:?} nn_mean {} below min_spacing {}",
                tier,
                nn_mean,
                params.min_spacing_m
            );
            assert!(
                nn_mean <= plausible_upper,
                "{tier:?} nn_mean {nn_mean:.0}m > density-plausible upper \
                 {plausible_upper:.0}m (expected_nn={expected_nn:.0}m)"
            );
            eprintln!(
                "  {:?}: count={} nn_min={:.0}m nn_mean={:.0}m nn_max={:.0}m \
                 (min_spacing={:.0}m expected_nn={:.0}m)",
                tier,
                hubs.len(),
                nn_min,
                nn_mean,
                nn_max,
                params.min_spacing_m,
                expected_nn,
            );
        }
    }

    #[test]
    fn distance_to_nearest_major_hub_reasonable() {
        // For every Regional hub, distance to nearest Major hub.
        // Mission routing assumes majors are "within a few hundred km"
        // of any operational hub. If Regional → nearest Major > 1000 km
        // something has gone wrong.
        let c = bake_canonical();
        let majors: Vec<&HubEntry> = c
            .hubs()
            .iter()
            .filter(|h| h.tier == HubTier::Major)
            .collect();
        assert!(!majors.is_empty(), "no major hubs to measure against");

        let regionals: Vec<&HubEntry> = c
            .hubs()
            .iter()
            .filter(|h| h.tier == HubTier::Regional)
            .collect();
        let mut max_dist = 0.0_f32;
        let mut mean_dist_sum = 0.0_f64;
        for r in &regionals {
            let mut nearest = f32::INFINITY;
            for m in &majors {
                let d = toroidal_distance_m(r.approx_world_pos, m.approx_world_pos);
                if d < nearest {
                    nearest = d;
                }
            }
            mean_dist_sum += f64::from(nearest);
            if nearest > max_dist {
                max_dist = nearest;
            }
        }
        let mean_dist = (mean_dist_sum / regionals.len() as f64) as f32;
        eprintln!(
            "Regional → nearest Major: max {:.0} km, mean {:.0} km",
            max_dist / 1000.0,
            mean_dist / 1000.0
        );
        // Max regional-to-major distance: reasonable bound given world
        // is 5000 km and majors are ~200 km apart. A regional hub far
        // from any major can be up to ~half the major-spacing from one
        // + sqrt(region_diameter²), so ~500 km is plausible upper
        // bound. 1500 km would indicate a dead-zone.
        assert!(
            max_dist < 1_500_000.0,
            "regional-to-major max distance {:.0} km too far — placement dead zones?",
            max_dist / 1000.0
        );
    }

    #[test]
    fn region_diameter_range_matches_target() {
        // Measure per-region "diameter" as max inter-hub distance
        // among hubs assigned to that region. Assert diameters span
        // a realistic range.
        let c = bake_canonical();
        let mut per_region: std::collections::HashMap<u32, Vec<(f32, f32)>> =
            std::collections::HashMap::new();
        for h in c.hubs() {
            per_region
                .entry(h.region_id)
                .or_default()
                .push(h.approx_world_pos);
        }
        let mut diameters: Vec<f32> = Vec::new();
        for hubs in per_region.values() {
            if hubs.len() < 2 {
                continue;
            }
            let mut max_d = 0.0_f32;
            for (i, a) in hubs.iter().enumerate() {
                for b in hubs.iter().skip(i + 1) {
                    let d = toroidal_distance_m(*a, *b);
                    if d > max_d {
                        max_d = d;
                    }
                }
            }
            diameters.push(max_d);
        }
        diameters.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p10 = diameters[diameters.len() / 10];
        let p50 = diameters[diameters.len() / 2];
        let p90 = diameters[(diameters.len() * 9) / 10];
        eprintln!(
            "region diameter percentiles: p10={:.0} km, p50={:.0} km, p90={:.0} km",
            p10 / 1000.0,
            p50 / 1000.0,
            p90 / 1000.0,
        );
        // P50 should be roughly 300 km (our nominal region size). P10
        // and P90 bracket the variation. At 55 K hubs distributed
        // uniformly, diameter measurement is well-sampled per region,
        // so p90/p10 reflects actual region-size variation introduced
        // by power-diagram weights + grid jitter — ~1.3-3× typical.
        let linear_ratio = p90 / p10;
        assert!(
            (1.1..=10.0).contains(&linear_ratio),
            "region diameter p90/p10 ratio {linear_ratio:.2}× outside 1.1-10× plausible band"
        );
        // P50 plausibility: 150-700 km (brackets nominal 300 km with
        // margin in both directions).
        assert!(
            (150_000.0..=700_000.0).contains(&p50),
            "region diameter median {:.0} km outside 150-700 km plausible band",
            p50 / 1000.0
        );
    }

    #[test]
    fn min_spacing_respected_within_tier() {
        let c = bake_canonical();
        // Check min-spacing holds for Major tier — smallest count,
        // easiest to verify.
        let majors: Vec<&HubEntry> = c
            .hubs()
            .iter()
            .filter(|h| h.tier == HubTier::Major)
            .collect();
        let min_spacing = MIN_SPACING_MAJOR_M;
        let min_d2 = min_spacing * min_spacing;
        let half_w = WORLD_EXTENT_M * 0.5;
        for (i, a) in majors.iter().enumerate() {
            for b in majors.iter().skip(i + 1) {
                let (ax, az) = a.approx_world_pos;
                let (bx, bz) = b.approx_world_pos;
                let mut dx = (ax - bx).abs();
                let mut dz = (az - bz).abs();
                if dx > half_w {
                    dx = WORLD_EXTENT_M - dx;
                }
                if dz > half_w {
                    dz = WORLD_EXTENT_M - dz;
                }
                assert!(
                    dx * dx + dz * dz >= min_d2 - 1.0,
                    "major hubs {} and {} closer than min spacing",
                    a.id,
                    b.id
                );
            }
        }
    }
}
