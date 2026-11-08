//! The registry rule table — OMT's own conversion rule schema.
//!
//! The RE pass established that UMT's item/stack mapping is **fully
//! data-driven** (`database.ndb` + `_tagModifiers` rule column; no id
//! literals in the binary — findings, "Task A" conclusion). OMT mirrors the
//! *architecture*, not the data: a versioned RON table the converter
//! consults, filled by the `tools/registry-gen` generator (2026-11-08).
//!
//! Contents:
//! - [`RegistryRules::state_rules`] — the semantic Java-state → Bedrock
//!   block+state table, keyed `name[k=v,…]` with properties sorted and
//!   values in the exact SNBT canonical form `omt-bedrock` decodes palette
//!   NBT into (ints bare, bytes `b`-suffixed, strings raw). Generated from
//!   the pinned client.jar blockstates + Chunker registry data + the
//!   hand-authored `hand-maps.ron` semantics; see the table header.
//! - [`RegistryRules::block_rules`] — static renames (Java-only names
//!   with a stateless Bedrock twin); blocks absent here **and** in
//!   `state_rules` keep the honest loss-ledger passthrough.
//! - [`RegistryRules::biome_ids`] — the full Bedrock 3D-biome id table;
//!   unknown names fall back to ocean (0) with a loss row.
//! - [`RegistryRules::enchant_ids`] — Java enchantment id → Bedrock
//!   numeric id; unmapped enchantments (e.g. `sweeping_edge`) drop with a
//!   loss row, never guessed.
//! - `block_palette_version`: probe-derived from the 1.26.52 fixtures —
//!   every palette entry in 6228 checked entries carries `18168865`
//!   (packed 1.21.60.33; see `omt-bedrock/examples/heightprobe.rs`).
//!
//! The generated table is committed at `data/registry.ron` and embedded
//! with `include_str!` (versioned RON chosen over a generated `.rs`
//! constant: data stays human-reviewable and diffable, mirroring the
//! `omt-versions/registry.ron` precedent). A user-supplied `--rules` RON
//! can override the embedded default; malformed tables are hard errors.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

use omt_world::BlockState;

use crate::items::EnchantIdMap;
use crate::loss::{ConversionReport, LossKind};

/// Block palette entry version written into converted subchunks.
pub const DEFAULT_BLOCK_PALETTE_VERSION: i32 = 18168865;

/// One Java block → Bedrock mapping. `states` values are SNBT strings so
/// Bedrock's typed states (ints, bytes) survive the RON round-trip; they
/// are parsed to typed NBT at serialization time.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct BlockRule {
    /// Bedrock block name (namespaced). Same name ⇒ state-only mapping.
    pub name: String,
    /// Bedrock states to write. Empty ⇒ emit the block stateless.
    #[serde(default)]
    pub states: BTreeMap<String, String>,
}

/// The conversion rule table (embedded default, or loaded from RON).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct RegistryRules {
    /// Schema version, bumped when the shape changes.
    #[serde(default)]
    pub version: u32,
    /// Java block id (`minecraft:bricks`, no property suffix) → rule.
    /// Static renames only — stateful mapping lives in [`state_rules`].
    #[serde(default)]
    pub block_rules: BTreeMap<String, BlockRule>,
    /// `java name[k=v,…]` → Bedrock name + states. Keys are sorted,
    /// canonical SNBT strings (see module docs).
    #[serde(default)]
    pub state_rules: BTreeMap<String, BlockRule>,
    /// Java biome id (`minecraft:plains`) → Bedrock numeric biome id.
    #[serde(default)]
    pub biome_ids: BTreeMap<String, i32>,
    /// Java enchantment id (`minecraft:sharpness`) → Bedrock numeric id.
    #[serde(default)]
    pub enchant_ids: BTreeMap<String, i16>,
    /// `version` int written into generated Bedrock palette entries.
    #[serde(default = "default_palette_version")]
    pub block_palette_version: i32,
}

fn default_palette_version() -> i32 {
    DEFAULT_BLOCK_PALETTE_VERSION
}

/// `Default` is the seed baseline (empty rules, seeded biomes) — the
/// pre-registry behavior. Production conversion uses
/// [`RegistryRules::embedded`].
impl Default for RegistryRules {
    fn default() -> Self {
        Self::seed()
    }
}

impl RegistryRules {
    /// The empty baseline: no block/state/enchant rules, seeded biomes
    /// only. This is the pre-registry behavior (loss-ledger passthrough);
    /// production conversion uses [`Self::embedded`].
    pub fn seed() -> Self {
        Self { biome_ids: seed_biome_ids(), ..Self::empty() }
    }

    /// All-empty table (no biome seed either); the base for [`Self::seed`].
    fn empty() -> Self {
        Self {
            version: 1,
            block_rules: BTreeMap::new(),
            state_rules: BTreeMap::new(),
            biome_ids: BTreeMap::new(),
            enchant_ids: BTreeMap::new(),
            block_palette_version: DEFAULT_BLOCK_PALETTE_VERSION,
        }
    }

    /// The embedded generated table (`data/registry.ron`, committed with
    /// the repo). Parsing it can only fail if the committed artifact were
    /// corrupted in-tree; `tools/registry-gen` regenerates it and a unit
    /// test parses it on every build, so the `expect` documents an
    /// impossible-by-construction state rather than a runtime risk.
    pub fn embedded() -> &'static Self {
        static EMBEDDED: OnceLock<RegistryRules> = OnceLock::new();
        EMBEDDED.get_or_init(|| {
            ron::from_str(EMBEDDED_TABLE)
                .expect("embedded data/registry.ron is generated and unit-tested")
        })
    }

    /// The [`EnchantIdMap`] view of [`Self::enchant_ids`] for the item
    /// converters.
    pub fn enchant_id_map(&self) -> EnchantIdMap {
        EnchantIdMap::from_pairs(self.enchant_ids.clone())
    }
}

/// The embedded generated table (see `data/registry.ron`).
const EMBEDDED_TABLE: &str = include_str!("../data/registry.ron");

/// Load a rule table from RON. Malformed files are hard errors — a silently
/// ignored rule table would convert wrong.
pub fn load_rules(path: &Path) -> Result<RegistryRules, crate::error::ConvertError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| crate::error::ConvertError::Store(format!("reading {}: {e}", path.display())))?;
    ron::from_str(&text)
        .map_err(|e| crate::error::ConvertError::Store(format!("parsing {}: {e}", path.display())))
}

/// Canonical state key: `name[k=v,k=v]`, keys sorted, values exactly as
/// the palette decode renders them. The single key form for both rule
/// directions (Java keys from chunk palettes, Bedrock keys likewise).
pub fn state_key(name: &str, properties: &[(String, String)]) -> String {
    if properties.is_empty() {
        return name.to_string();
    }
    let sorted: BTreeMap<&str, &str> =
        properties.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let inner: Vec<String> = sorted.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{name}[{}]", inner.join(","))
}

/// The classic Bedrock 3D-biome id seed (stable since per-block biomes
/// shipped in 1.16). The generated table carries the full current set;
/// these aliases remain forward-mappable for older-world inputs.
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

/// The result of mapping one palette entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedBlock {
    pub state: BlockState,
    /// True when source properties existed and could not be carried over.
    pub states_dropped: bool,
}

impl RegistryRules {
    /// Map one Java palette entry (Java → Bedrock). Exact state rule
    /// first, then the static rename rule, then the name-only passthrough
    /// with a loss row (per distinct block id per chunk, deduped by the
    /// caller's `reported` set) — guessing state names would corrupt
    /// worlds rather than lose data.
    pub fn map_block(
        &self,
        java: &BlockState,
        reported: &mut BTreeSet<String>,
        report: &mut ConversionReport,
    ) -> MappedBlock {
        let had_properties = !java.properties.is_empty();
        if had_properties {
            let key = state_key(&java.name, &java.properties);
            if let Some(rule) = self.state_rules.get(&key) {
                return rule_output(rule, false);
            }
        }
        if let Some(rule) = self.block_rules.get(&java.name) {
            // Static rename: any properties are uncovered (documented
            // above — renames are stateless by construction).
            return rule_output(rule, had_properties);
        }
        if had_properties && reported.insert(java.name.clone()) {
            report.record(
                LossKind::UnmappedId,
                format!("block {}", java.name),
                "palette-entry properties dropped: no semantic cross-edition state \
                 rule (extend tools/registry-gen hand-maps and regenerate)",
            );
        }
        MappedBlock {
            state: BlockState { name: java.name.clone(), properties: Vec::new() },
            states_dropped: had_properties,
        }
    }

    /// Map one Bedrock palette entry (Bedrock → Java) through the inverse
    /// derived from this table. See [`ReverseBlockRules::map_block`].
    pub fn map_block_to_java(
        &self,
        bedrock: &BlockState,
        reported: &mut BTreeSet<String>,
        report: &mut ConversionReport,
    ) -> MappedBlock {
        self.reverse().map_block(bedrock, reported, report)
    }

    /// The derived inverse rules (Bedrock → Java). Cached per table
    /// instance by the caller for batch conversions — see
    /// [`ReverseBlockRules`].
    pub fn reverse(&self) -> ReverseBlockRules {
        ReverseBlockRules::build(self)
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

fn rule_output(rule: &BlockRule, states_dropped: bool) -> MappedBlock {
    MappedBlock {
        state: BlockState {
            name: rule.name.clone(),
            properties: rule.states.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        },
        states_dropped,
    }
}

/// The Bedrock → Java inverse of a [`RegistryRules`], derived from the one
/// seed table so the directions cannot drift (same principle as
/// [`RegistryRules::biome_names`]).
///
/// - `states`: Bedrock `name[k=v,…]` → Java name + properties. Built by
///   iterating the Java-key-sorted `state_rules`, so when two Java states
///   map to one Bedrock state (reverse collisions, counted by the
///   generator) the alphabetically-first Java state wins — deterministic
///   and documented.
/// - `names`: Bedrock block name → Java name (alphabetically-first Java
///   id over `block_rules` + `state_rules`), the fallback when an exact
///   Bedrock state combo has no forward entry (e.g. a Bedrock-only state
///   of a shared block): name survives, properties drop with a loss row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReverseBlockRules {
    states: BTreeMap<String, BlockState>,
    names: BTreeMap<String, String>,
}

impl ReverseBlockRules {
    pub fn build(rules: &RegistryRules) -> Self {
        let mut out = Self::default();
        for (java_key, rule) in &rules.block_rules {
            out.names.entry(rule.name.clone()).or_insert_with(|| java_key.clone());
        }
        for (java_key, rule) in &rules.state_rules {
            let (java_name, java_props) = parse_state_key(java_key);
            let bedrock_props: Vec<(String, String)> =
                rule.states.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let bedrock_key = state_key(&rule.name, &bedrock_props);
            out.names.entry(rule.name.clone()).or_insert_with(|| java_name.clone());
            out.states
                .entry(bedrock_key)
                .or_insert_with(|| BlockState { name: java_name, properties: java_props });
        }
        out
    }

    /// Map one Bedrock palette entry. Exact state inverse first, then the
    /// name inverse (properties dropped with a loss row), then the
    /// name-only passthrough (modded / unknown blocks; stateful entries
    /// get one deduped loss row per name).
    pub fn map_block(
        &self,
        bedrock: &BlockState,
        reported: &mut BTreeSet<String>,
        report: &mut ConversionReport,
    ) -> MappedBlock {
        let had_properties = !bedrock.properties.is_empty();
        let key = state_key(&bedrock.name, &bedrock.properties);
        if let Some(java) = self.states.get(&key) {
            return MappedBlock { state: java.clone(), states_dropped: false };
        }
        if let Some(java_name) = self.names.get(&bedrock.name) {
            if had_properties {
                if reported.insert(bedrock.name.clone()) {
                    report.record(
                        LossKind::FieldDropped,
                        format!("block {}", bedrock.name),
                        "Bedrock state combo has no Java-table entry; properties \
                         dropped, name mapped — extend tools/registry-gen and regenerate",
                    );
                }
                return MappedBlock {
                    state: BlockState { name: java_name.clone(), properties: Vec::new() },
                    states_dropped: true,
                };
            }
            return MappedBlock {
                state: BlockState { name: java_name.clone(), properties: Vec::new() },
                states_dropped: false,
            };
        }
        if had_properties && reported.insert(bedrock.name.clone()) {
            report.record(
                LossKind::UnmappedId,
                format!("block {}", bedrock.name),
                "palette-entry properties dropped: no Bedrock→Java state inverse \
                 (extend tools/registry-gen hand-maps and regenerate)",
            );
        }
        MappedBlock {
            state: BlockState { name: bedrock.name.clone(), properties: Vec::new() },
            states_dropped: had_properties,
        }
    }
}

/// Split `name[k=v,k=v]` back into name + property pairs (the inverse of
/// [`state_key`]). Keys and values cannot contain `[`, `]`, `,` or `=` in
/// the registry data, so a plain scan is exact.
fn parse_state_key(key: &str) -> (String, Vec<(String, String)>) {
    let Some((name, rest)) = key.split_once('[') else {
        return (key.to_string(), Vec::new());
    };
    let rest = rest.strip_suffix(']').unwrap_or(rest);
    let mut props = Vec::new();
    for part in rest.split(',') {
        if let Some((k, v)) = part.split_once('=') {
            props.push((k.to_string(), v.to_string()));
        }
    }
    (name.to_string(), props)
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
    fn seed_has_biome_ids_and_defaults_and_no_rules() {
        let rules = RegistryRules::seed();
        assert_eq!(rules.biome_ids.get("minecraft:plains"), Some(&1));
        assert_eq!(rules.block_palette_version, DEFAULT_BLOCK_PALETTE_VERSION);
        assert!(rules.block_rules.is_empty());
        assert!(rules.state_rules.is_empty());
        assert!(rules.enchant_ids.is_empty());
    }

    #[test]
    fn embedded_table_parses_and_covers_air() {
        let rules = RegistryRules::embedded();
        assert!(rules.version >= 1);
        // Air resolves through passthrough without loss.
        let mut report = ConversionReport::new();
        let out = rules.map_block(&block("minecraft:air", &[]), &mut BTreeSet::new(), &mut report);
        assert_eq!(out.state.name, "minecraft:air");
        assert!(report.is_clean());
        assert!(!rules.state_rules.is_empty(), "generated table carries state rules");
        assert!(!rules.enchant_ids.is_empty(), "generated table carries enchantments");
    }

    #[test]
    fn stateless_block_passes_through_without_loss() {
        let mut rules = RegistryRules::seed();
        rules.biome_ids.clear(); // irrelevant here
        let mut report = ConversionReport::new();
        let out = rules.map_block(&block("minecraft:stone", &[]), &mut BTreeSet::new(), &mut report);
        assert_eq!(out.state.name, "minecraft:stone");
        assert!(report.is_clean());
    }

    #[test]
    fn stateful_block_drops_properties_with_one_loss_per_name() {
        let rules = RegistryRules::seed();
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
    fn state_rule_maps_exact_combo() {
        let mut rules = RegistryRules::seed();
        rules.state_rules.insert(
            "minecraft:oak_stairs[facing=east,half=top]".into(),
            BlockRule {
                name: "minecraft:oak_stairs".into(),
                states: [("upside_down_bit".to_string(), "1b".to_string())].into_iter().collect(),
            },
        );
        let mut report = ConversionReport::new();
        let out = rules.map_block(
            &block("minecraft:oak_stairs", &[("half", "top"), ("facing", "east")]),
            &mut BTreeSet::new(),
            &mut report,
        );
        assert_eq!(out.state.properties, vec![("upside_down_bit".to_string(), "1b".to_string())]);
        assert!(!out.states_dropped);
        assert!(report.is_clean());
        // A different combo of the same block misses and stays honest.
        let miss = rules.map_block(
            &block("minecraft:oak_stairs", &[("facing", "east")]),
            &mut BTreeSet::new(),
            &mut report,
        );
        assert!(miss.states_dropped);
        assert_eq!(report.count_with_kind(LossKind::UnmappedId), 1);
    }

    #[test]
    fn reverse_state_rule_round_trips() {
        let mut rules = RegistryRules::seed();
        rules.state_rules.insert(
            "minecraft:oak_stairs[facing=north,half=bottom]".into(),
            BlockRule {
                name: "minecraft:oak_stairs".into(),
                states: [
                    ("weirdo_direction".to_string(), "3".to_string()),
                    ("upside_down_bit".to_string(), "0b".to_string()),
                ]
                .into_iter()
                .collect(),
            },
        );
        let reverse = rules.reverse();
        let mut report = ConversionReport::new();
        let out = reverse.map_block(
            &block("minecraft:oak_stairs", &[("upside_down_bit", "0b"), ("weirdo_direction", "3")]),
            &mut BTreeSet::new(),
            &mut report,
        );
        assert_eq!(out.state.name, "minecraft:oak_stairs");
        assert_eq!(
            out.state.properties,
            vec![("facing".to_string(), "north".to_string()), ("half".to_string(), "bottom".to_string())]
        );
        assert!(!out.states_dropped);
        assert!(report.is_clean());
    }

    #[test]
    fn reverse_static_rename_and_unmapped_passthrough() {
        let mut rules = RegistryRules::seed();
        rules.block_rules.insert(
            "minecraft:bricks".into(),
            BlockRule { name: "minecraft:brick_block".into(), states: Default::default() },
        );
        let reverse = rules.reverse();
        let mut report = ConversionReport::new();
        let out = reverse.map_block(&block("minecraft:brick_block", &[]), &mut BTreeSet::new(), &mut report);
        assert_eq!(out.state.name, "minecraft:bricks");
        // Modded blocks pass through by name; stateful ones report once.
        let modded = block("panascais_caves:gloopstone", &[("goo", "1")]);
        let mut seen = BTreeSet::new();
        let first = reverse.map_block(&modded, &mut seen, &mut report);
        let second = reverse.map_block(&modded, &mut seen, &mut report);
        assert_eq!(first.state.name, "panascais_caves:gloopstone");
        assert_eq!(report.count_with_kind(LossKind::UnmappedId), 1, "deduped per chunk");
        assert!(second.states_dropped);
    }

    #[test]
    fn explicit_rule_maps_name_and_states() {
        let mut rules = RegistryRules::seed();
        rules.block_rules.insert(
            "minecraft:oak_stairs".into(),
            BlockRule {
                name: "minecraft:oak_stairs".into(),
                states: [("weirdo_direction".to_string(), "1".to_string())].into_iter().collect(),
            },
        );
        let mut report = ConversionReport::new();
        let out = rules.map_block(&stairs(), &mut BTreeSet::new(), &mut report);
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

    #[test]
    fn state_keys_are_sorted_and_parse_back() {
        assert_eq!(state_key("minecraft:a", &[]), "minecraft:a");
        assert_eq!(
            state_key("minecraft:a", &[("b".into(), "1".into()), ("a".into(), "2".into())]),
            "minecraft:a[a=2,b=1]"
        );
        let (name, props) = parse_state_key("minecraft:a[a=2,b=1]");
        assert_eq!(name, "minecraft:a");
        assert_eq!(props, vec![("a".to_string(), "2".to_string()), ("b".to_string(), "1".to_string())]);
    }

    fn stairs() -> BlockState {
        block("minecraft:oak_stairs", &[("facing", "east")])
    }
}
