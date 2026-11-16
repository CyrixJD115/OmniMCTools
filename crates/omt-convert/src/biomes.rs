//! Biome name normalization between the generated id table (keyed partly
//! by Bedrock/legacy names) and current Java names.
//!
//! The 26.3 Java biome list is the pinned `client.jar`'s
//! `data/minecraft/worldgen/biome/*.json` set (data, not code). Two jobs:
//! - **forward**: every current Java name must resolve to a Bedrock id —
//!   modern names missing from the table borrow their legacy twin's id;
//! - **reverse**: a Bedrock id must only ever produce a *valid* Java name —
//!   legacy/Bedrock-only variants (hills, `_mutated`, `hell`, …) fold into
//!   the modern biome that replaced them.

/// Every Java 26.3 worldgen biome (without `minecraft:`).
pub const JAVA_BIOMES: [&str; 67] = [
    "badlands", "bamboo_jungle", "basalt_deltas", "beach", "birch_forest", "cherry_grove", "cold_ocean",
    "crimson_forest", "dappled_forest", "dark_forest", "deep_cold_ocean", "deep_dark", "deep_frozen_ocean",
    "deep_lukewarm_ocean", "deep_ocean", "desert", "dripstone_caves", "end_barrens", "end_highlands", "end_midlands",
    "eroded_badlands", "flower_forest", "forest", "frozen_ocean", "frozen_peaks", "frozen_river", "grove",
    "ice_spikes", "jagged_peaks", "jungle", "lukewarm_ocean", "lush_caves", "mangrove_swamp", "meadow",
    "mushroom_fields", "nether_wastes", "ocean", "old_growth_birch_forest", "old_growth_pine_taiga",
    "old_growth_spruce_taiga", "pale_garden", "plains", "river", "savanna", "savanna_plateau", "small_end_islands",
    "snowy_beach", "snowy_plains", "snowy_slopes", "snowy_taiga", "soul_sand_valley", "sparse_jungle", "stony_peaks",
    "stony_shore", "sulfur_caves", "sunflower_plains", "swamp", "taiga", "the_end", "the_void", "warm_ocean",
    "warped_forest", "windswept_forest", "windswept_gravelly_hills", "windswept_hills", "windswept_savanna",
    "wooded_badlands",
];

/// Modern Java name → the legacy/Bedrock name whose id it shares.
pub const MODERN_ALIASES: &[(&str, &str)] = &[
    ("dark_forest", "roofed_forest"),
    ("end_barrens", "the_end"),
    ("end_highlands", "the_end"),
    ("end_midlands", "the_end"),
    ("small_end_islands", "the_end"),
    ("eroded_badlands", "mesa_bryce"),
    ("ice_spikes", "ice_plains_spikes"),
    ("nether_wastes", "hell"),
    ("old_growth_birch_forest", "birch_forest_mutated"),
    ("old_growth_pine_taiga", "mega_taiga"),
    ("old_growth_spruce_taiga", "redwood_taiga_mutated"),
    ("snowy_beach", "cold_beach"),
    ("snowy_taiga", "cold_taiga"),
    ("soul_sand_valley", "soulsand_valley"),
    ("sparse_jungle", "jungle_edge"),
    ("stony_shore", "stone_beach"),
    ("windswept_forest", "extreme_hills_plus_trees"),
    ("windswept_gravelly_hills", "extreme_hills_mutated"),
    ("windswept_hills", "extreme_hills"),
    ("windswept_savanna", "savanna_mutated"),
    ("wooded_badlands", "mesa_plateau_stone"),
    // Bedrock has no void biome; superflat-void columns read as plains.
    ("the_void", "plains"),
];

/// Legacy/Bedrock-only name → the current Java biome it became.
pub const LEGACY_FOLD: &[(&str, &str)] = &[
    ("hell", "nether_wastes"),
    ("roofed_forest", "dark_forest"),
    ("roofed_forest_mutated", "dark_forest"),
    ("mesa", "badlands"),
    ("mesa_plateau", "badlands"),
    ("mesa_plateau_mutated", "badlands"),
    ("mesa_plateau_stone", "wooded_badlands"),
    ("mesa_plateau_stone_mutated", "wooded_badlands"),
    ("mesa_bryce", "eroded_badlands"),
    ("ice_plains", "snowy_plains"),
    ("ice_mountains", "snowy_plains"),
    ("ice_plains_spikes", "ice_spikes"),
    ("mushroom_island", "mushroom_fields"),
    ("mushroom_island_shore", "mushroom_fields"),
    ("desert_hills", "desert"),
    ("desert_mutated", "desert"),
    ("forest_hills", "forest"),
    ("taiga_hills", "taiga"),
    ("taiga_mutated", "taiga"),
    ("swampland", "swamp"),
    ("swampland_mutated", "swamp"),
    ("extreme_hills", "windswept_hills"),
    ("extreme_hills_edge", "windswept_hills"),
    ("extreme_hills_mutated", "windswept_gravelly_hills"),
    ("extreme_hills_plus_trees", "windswept_forest"),
    ("extreme_hills_plus_trees_mutated", "windswept_forest"),
    ("jungle_hills", "jungle"),
    ("jungle_mutated", "jungle"),
    ("jungle_edge", "sparse_jungle"),
    ("jungle_edge_mutated", "sparse_jungle"),
    ("bamboo_jungle_hills", "bamboo_jungle"),
    ("stone_beach", "stony_shore"),
    ("cold_beach", "snowy_beach"),
    ("birch_forest_hills", "birch_forest"),
    ("birch_forest_mutated", "old_growth_birch_forest"),
    ("birch_forest_hills_mutated", "old_growth_birch_forest"),
    ("cold_taiga", "snowy_taiga"),
    ("cold_taiga_hills", "snowy_taiga"),
    ("cold_taiga_mutated", "snowy_taiga"),
    ("mega_taiga", "old_growth_pine_taiga"),
    ("mega_taiga_hills", "old_growth_pine_taiga"),
    ("redwood_taiga_mutated", "old_growth_spruce_taiga"),
    ("redwood_taiga_hills_mutated", "old_growth_spruce_taiga"),
    ("savanna_mutated", "windswept_savanna"),
    ("savanna_plateau_mutated", "windswept_savanna"),
    ("soulsand_valley", "soul_sand_valley"),
    ("legacy_frozen_ocean", "frozen_ocean"),
    ("snowy_tundra", "snowy_plains"),
];

pub fn is_java_biome(name: &str) -> bool {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    JAVA_BIOMES.contains(&n)
}

/// The valid Java name for a table name (itself, or its modern fold).
pub fn to_java(name: &str) -> Option<&'static str> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    if let Some(j) = JAVA_BIOMES.iter().find(|j| **j == n) {
        return Some(j);
    }
    LEGACY_FOLD.iter().find(|(l, _)| *l == n).map(|(_, j)| *j)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_are_valid_java() {
        for (_, j) in LEGACY_FOLD {
            assert!(is_java_biome(j), "{j}");
        }
        for (m, _) in MODERN_ALIASES {
            assert!(is_java_biome(m), "{m}");
        }
        assert_eq!(to_java("minecraft:hell"), Some("nether_wastes"));
        assert_eq!(to_java("plains"), Some("plains"));
        assert_eq!(to_java("nonsense"), None);
    }
}
