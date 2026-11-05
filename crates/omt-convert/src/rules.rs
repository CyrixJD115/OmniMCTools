//! The registry rule table — OMT's own conversion rule schema.
//!
//! The RE pass established that UMT's item/stack mapping is **fully
//! data-driven** (`database.ndb` + `_tagModifiers` rule column; no id
//! literals in the binary — findings, "Task A" conclusion). OMT mirrors the
//! *architecture*, not the data: a versioned RON table the converter
//! consults, designed for the M3 `tools/registry-gen` generator to fill.
//!
//! What exists today (honest scope):
//! - `block_rules`: empty by default. Unmapped Java blocks pass through by
//!   name; any Java properties are **dropped with a loss row** because the
//!   Java-state → Bedrock-state semantic table is exactly what
//!   `registry-gen` will produce once a `client.jar` is pinned. No guessed
//!   mappings.
//! - `biome_ids`: the classic stable Bedrock 3D-biome id table (stable
//!   since 1.16 when per-block biomes shipped). Unmapped biome names fall
//!   back to ocean (0) with a loss row.
//! - `block_palette_version`: probe-derived from the 1.26.52 fixtures —
//!   every palette entry in 6228 checked entries carries `18168865`
//!   (packed major.minor.patch.rev = 1.21.60.33; see
//!   `omt-bedrock/examples/heightprobe.rs`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use omt_world::BlockState;

use crate::loss::{ConversionReport, LossKind};

/// Block palette entry version written into converted subchunks.
pub const DEFAULT_BLOCK_PALETTE_VERSION: i32 = 18168865;

/// One Java block → Bedrock mapping. `states` values are SNBT strings so
/// Bedrock's typed states (ints, bytes) survive the RON round-trip; they
/// are parsed to NBT at serialization time.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct BlockRule {
    /// Bedrock block name (namespaced). Same name ⇒ state-only mapping.
    pub name: String,
    /// Bedrock states to write. Empty ⇒ emit the block stateless.
    #[serde(default)]
    pub states: BTreeMap<String, String>,
}

/// The conversion rule table (loaded from RON or seeded).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct RegistryRules {
    /// Schema version, bumped when the shape changes.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Java block id (`minecraft:oak_stairs`, no property suffix) → rule.
    #[serde(default)]
    pub block_rules: BTreeMap<String, BlockRule>,
    /// Java biome id (`minecraft:plains`) → Bedrock numeric biome id.
    #[serde(default)]
    pub biome_ids: BTreeMap<String, i32>,
    /// `version` int written into generated Bedrock palette entries.
    #[serde(default = "default_palette_version")]
    pub block_palette_version: i32,
}

fn default_version() -> u32 {
    1
}

fn default_palette_version() -> i32 {
    DEFAULT_BLOCK_PALETTE_VERSION
}

impl Default for RegistryRules {
    fn default() -> Self {
        Self::seed()
    }
}

impl RegistryRules {
    /// The seeded rule table: stable biome ids, no block rules yet.
    pub fn seed() -> Self {
        Self {
            version: 1,
            block_rules: BTreeMap::new(),
            biome_ids: seed_biome_ids(),
            block_palette_version: DEFAULT_BLOCK_PALETTE_VERSION,
        }
    }
}

/// Load a rule table from RON. Malformed files are hard errors — a silently
/// ignored rule table would convert wrong.
pub fn load_rules(path: &Path) -> Result<RegistryRules, crate::error::ConvertError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| crate::error::ConvertError::Store(format!("reading {}: {e}", path.display())))?;
    ron::from_str(&text)
        .map_err(|e| crate::error::ConvertError::Store(format!("parsing {}: {e}", path.display())))
}

/// The classic Bedrock 3D-biome id table (stable since per-block biomes
/// shipped in 1.16). Seed scope only — `registry-gen` (client.jar) will
/// emit the complete current table; unknown names are loss-recorded and
/// fall back to ocean, never guessed.
///
/// **The single source for both directions**: the Java→Bedrock map and the
/// Bedrock→Java inverse are both derived from this one table, so they
/// cannot drift apart. The inverse keeps the alphabetically-first name per
/// Bedrock id (`snowy_plains` beats the `snowy_tundra` alias, `badlands`
/// beats `mesa`) — deterministic, and the aliases remain forward-mappable.
const SEED_BIOME_IDS: &[(&str, i32)] = &[
    ("minecraft:ocean", 0),
    ("minecraft:plains", 1),
    ("minecraft:desert", 2),
    ("minecraft:extreme_hills", 3),
    ("minecraft:forest", 4),
    ("minecraft:taiga", 5),
    ("minecraft:swamp", 6),
    ("minecraft:river", 7),
    ("minecraft:frozen_ocean", 10),
    ("minecraft:frozen_river", 11),
    ("minecraft:snowy_tundra", 12),
    ("minecraft:snowy_plains", 12),
    ("minecraft:mushroom_fields", 14),
    ("minecraft:beach", 16),
    ("minecraft:jungle", 19),
    ("minecraft:jungle_hills", 22),
    ("minecraft:deep_ocean", 24),
    ("minecraft:savanna", 35),
    ("minecraft:badlands", 37),
    ("minecraft:mesa", 37),
];

fn seed_biome_ids() -> BTreeMap<String, i32> {
    SEED_BIOME_IDS.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

/// The result of mapping one Java block palette entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedBlock {
    pub state: BlockState,
    /// True when Java properties existed and could not be carried over.
    pub states_dropped: bool,
}

impl RegistryRules {
    /// Map one Java palette entry. Java properties are dropped (with one
    /// loss row per distinct block id per chunk, deduped by the caller's
    /// `reported` set) unless an explicit rule maps them — the semantic
    /// Java-state → Bedrock-state table is registry-gen's job, and guessing
    /// state names would corrupt worlds rather than lose data.
    pub fn map_block(
        &self,
        java: &BlockState,
        reported: &mut BTreeSet<String>,
        report: &mut ConversionReport,
    ) -> MappedBlock {
        let had_properties = !java.properties.is_empty();
        if let Some(rule) = self.block_rules.get(&java.name) {
            let properties: Vec<(String, String)> = rule
                .states
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            return MappedBlock {
                state: BlockState { name: rule.name.clone(), properties },
                states_dropped: had_properties && rule.states.is_empty(),
            };
        }
        if had_properties && reported.insert(java.name.clone()) {
            report.record(
                LossKind::UnmappedId,
                format!("block {}", java.name),
                "palette-entry properties dropped: no semantic cross-edition state \
                 rule yet (registry-gen pending client.jar)",
            );
        }
        MappedBlock {
            state: BlockState { name: java.name.clone(), properties: Vec::new() },
            states_dropped: had_properties,
        }
    }

    /// Map a Java biome id name → Bedrock numeric id. Unknown names fall
    /// back to ocean (0) with one loss row per distinct name per chunk.
    pub fn map_biome(
        &self,
        java_name: &str,
        reported: &mut BTreeSet<String>,
        report: &mut ConversionReport,
    ) -> i32 {
        match self.biome_ids.get(java_name) {
            Some(&id) => id,
            None => {
                if reported.insert(java_name.to_string()) {
                    report.record(
                        LossKind::UnmappedId,
                        format!("biome {java_name}"),
                        "no Java→Bedrock biome id; fell back to ocean (0) — extend \
                         biome_ids in the rule table",
                    );
                }
                0
            }
        }
    }

    /// The Bedrock-id → Java-name inverse of [`Self::biome_ids`], derived
    /// on the fly so the two directions can never drift (see
    /// [`SEED_BIOME_IDS`]): iterating the name-sorted map, the
    /// alphabetically-first name per id wins (`snowy_plains` over
    /// `snowy_tundra`, `badlands` over `mesa`).
    pub fn biome_names(&self) -> BTreeMap<i32, String> {
        let mut out = BTreeMap::new();
        for (name, id) in &self.biome_ids {
            out.entry(*id).or_insert_with(|| name.clone());
        }
        out
    }

    /// Map a Bedrock numeric biome id → Java biome name for the reverse
    /// direction. Unknown ids fall back to ocean with one loss row per
    /// distinct id per chunk (deduped by the caller's `reported` set,
    /// keyed by the id).
    pub fn map_biome_to_java(
        &self,
        names: &BTreeMap<i32, String>,
        bedrock_id: i32,
        reported: &mut BTreeSet<i32>,
        report: &mut ConversionReport,
    ) -> String {
        if let Some(name) = names.get(&bedrock_id) {
            return name.clone();
        }
        if reported.insert(bedrock_id) {
            report.record(
                LossKind::UnmappedId,
                format!("biome id {bedrock_id}"),
                "no Bedrock→Java biome name; fell back to minecraft:ocean — extend \
                 biome_ids in the rule table",
            );
        }
        "minecraft:ocean".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(name: &str, props: &[(&str, &str)]) -> BlockState {
        BlockState {
            name: name.into(),
            properties: props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn seed_has_biome_ids_and_defaults() {
        let rules = RegistryRules::default();
        assert_eq!(rules.biome_ids.get("minecraft:plains"), Some(&1));
        assert_eq!(rules.biome_ids.get("minecraft:ocean"), Some(&0));
        assert_eq!(rules.block_palette_version, DEFAULT_BLOCK_PALETTE_VERSION);
        assert!(rules.block_rules.is_empty());
    }

    #[test]
    fn stateless_block_passes_through_without_loss() {
        let mut rules = RegistryRules::default();
        rules.biome_ids.clear(); // irrelevant here
        let mut report = ConversionReport::new();
        let mut seen = BTreeSet::new();
        let out = rules.map_block(&block("minecraft:stone", &[]), &mut seen, &mut report);
        assert_eq!(out.state.name, "minecraft:stone");
        assert!(report.is_clean());
    }

    #[test]
    fn stateful_block_drops_properties_with_one_loss_per_name() {
        let rules = RegistryRules::default();
        let mut report = ConversionReport::new();
        let mut seen = BTreeSet::new();
        let stairs = block("minecraft:oak_stairs", &[("facing", "east"), ("half", "bottom")]);
        let first = rules.map_block(&stairs, &mut seen, &mut report);
        let second = rules.map_block(&stairs, &mut seen, &mut report);
        assert!(first.states_dropped && second.states_dropped);
        assert!(first.state.properties.is_empty());
        assert_eq!(report.count_with_kind(LossKind::UnmappedId), 1, "deduped per chunk");
    }

    #[test]
    fn explicit_rule_maps_name_and_states() {
        let mut rules = RegistryRules::default();
        rules.block_rules.insert(
            "minecraft:oak_stairs".into(),
            BlockRule {
                name: "minecraft:oak_stairs".into(),
                states: [("weirdo_direction".to_string(), "1".to_string())].into_iter().collect(),
            },
        );
        let mut report = ConversionReport::new();
        let mut seen = BTreeSet::new();
        let out = rules.map_block(&stairs(), &mut seen, &mut report);
        assert_eq!(out.state.properties, vec![("weirdo_direction".to_string(), "1".to_string())]);
        assert!(report.is_clean());
    }

    #[test]
    fn biome_mapping_unknown_falls_back_to_ocean_with_loss() {
        let rules = RegistryRules::default();
        let mut report = ConversionReport::new();
        let mut seen = BTreeSet::new();
        assert_eq!(rules.map_biome("minecraft:plains", &mut seen, &mut report), 1);
        assert_eq!(rules.map_biome("minecraft:nowhere", &mut seen, &mut report), 0);
        assert_eq!(rules.map_biome("minecraft:nowhere", &mut seen, &mut report), 0);
        assert_eq!(report.count_with_kind(LossKind::UnmappedId), 1, "deduped per chunk");
    }

    #[test]
    fn inverse_biome_table_is_derived_from_the_seed() {
        let rules = RegistryRules::default();
        let names = rules.biome_names();
        // Forward/inverse agree for unambiguous ids.
        assert_eq!(names.get(&1).map(String::as_str), Some("minecraft:plains"));
        assert_eq!(names.get(&0).map(String::as_str), Some("minecraft:ocean"));
        // Aliased ids keep the alphabetically-first name per id (documented
        // deterministic tie-break): snowy_plains beats snowy_tundra,
        // badlands beats mesa.
        assert_eq!(names.get(&12).map(String::as_str), Some("minecraft:snowy_plains"));
        assert_eq!(names.get(&37).map(String::as_str), Some("minecraft:badlands"));
        // Both alias directions still map forward.
        assert_eq!(rules.map_biome("minecraft:snowy_plains", &mut BTreeSet::new(), &mut ConversionReport::new()), 12);
        assert_eq!(rules.map_biome("minecraft:mesa", &mut BTreeSet::new(), &mut ConversionReport::new()), 37);
    }

    #[test]
    fn reverse_biome_unknown_id_falls_back_to_ocean_with_loss() {
        let rules = RegistryRules::default();
        let names = rules.biome_names();
        let mut report = ConversionReport::new();
        let mut seen = BTreeSet::new();
        assert_eq!(rules.map_biome_to_java(&names, 1, &mut seen, &mut report), "minecraft:plains");
        assert_eq!(rules.map_biome_to_java(&names, 999, &mut seen, &mut report), "minecraft:ocean");
        assert_eq!(rules.map_biome_to_java(&names, 999, &mut seen, &mut report), "minecraft:ocean");
        assert_eq!(report.count_with_kind(LossKind::UnmappedId), 1, "deduped per chunk");
    }

    fn stairs() -> BlockState {
        block("minecraft:oak_stairs", &[("facing", "east")])
    }
}
