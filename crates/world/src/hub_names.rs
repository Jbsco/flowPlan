//! Hub display name generation (Phase 3.6.1e).
//!
//! Procedural name generation with per-biome word lists + per-tier
//! templates. Deterministic per `(master_seed, hub_id)` — same world
//! produces the same names.
//!
//! # Design
//!
//! Each biome has its own vocabulary (flora/fauna, geographic features,
//! descriptors, surnames) curated to the regional aesthetic. Alaska
//! boreal uses "Moose / Eagle / Cedar / Bay"; High Desert uses
//! "Raven / Sage / Mesa / Canyon"; Mediterranean uses "Olive / Isle /
//! Cove". Tier controls the template — Major hubs get Airport /
//! Regional / Intl suffixes; Remote hubs get just the place name.
//!
//! Tier 5 (Isolated) returns `None` — these are uncharted until the
//! player discovers them; Phase 4.8 cartographer can support player-
//! given names.
//!
//! # Evolution path
//!
//! Shipping with Rust const arrays for simplicity + zero new dep.
//! Content is editable at the language level (modify arrays below).
//! When the authoring surface grows (hundreds of names), migrate to
//! a RON file with `include_str!` + `ron` crate parsing. No user-
//! visible change at that migration.

#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::literal_string_with_formatting_args)]

use crate::biome::BiomeClass;
use crate::hub_catalog::HubTier;

// ═══════════════════════════════════════════════════════════════════════════
// Per-biome word lists
// ═══════════════════════════════════════════════════════════════════════════

/// Per-biome vocabulary.
struct BiomeVocab {
    /// Flora / fauna nouns ("Moose", "Eagle", "Cedar").
    nouns: &'static [&'static str],
    /// Geographic features ("Bay", "Ridge", "Creek").
    geo: &'static [&'static str],
    /// Adjective-like descriptors ("Big", "Silver", "Lost").
    descriptors: &'static [&'static str],
    /// Surnames for "Mount X" / "Fort X" / "Port X" styles.
    surnames: &'static [&'static str],
}

impl BiomeVocab {
    /// Look up vocabulary by biome class.
    const fn for_biome(biome: BiomeClass) -> Self {
        match biome {
            BiomeClass::HighDesert => HIGH_DESERT_VOCAB,
            BiomeClass::RockyAlpine => ROCKY_ALPINE_VOCAB,
            BiomeClass::CoastalBoreal => COASTAL_BOREAL_VOCAB,
            BiomeClass::MediterraneanArchipelago => MEDITERRANEAN_VOCAB,
        }
    }
}

/// Colorado Plateau / Grand Mesa / red-rock flavor (see memory:
/// `biome_grand_mesa_ground_truth`).
const HIGH_DESERT_VOCAB: BiomeVocab = BiomeVocab {
    nouns: &[
        "Raven",
        "Sage",
        "Juniper",
        "Rattler",
        "Coyote",
        "Cactus",
        "Mule Deer",
        "Pronghorn",
        "Pinyon",
        "Cottonwood",
        "Sego",
        "Magpie",
    ],
    geo: &[
        "Canyon", "Mesa", "Arroyo", "Wash", "Butte", "Spring", "Hollow", "Flats", "Plateau",
        "Basin", "Gulch", "Bench",
    ],
    descriptors: &[
        "Red", "Lost", "Dry", "High", "Big", "Silent", "Hidden", "Wild", "Painted", "Shadow",
        "Rock",
    ],
    surnames: &[
        "Harrison",
        "Wyatt",
        "Sterling",
        "Ellsworth",
        "Cortez",
        "Monroe",
        "Travis",
        "Thornton",
        "Kane",
    ],
};

/// Colorado Rockies / Sawtooth / Cascades flavor.
const ROCKY_ALPINE_VOCAB: BiomeVocab = BiomeVocab {
    nouns: &[
        "Bear",
        "Elk",
        "Pine",
        "Aspen",
        "Bighorn",
        "Spruce",
        "Marmot",
        "Golden Eagle",
        "Grayling",
        "Ptarmigan",
        "Glacier Lily",
        "Cirque",
    ],
    geo: &[
        "Peak",
        "Tarn",
        "Col",
        "Ridge",
        "Gulch",
        "Pass",
        "Basin",
        "Saddle",
        "Meadow",
        "Headwaters",
        "Divide",
        "Spire",
    ],
    descriptors: &[
        "High",
        "Silver",
        "Iron",
        "Granite",
        "Divide",
        "Glacier",
        "Alpine",
        "Snowy",
        "Lone",
        "Timberline",
        "Upper",
    ],
    surnames: &[
        "Ellsworth",
        "Fremont",
        "Sawtooth",
        "Broncho",
        "Ridgeway",
        "Stewart",
        "Hayden",
        "McKinley",
        "Powell",
    ],
};

/// Alaska panhandle / BC coast / PNW fjord flavor.
const COASTAL_BOREAL_VOCAB: BiomeVocab = BiomeVocab {
    nouns: &[
        "Bear", "Eagle", "Salmon", "Cedar", "Willow", "Moose", "Halibut", "Puffin", "Heron",
        "Otter", "Hemlock", "Seal", "Whale", "Orca", "Raven", "Sitka",
    ],
    geo: &[
        "Bay", "Cove", "Sound", "Inlet", "Fjord", "Point", "Reach", "Passage", "Harbour", "Strait",
        "Narrows", "Rookery",
    ],
    descriptors: &[
        "Big",
        "Little",
        "North",
        "South",
        "Lost",
        "Wild",
        "Far",
        "Misty",
        "Silver",
        "Deep",
        "Quiet",
        "Sheltered",
    ],
    surnames: &[
        "McKenzie",
        "Thornton",
        "Pellegrini",
        "Reavis",
        "Hollister",
        "Vancouver",
        "Mackenzie",
        "Cook",
        "Bering",
        "Dall",
    ],
};

/// Aegean / Corsica / Mediterranean archipelago flavor.
const MEDITERRANEAN_VOCAB: BiomeVocab = BiomeVocab {
    nouns: &[
        "Olive",
        "Cypress",
        "Dolphin",
        "Fig",
        "Tern",
        "Kestrel",
        "Laurel",
        "Almond",
        "Sardine",
        "Tuna",
        "Myrtle",
        "Cormorant",
    ],
    geo: &[
        "Isle",
        "Cove",
        "Harbour",
        "Strand",
        "Reach",
        "Cape",
        "Channel",
        "Point",
        "Islet",
        "Bluff",
        "Shore",
        "Anchorage",
    ],
    descriptors: &[
        "Little", "Sun", "Calm", "Old", "Blue", "White", "Golden", "Bright", "Quiet", "Lower",
        "Upper",
    ],
    surnames: &[
        "Castellanos",
        "Lorca",
        "Kouros",
        "Verano",
        "Moreno",
        "Calpe",
        "Delfino",
        "Marchetti",
        "Vila",
        "Santos",
    ],
};

// ═══════════════════════════════════════════════════════════════════════════
// Templates per tier
// ═══════════════════════════════════════════════════════════════════════════

/// Template token placeholder — any of `{noun}`, `{geo}`, `{descriptor}`,
/// `{surname}`. Templates are compiled at sample time by replacement.
type Template = &'static str;

/// Tier-1 major hubs: full airport-style names.
const TEMPLATES_MAJOR: &[Template] = &[
    "{descriptor} {geo} Airport",
    "Port {surname}",
    "{noun} {geo} Intl",
    "{surname} Field",
    "{noun} {geo} Airport",
];

/// Tier-2 regional hubs: aerodrome / field style, often named after people.
const TEMPLATES_REGIONAL: &[Template] = &[
    "{descriptor} {geo}",
    "Mount {surname}",
    "{noun} {geo} Field",
    "Fort {surname}",
    "{descriptor} {noun} Field",
];

/// Tier-3 local hubs: dirt strips, quiet names.
const TEMPLATES_LOCAL: &[Template] = &[
    "{noun} {geo} Strip",
    "{descriptor} Creek",
    "{surname}'s Landing",
    "{descriptor} {geo}",
    "{noun} Strip",
];

/// Tier-4 remote hubs: place-names, the location IS the name.
const TEMPLATES_REMOTE: &[Template] = &[
    "{noun} {geo}",
    "{descriptor} {noun}",
    "{surname}'s Camp",
    "{descriptor} {geo}",
    "{noun} Camp",
];

/// Tier-5 isolated: none (catalog stays None).
const TEMPLATES_ISOLATED: &[Template] = &[];

/// Look up templates for a tier.
const fn templates_for_tier(tier: HubTier) -> &'static [Template] {
    match tier {
        HubTier::Major => TEMPLATES_MAJOR,
        HubTier::Regional => TEMPLATES_REGIONAL,
        HubTier::Local => TEMPLATES_LOCAL,
        HubTier::Remote => TEMPLATES_REMOTE,
        HubTier::Isolated => TEMPLATES_ISOLATED,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Deterministic sampling
// ═══════════════════════════════════════════════════════════════════════════

/// Splitmix64-inspired hash.
const fn hash64(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_add(b).wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Pick a deterministic index into a slice given a seed + salt.
fn pick_index(slice_len: usize, seed: u64, salt: u64) -> usize {
    if slice_len == 0 {
        return 0;
    }
    (hash64(seed, salt) as usize) % slice_len
}

const HUB_NAMES_TAG: u64 = u64::from_le_bytes(*b"hubnames");

/// Generate a display name for a hub, or `None` for tier-5 isolated
/// / when templates exist but something unusual happened.
///
/// Deterministic per `(master_seed, hub_id, biome, tier)`.
#[must_use]
pub fn generate_name(
    master_seed: u64,
    hub_id: u32,
    biome: BiomeClass,
    tier: HubTier,
) -> Option<String> {
    let templates = templates_for_tier(tier);
    if templates.is_empty() {
        return None;
    }
    let vocab = BiomeVocab::for_biome(biome);
    let seed = hash64(master_seed, HUB_NAMES_TAG);
    let hub_salt = u64::from(hub_id);
    let template = templates[pick_index(templates.len(), seed, hub_salt)];

    let noun = vocab.nouns[pick_index(vocab.nouns.len(), seed, hub_salt.wrapping_add(1))];
    let geo = vocab.geo[pick_index(vocab.geo.len(), seed, hub_salt.wrapping_add(2))];
    let descriptor =
        vocab.descriptors[pick_index(vocab.descriptors.len(), seed, hub_salt.wrapping_add(3))];
    let surname = vocab.surnames[pick_index(vocab.surnames.len(), seed, hub_salt.wrapping_add(4))];

    let name = template
        .replace("{noun}", noun)
        .replace("{geo}", geo)
        .replace("{descriptor}", descriptor)
        .replace("{surname}", surname);
    Some(name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn isolated_tier_returns_none() {
        for biome in [
            BiomeClass::HighDesert,
            BiomeClass::RockyAlpine,
            BiomeClass::CoastalBoreal,
            BiomeClass::MediterraneanArchipelago,
        ] {
            assert!(generate_name(42, 0, biome, HubTier::Isolated).is_none());
        }
    }

    #[test]
    fn non_isolated_tiers_always_return_name() {
        for tier in [
            HubTier::Major,
            HubTier::Regional,
            HubTier::Local,
            HubTier::Remote,
        ] {
            for biome in [
                BiomeClass::HighDesert,
                BiomeClass::RockyAlpine,
                BiomeClass::CoastalBoreal,
                BiomeClass::MediterraneanArchipelago,
            ] {
                let name = generate_name(42, 0, biome, tier);
                assert!(name.is_some());
                assert!(!name.unwrap().is_empty());
            }
        }
    }

    #[test]
    fn deterministic_per_seed_and_id() {
        let a = generate_name(42, 100, BiomeClass::CoastalBoreal, HubTier::Regional);
        let b = generate_name(42, 100, BiomeClass::CoastalBoreal, HubTier::Regional);
        assert_eq!(a, b);
    }

    #[test]
    fn different_hub_ids_usually_produce_different_names() {
        // Not a strict uniqueness guarantee (collisions possible at
        // small vocab × small template sets), but across 20 hub ids
        // we should see variety.
        use std::collections::HashSet;
        let mut seen: HashSet<String> = HashSet::new();
        for id in 0..20 {
            if let Some(n) = generate_name(42, id, BiomeClass::CoastalBoreal, HubTier::Regional) {
                seen.insert(n);
            }
        }
        assert!(
            seen.len() >= 5,
            "expected >= 5 distinct names across 20 hubs; got {}",
            seen.len()
        );
    }

    #[test]
    fn different_biomes_use_different_vocab() {
        // Generating a Regional-tier name for each biome should
        // produce biome-appropriate terms most of the time. We
        // sample-check that the produced strings land in the
        // expected vocab for each biome.
        let desert = generate_name(42, 1, BiomeClass::HighDesert, HubTier::Regional).unwrap();
        let coastal = generate_name(42, 1, BiomeClass::CoastalBoreal, HubTier::Regional).unwrap();
        // At least one word in each name should be a biome-specific
        // vocab entry. Weak check but catches accidental cross-biome
        // vocab usage.
        let desert_vocab_words: Vec<&str> = HIGH_DESERT_VOCAB
            .nouns
            .iter()
            .chain(HIGH_DESERT_VOCAB.geo.iter())
            .chain(HIGH_DESERT_VOCAB.descriptors.iter())
            .chain(HIGH_DESERT_VOCAB.surnames.iter())
            .copied()
            .collect();
        let coastal_vocab_words: Vec<&str> = COASTAL_BOREAL_VOCAB
            .nouns
            .iter()
            .chain(COASTAL_BOREAL_VOCAB.geo.iter())
            .chain(COASTAL_BOREAL_VOCAB.descriptors.iter())
            .chain(COASTAL_BOREAL_VOCAB.surnames.iter())
            .copied()
            .collect();
        assert!(
            desert_vocab_words.iter().any(|w| desert.contains(w)),
            "desert name '{desert}' has no high-desert vocab word"
        );
        assert!(
            coastal_vocab_words.iter().any(|w| coastal.contains(w)),
            "coastal name '{coastal}' has no coastal-boreal vocab word"
        );
    }
}
