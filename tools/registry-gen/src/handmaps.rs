//! Hand-authored semantic mappings (`hand-maps.ron`).
//!
//! The generator derives everything it can from data (registry joins,
//! property-name identity); the *semantic* correspondence that data cannot
//! express (Java `facing=east` ⇄ Bedrock `weirdo_direction=0`, crosswalks
//! like Java `bricks` ⇄ Bedrock `brick_block`) lives here, separated from
//! generated data so regeneration never overwrites manual work (PLAN §7).
//!
//! Value semantics are grounded in the MIT-licensed HiveGamesOSS/Chunker
//! reference (`BedrockStateGroups`/`BedrockStateTypes` value tables) and in
//! palette probes of the 1.26.52 fixture worlds; both are cited in the
//! emitted table header. Pattern families are reusable across the block
//! lists that share their semantics (stairs ×~70 woods, slabs, walls, …).

use std::collections::BTreeMap;

/// Where one Java property value lands on the Bedrock side.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropTarget {
    /// A Bedrock state: `(state key, canonical value)`.
    State(String, String),
    /// Several Bedrock states from one Java value (e.g. a hanging sign's
    /// `attached` feeds both `attached_bit` and `hanging`).
    States(Vec<(String, String)>),
    /// Rename the output block (template like [`PropTarget::Rename`]) and
    /// also write Bedrock states (e.g. comparator `powered` renames to
    /// `powered_comparator` *and* sets `output_lit_bit`).
    RenameStates(String, Vec<(String, String)>),
    /// Rename the output block. `{stem}` in the template is the mapped
    /// Bedrock name with the namespace and the pattern's `stem_strip`
    /// suffix removed (`oak_slab` − `_slab` → `oak` + `{stem}_double_slab`).
    Rename(String),
}

/// A reusable property-transform family (e.g. `stairs`, `slab`).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Pattern {
    /// Java property → Java value → target. Every Java value of a mapped
    /// property must appear here or the affected states are skipped with a
    /// warning (no guessed values).
    #[serde(default)]
    pub props: BTreeMap<String, BTreeMap<String, PropTarget>>,
    /// Bedrock states that always take a fixed canonical value (Java has
    /// no counterpart; e.g. leaves' `update_bit` is always written `0b`).
    #[serde(default)]
    pub fixed: BTreeMap<String, String>,
    /// Java properties intentionally not representable Bedrock-side
    /// (documented loss; e.g. leaves `distance`). Mapping them here means
    /// no runtime loss row for the covered states.
    #[serde(default)]
    pub drop: Vec<String>,
    /// Suffix stripped from the Bedrock name before `{stem}` substitution
    /// in [`PropTarget::Rename`] templates (slabs strip `_slab`).
    #[serde(default)]
    pub stem_strip: Option<String>,
    /// Generator-side multi-property computation (bitfields). Consumed
    /// properties are removed from the Java state before the per-property
    /// map runs; the computation may also *fail* for a combo (None), which
    /// skips that state with a warning.
    #[serde(default)]
    pub computed: Option<Computed>,
}

/// Named generator-side computations for state families that are a
/// *function of several Java properties at once* (bitfields and partial
/// tables) — not expressible as per-property maps. Semantics transcribed
/// from the Chunker reference tables (MIT; attributed in the table header).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Computed {
    /// `glow_lichen` etc: boolean faces → `multi_face_direction_bits`
    /// (bit order down, up, south, west, north, east).
    MultiFaceBits,
    /// `vine`: boolean sides → `vine_direction_bits`
    /// (bit order south, west, north, east).
    VineBits,
    /// mushroom blocks: boolean faces → `huge_mushroom_bits` (partial
    /// table; several Java combos share a Bedrock value).
    MushroomFaceBits,
    /// chiseled bookshelf: occupied-slot booleans → `books_stored`
    /// (bit j = slot j).
    BooksStored,
    /// buttons: `face` + `facing` → `facing_direction` int
    /// (floor→1, ceiling→0, wall n2/s3/w4/e5).
    ButtonFacing,
    /// levers: `face` + `facing` → `lever_direction` string
    /// (floor: up_north_south/up_east_west; wall: n/s/e/w;
    /// ceiling: down_north_south/down_east_west).
    LeverDirection,
}

/// One Java block's mapping to a Bedrock block, optionally via a pattern.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct BlockMap {
    /// Bedrock block id (`minecraft:brick_block`). Same name ⇒ keep.
    pub name: String,
    /// Pattern family id, or none for pure renames/static blocks.
    #[serde(default)]
    pub pattern: Option<String>,
}

/// The whole hand-map file.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct HandMaps {
    #[serde(default)]
    pub patterns: BTreeMap<String, Pattern>,
    #[serde(default)]
    pub blocks: BTreeMap<String, BlockMap>,
    /// Java biome name (short form) → Bedrock short name. Bridges the
    /// naming eras the numeric tables cannot (`snowy_plains` →
    /// `ice_plains`); ids still come from the Bedrock id table.
    #[serde(default)]
    pub biome_aliases: BTreeMap<String, String>,
}

impl HandMaps {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("parsing {}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pattern_and_block_entries() {
        let text = r#"(
            patterns: {
                "stairs": (
                    props: {
                        "facing": {
                            "east": state("weirdo_direction", "0"),
                            "north": state("weirdo_direction", "3"),
                        },
                        "half": {
                            "bottom": state("upside_down_bit", "0b"),
                            "top": state("upside_down_bit", "1b"),
                        },
                    },
                    fixed: {"minecraft:corner": "none"},
                    drop: ["waterlogged"],
                ),
                "slab": (
                    stem_strip: Some("_slab"),
                    props: {
                        "type": {
                            "top": state("minecraft:vertical_half", "top"),
                            "double": rename("{stem}_double_slab"),
                        },
                    },
                ),
            },
            blocks: {
                "minecraft:oak_stairs": (name: "minecraft:oak_stairs", pattern: Some("stairs")),
                "minecraft:bricks": (name: "minecraft:brick_block"),
            },
        )"#;
        let maps: HandMaps = ron::from_str(text).unwrap();
        let stairs = &maps.patterns["stairs"];
        assert_eq!(
            stairs.props["facing"]["east"],
            PropTarget::State("weirdo_direction".into(), "0".into())
        );
        assert_eq!(stairs.fixed["minecraft:corner"], "none");
        assert_eq!(stairs.drop, ["waterlogged"]);
        let slab = &maps.patterns["slab"];
        assert_eq!(slab.stem_strip.as_deref(), Some("_slab"));
        assert_eq!(
            slab.props["type"]["double"],
            PropTarget::Rename("{stem}_double_slab".into())
        );
        assert_eq!(maps.blocks["minecraft:oak_stairs"].pattern.as_deref(), Some("stairs"));
        assert_eq!(maps.blocks["minecraft:bricks"].name, "minecraft:brick_block");
        assert_eq!(maps.blocks["minecraft:bricks"].pattern, None);
    }
}
