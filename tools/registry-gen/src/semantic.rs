//! Semantic assembly: joins the Java and Bedrock registries into the
//! conversion rule table.
//!
//! Three paths per Java block, tried in order:
//! 1. **hand-map** (`hand-maps.ron`): pattern application with
//!    registry-validated output states — the semantic cross-edition
//!    knowledge (stairs direction encodings, crosswalk renames, …).
//! 2. **auto-join**: same block name on both sides with identical
//!    property-name sets; values map through canonical-form normalization
//!    (Java `"true"` ⇄ Bedrock byte `1b`, ints and strings pass through).
//! 3. nothing — the block stays unmapped and is reported as a gap; the
//!    runtime keeps the honest loss-row passthrough for it.
//!
//! Every emitted Bedrock state is validated against the Bedrock registry
//! (an exact name+state combo must exist) — a wrong transform can never
//! enter the table silently; it becomes a warning and a skipped state.

use std::collections::{BTreeMap, BTreeSet};

use omt_convert::{BlockRule, RegistryRules};

use crate::handmaps::{HandMaps, Pattern, PropTarget};
use crate::model::{state_key, BedrockBlock, JavaBlock};

/// Per-block gap record (block, reason) — the honest scoping report.
pub type Gap = (String, String);

/// Assembly statistics, surfaced by the CLI and written next to the table.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AssemblyStats {
    pub java_blocks: usize,
    pub java_states: usize,
    /// Blocks with at least one mapped state or a static rule.
    pub blocks_mapped: usize,
    /// Valid Java states that received a rule.
    pub states_mapped: usize,
    pub static_rules: usize,
    pub state_rules: usize,
    pub gaps: Vec<Gap>,
    /// Data-quality findings that do not block a block (deduped).
    pub warnings: BTreeSet<String>,
    /// (java key kept, java key dropped) collisions where two Java states
    /// map to the same Bedrock state; the runtime inverse keeps the
    /// alphabetically first Java state (documented, same tie-break as
    /// biome ids).
    pub reverse_collisions: Vec<(String, String)>,
}

/// Assemble the rule table from the three inputs plus the seed biome and
/// enchantment tables.
pub fn assemble(
    java: &BTreeMap<String, JavaBlock>,
    bedrock: &BTreeMap<String, BedrockBlock>,
    hand: &HandMaps,
    biome_ids: BTreeMap<String, i32>,
    enchant_ids: BTreeMap<String, i16>,
    seed: &RegistryRules,
) -> (RegistryRules, AssemblyStats) {
    let mut stats = AssemblyStats::default();
    let mut state_rules: BTreeMap<String, BlockRule> = BTreeMap::new();
    let mut block_rules: BTreeMap<String, BlockRule> = BTreeMap::new();
    let mut reverse_seen: BTreeMap<String, String> = BTreeMap::new();

    for (name, block) in java {
        stats.java_blocks += 1;
        stats.java_states += block.states.len();

        let Some(hand_entry) = hand.blocks.get(name) else {
            if !bedrock.contains_key(&block.name) {
                // No hand-map and the Bedrock registry has no such block:
                // report it — the runtime passthrough must stay honest.
                stats.gaps.push((
                    name.clone(),
                    "no hand-map and absent from the Bedrock registry".into(),
                ));
                continue;
            }
            if let Some(states) = auto_join(block, bedrock, &mut stats) {
                for (java_state, out_states) in states {
                    insert_one(
                        name,
                        &java_state,
                        &block.name,
                        &out_states,
                        &mut state_rules,
                        &mut reverse_seen,
                        &mut stats,
                    );
                }
                stats.blocks_mapped += 1;
            }
            continue;
        };

        let target = &hand_entry.name;
        let Some(target_block) = bedrock.get(target) else {
            stats.gaps.push((
                name.clone(),
                format!("hand-map target {target} not in Bedrock registry"),
            ));
            continue;
        };
        let pattern = match hand_entry.pattern.as_deref() {
            None => None,
            Some(id) => match hand.patterns.get(id) {
                Some(p) => Some(p),
                None => {
                    stats.gaps.push((name.clone(), format!("unknown pattern id {id:?}")));
                    continue;
                }
            },
        };
        let stem = target
            .trim_start_matches("minecraft:")
            .strip_suffix(pattern.and_then(|p| p.stem_strip.as_deref()).unwrap_or(""))
            .unwrap_or_else(|| target.trim_start_matches("minecraft:"));

        let property_less = block.states.iter().all(|s| s.is_empty());
        let mut mapped = 0usize;
        if pattern.is_none() && property_less {
            // Static rename of a property-less block: a per-state rule is
            // meaningless, so one block rule carries the rename — validated
            // against the target's property-less default state.
            if target_block.states.iter().any(|s| s.is_empty()) {
                block_rules.insert(
                    name.clone(),
                    BlockRule { name: target.clone(), states: BTreeMap::new() },
                );
                stats.static_rules += 1;
                mapped += 1;
            } else {
                stats.gaps.push((
                    name.clone(),
                    format!("hand-map target {target} has no property-less state"),
                ));
            }
        } else {
            for state in &block.states {
                if let Some((out_name, out_states)) =
                    apply_pattern(state, pattern, target, stem, bedrock, &mut stats)
                {
                    insert_one(
                        name,
                        state,
                        &out_name,
                        &out_states,
                        &mut state_rules,
                        &mut reverse_seen,
                        &mut stats,
                    );
                    mapped += 1;
                }
            }
            if block.states.is_empty() && target_block.states.iter().any(|s| s.is_empty()) {
                // Patterned block with no states: one block rule keeps the
                // default state honest.
                block_rules.insert(
                    name.clone(),
                    BlockRule { name: target.clone(), states: BTreeMap::new() },
                );
                stats.static_rules += 1;
                mapped += 1;
            }
        }
        if mapped > 0 {
            stats.blocks_mapped += 1;
        } else if !block.states.is_empty() {
            stats.gaps.push((name.clone(), "no hand-mapped state passed validation".into()));
        }
    }

    stats.state_rules = state_rules.len();
    let mut rules = RegistryRules {
        version: seed.version,
        block_rules,
        state_rules,
        biome_ids,
        enchant_ids,
        block_palette_version: seed.block_palette_version,
    };
    // Seed biome aliases stay forward-mappable alongside generated ids.
    for (name, id) in &seed.biome_ids {
        rules.biome_ids.entry(name.clone()).or_insert(*id);
    }
    (rules, stats)
}

/// One auto-joined mapping: (java state — keys the forward rule, mapped
/// Bedrock state — the rule body).
type JoinedState = (BTreeMap<String, String>, BTreeMap<String, String>);

/// Auto-join: same name, identical property sets, every Java value
/// representable in the Bedrock value set (modulo bool canonicalization).
/// Returns (java state, mapped state) pairs — the forward rule keys on the
/// JAVA state, the rule body carries the Bedrock form (out name is the
/// same block).
fn auto_join(
    block: &JavaBlock,
    bedrock: &BTreeMap<String, BedrockBlock>,
    stats: &mut AssemblyStats,
) -> Option<Vec<JoinedState>> {
    let Some(target) = bedrock.get(&block.name) else {
        // Java-only block (no Bedrock equivalent): an explicit gap so the
        // loss-ledger passthrough it gets at runtime is accounted for.
        stats.gaps.push((
            block.name.clone(),
            "java block absent from the Bedrock registry (java-only name;              converts by name with a loss row)"
                .into(),
        ));
        return None;
    };
    if block.props.is_empty() && target.props().is_empty() {
        // Static same-name block: the name-only passthrough already
        // converts it; no table entry needed.
        return None;
    }
    let target_props = target.props();
    if block.props.keys().ne(target_props.keys()) {
        stats.gaps.push((
            block.name.clone(),
            format!(
                "same-name block with differing properties J={:?} B={:?}",
                block.props.keys().collect::<Vec<_>>(),
                target_props.keys().collect::<Vec<_>>(),
            ),
        ));
        return None;
    }
    let mut out = Vec::with_capacity(block.states.len());
    for state in &block.states {
        let mut mapped: BTreeMap<String, String> = BTreeMap::new();
        let mut ok = true;
        for (prop, value) in state {
            let allowed = &target_props[prop];
            let canonical = bool_canonical(value, allowed).unwrap_or_else(|| value.clone());
            if !allowed.contains(&canonical) {
                stats.gaps.push((
                    block.name.clone(),
                    format!("value {prop}={value} absent from Bedrock value set"),
                ));
                ok = false;
                break;
            }
            mapped.insert(prop.clone(), canonical);
        }
        if ok {
            out.push((state.clone(), mapped));
        }
    }
    if out.is_empty() {
        stats.gaps.push((block.name.clone(), "no auto-joined state passed validation".into()));
        None
    } else {
        Some(out)
    }
}

/// Normalize Java booleans to Bedrock byte form when the target value set
/// wants it (`"true"` → `"1b"`).
fn bool_canonical(value: &str, allowed: &BTreeSet<String>) -> Option<String> {
    match value {
        "true" if allowed.contains("1b") => Some("1b".into()),
        "false" if allowed.contains("0b") => Some("0b".into()),
        _ => None,
    }
}

/// Apply one pattern to one Java state. `None` ⇒ skipped (reason already
/// recorded as a gap). Returns the output name + states, validated
/// against the Bedrock registry (which is also where *renamed* targets —
/// `{stem}_double_slab`, `powered_repeater`, `light_block_{value}` — are
/// looked up).
#[allow(clippy::too_many_arguments)]
fn apply_pattern(
    state: &BTreeMap<String, String>,
    pattern: Option<&Pattern>,
    target: &str,
    stem: &str,
    bedrock: &BTreeMap<String, BedrockBlock>,
    stats: &mut AssemblyStats,
) -> Option<(String, BTreeMap<String, String>)> {
    let mut out_name = target.to_string();
    let mut out_states: BTreeMap<String, String> = BTreeMap::new();
    let target_block = bedrock.get(target)?;
    let target_props = target_block.props();

    // Multi-property computation first (bitfields consume their inputs).
    if let Some(kind) = pattern.and_then(|p| p.computed) {
        match crate::computed::run_computed(kind, state, &mut out_states) {
            Ok(consumed) => {
                let mut remainder = state.clone();
                remainder.retain(|k, _| !consumed.iter().any(|c| c == k));
                for (prop, value) in &remainder {
                    if pattern.is_some_and(|p| p.drop.iter().any(|d| d == prop)) {
                        continue;
                    }
                    // Leftover properties may still carry explicit pattern
                    // entries (e.g. button `powered` → `button_pressed_bit`).
                    if let Some(PropTarget::State(bprop, bval)) =
                        pattern.and_then(|p| p.props.get(prop)).and_then(|m| m.get(value))
                    {
                        out_states.insert(bprop.clone(), bval.clone());
                        continue;
                    }
                    let allowed = target_props.get(prop);
                    let canonical = allowed
                        .and_then(|a| bool_canonical(value, a))
                        .unwrap_or_else(|| value.to_owned());
                    if allowed.is_some_and(|a| a.contains(&canonical)) {
                        out_states.insert(prop.clone(), canonical);
                    } else {
                        stats.gaps.push((
                            state_key(target, state),
                            format!("computed leftover property {prop}={value} unmapped"),
                        ));
                        return None;
                    }
                }
                return finish_state(out_name, out_states, target, bedrock, pattern, stats);
            }
            Err(reason) => {
                stats.gaps.push((state_key(target, state), reason));
                return None;
            }
        }
    }

    for (prop, value) in state {
        let target_spec = pattern.and_then(|p| p.props.get(prop)).and_then(|m| m.get(value));
        match target_spec {
            Some(PropTarget::State(bprop, bval)) => {
                out_states.insert(bprop.clone(), bval.clone());
            }
            Some(PropTarget::States(pairs)) => {
                for (bprop, bval) in pairs {
                    out_states.insert(bprop.clone(), bval.clone());
                }
            }
            Some(PropTarget::Rename(template)) => {
                out_name = render_rename(template, stem, value);
            }
            Some(PropTarget::RenameStates(template, pairs)) => {
                out_name = render_rename(template, stem, value);
                for (bprop, bval) in pairs {
                    out_states.insert(bprop.clone(), bval.clone());
                }
            }
            None => {
                if pattern.is_some_and(|p| p.drop.iter().any(|d| d == prop)) {
                    continue; // documented loss
                }
                // Identity fallthrough when the Bedrock side has the same
                // property name and value (e.g. cauldron `fill_level`).
                let allowed = target_props.get(prop);
                let canonical = allowed
                    .and_then(|a| bool_canonical(value, a))
                    .unwrap_or_else(|| value.to_owned());
                if allowed.is_some_and(|a| a.contains(&canonical)) {
                    out_states.insert(prop.clone(), canonical);
                } else {
                    stats.gaps.push((
                        state_key(target, state),
                        format!(
                            "property {prop}={value} has no pattern entry and no identity \
                             fallthrough"
                        ),
                    ));
                    return None;
                }
            }
        }
    }
    finish_state(out_name, out_states, target, bedrock, pattern, stats)
}

/// Validate the assembled output against the Bedrock registry and apply
/// pattern-fixed states. Shared by the plain and computed paths.
fn finish_state(
    out_name: String,
    mut out_states: BTreeMap<String, String>,
    target: &str,
    bedrock: &BTreeMap<String, BedrockBlock>,
    pattern: Option<&Pattern>,
    stats: &mut AssemblyStats,
) -> Option<(String, BTreeMap<String, String>)> {
    if let Some(p) = pattern {
        for (bprop, bval) in &p.fixed {
            // Fixed states *fill* — a mapped value (e.g. a real slab half)
            // is never overwritten by the pattern's default.
            out_states.entry(bprop.clone()).or_insert_with(|| bval.clone());
        }
    }
    let validated = bedrock.get(&out_name).unwrap_or_else(|| {
        stats.warnings.insert(format!("{target}: rename target {out_name} not in registry"));
        // Validation below will fail against an empty block — no state is
        // emitted for a nonexistent target.
        EMPTY_BEDROCK.get_or_init(BedrockBlock::default)
    });
    if !validated.has_state(&out_states) {
        stats.gaps.push((
            state_key(target, &out_states),
            format!(
                "output {out_name}[{}] not in Bedrock registry",
                out_states.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","),
            ),
        ));
        return None;
    }
    Some((out_name, out_states))
}

/// Empty validation fallback for missing rename targets (always fails
/// `has_state`, so nothing is silently emitted).
static EMPTY_BEDROCK: std::sync::OnceLock<BedrockBlock> = std::sync::OnceLock::new();

/// Rename template: `{stem}` (the mapped name minus `stem_strip`) and
/// `{value}` (the Java value of the renaming property).
fn render_rename(template: &str, stem: &str, value: &str) -> String {
    let out = template.replace("{stem}", stem).replace("{value}", value);
    if out.starts_with("minecraft:") {
        out
    } else {
        format!("minecraft:{out}")
    }
}

/// Insert one mapped state into the tables, tracking reverse collisions.
/// `java_name` keys the forward entry; `out_name` may differ from the
/// pattern's base name when a value renamed the output.
fn insert_one(
    java_name: &str,
    java_state: &BTreeMap<String, String>,
    out_name: &str,
    out_states: &BTreeMap<String, String>,
    state_rules: &mut BTreeMap<String, BlockRule>,
    reverse_seen: &mut BTreeMap<String, String>,
    stats: &mut AssemblyStats,
) {
    let java_key = state_key(java_name, java_state);
    let bedrock_key = state_key(out_name, out_states);
    if let Some(previous) = reverse_seen.get(&bedrock_key) {
        stats.reverse_collisions.push((previous.clone(), java_key.clone()));
    } else {
        reverse_seen.insert(bedrock_key.clone(), java_key.clone());
    }
    state_rules.insert(java_key, BlockRule {
        name: out_name.to_string(),
        states: out_states.clone(),
    });
    stats.states_mapped += 1;
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::handmaps::BlockMap;
    use crate::model::BedrockBlock;

    fn java(name: &str, props: &[(&str, &[&str])], states: &[&[(&str, &str)]]) -> JavaBlock {
        JavaBlock {
            name: name.into(),
            props: props
                .iter()
                .map(|(p, vs)| (p.to_string(), vs.iter().map(|v| v.to_string()).collect()))
                .collect(),
            states: states
                .iter()
                .map(|s| s.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
                .collect(),
        }
    }

    fn bedrock(name: &str, states: &[&[(&str, &str)]]) -> BedrockBlock {
        BedrockBlock {
            name: name.into(),
            states: states
                .iter()
                .map(|s| s.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
                .collect(),
        }
    }

    #[test]
    fn auto_join_maps_identity_with_bool_canonicalization() {
        let mut java_blocks = BTreeMap::new();
        java_blocks.insert(
            "minecraft:barrel".into(),
            java(
                "minecraft:barrel",
                &[("facing", &["north", "up"]), ("open", &["true", "false"])],
                &[
                    &[("facing", "north"), ("open", "true")],
                    &[("facing", "up"), ("open", "false")],
                ],
            ),
        );
        let mut bedrock_blocks = BTreeMap::new();
        bedrock_blocks.insert(
            "minecraft:barrel".into(),
            bedrock(
                "minecraft:barrel",
                &[
                    &[("facing_direction", "2"), ("open_bit", "1b")],
                    &[("facing_direction", "1"), ("open_bit", "0b")],
                ],
            ),
        );
        // facing north≠bedrock 2 — identity fallthrough fails; use a
        // matching value set instead for the pure-bool case.
        bedrock_blocks.get_mut("minecraft:barrel").unwrap().states = vec![
            [("facing".to_string(), "north".to_string()), ("open".to_string(), "1b".to_string())]
                .into_iter()
                .collect(),
            [("facing".to_string(), "up".to_string()), ("open".to_string(), "0b".to_string())]
                .into_iter()
                .collect(),
        ];
        let hand = HandMaps::default();
        let (rules, stats) = assemble(
            &java_blocks,
            &bedrock_blocks,
            &hand,
            BTreeMap::new(),
            BTreeMap::new(),
            &RegistryRules::seed(),
        );
        assert_eq!(stats.gaps.len(), 0, "{:?}", stats.gaps);
        assert_eq!(stats.states_mapped, 2);
        let key = "minecraft:barrel[facing=north,open=true]";
        let rule = &rules.state_rules[key];
        assert_eq!(rule.states["open"], "1b", "java true canonicalized to bedrock byte");
        assert!(rule.states["facing"] == "north");
    }

    #[test]
    fn hand_pattern_maps_stairs_and_validates() {
        let mut java_blocks = BTreeMap::new();
        java_blocks.insert(
            "minecraft:oak_stairs".into(),
            java(
                "minecraft:oak_stairs",
                &[("facing", &["north", "east"]), ("half", &["bottom", "top"]), ("shape", &["straight"])],
                &[
                    &[("facing", "north"), ("half", "bottom"), ("shape", "straight")],
                    &[("facing", "east"), ("half", "top"), ("shape", "straight")],
                ],
            ),
        );
        let mut bedrock_blocks = BTreeMap::new();
        bedrock_blocks.insert(
            "minecraft:oak_stairs".into(),
            bedrock(
                "minecraft:oak_stairs",
                &[
                    &[("weirdo_direction", "3"), ("upside_down_bit", "0b"), ("minecraft:corner", "none")],
                    &[("weirdo_direction", "0"), ("upside_down_bit", "1b"), ("minecraft:corner", "none")],
                ],
            ),
        );
        let mut hand = HandMaps::default();
        hand.patterns.insert(
            "stairs".into(),
            Pattern {
                props: [
                    (
                        "facing".to_string(),
                        [
                            ("north".to_string(), PropTarget::State("weirdo_direction".into(), "3".into())),
                            ("east".to_string(), PropTarget::State("weirdo_direction".into(), "0".into())),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    (
                        "half".to_string(),
                        [
                            ("bottom".to_string(), PropTarget::State("upside_down_bit".into(), "0b".into())),
                            ("top".to_string(), PropTarget::State("upside_down_bit".into(), "1b".into())),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    (
                        "shape".to_string(),
                        [(
                            "straight".to_string(),
                            PropTarget::State("minecraft:corner".into(), "none".into()),
                        )]
                        .into_iter()
                        .collect(),
                    ),
                ]
                .into_iter()
                .collect(),
                ..Default::default()
            },
        );
        hand.blocks.insert(
            "minecraft:oak_stairs".into(),
            BlockMap { name: "minecraft:oak_stairs".into(), pattern: Some("stairs".into()) },
        );
        let (rules, stats) = assemble(
            &java_blocks,
            &bedrock_blocks,
            &hand,
            BTreeMap::new(),
            BTreeMap::new(),
            &RegistryRules::seed(),
        );
        assert_eq!(stats.gaps.len(), 0, "{:?}", stats.gaps);
        assert_eq!(stats.states_mapped, 2);
        let rule = &rules.state_rules["minecraft:oak_stairs[facing=east,half=top,shape=straight]"];
        assert_eq!(rule.states["weirdo_direction"], "0");
        assert_eq!(rule.states["upside_down_bit"], "1b");
    }

    #[test]
    fn rename_target_produces_static_rule() {
        let mut java_blocks = BTreeMap::new();
        java_blocks.insert(
            "minecraft:bricks".into(),
            java("minecraft:bricks", &[], &[&[]]),
        );
        let mut bedrock_blocks = BTreeMap::new();
        bedrock_blocks.insert(
            "minecraft:brick_block".into(),
            bedrock("minecraft:brick_block", &[&[]]),
        );
        let mut hand = HandMaps::default();
        hand.blocks.insert(
            "minecraft:bricks".into(),
            BlockMap { name: "minecraft:brick_block".into(), pattern: None },
        );
        let (rules, stats) = assemble(
            &java_blocks,
            &bedrock_blocks,
            &hand,
            BTreeMap::new(),
            BTreeMap::new(),
            &RegistryRules::seed(),
        );
        assert_eq!(stats.gaps, vec![]);
        assert_eq!(rules.block_rules["minecraft:bricks"].name, "minecraft:brick_block");
        assert_eq!(stats.static_rules, 1);
    }

    #[test]
    fn unjoined_block_becomes_reported_gap() {
        let mut java_blocks = BTreeMap::new();
        java_blocks.insert(
            "minecraft:mystery".into(),
            java("minecraft:mystery", &[("a", &["b"])], &[&[("a", "b")]]),
        );
        let bedrock_blocks = BTreeMap::new();
        let hand = HandMaps::default();
        let (_rules, stats) = assemble(
            &java_blocks,
            &bedrock_blocks,
            &hand,
            BTreeMap::new(),
            BTreeMap::new(),
            &RegistryRules::seed(),
        );
        assert_eq!(stats.blocks_mapped, 0);
        assert_eq!(stats.gaps.len(), 1);
        assert_eq!(stats.gaps[0].0, "minecraft:mystery");
    }
}
