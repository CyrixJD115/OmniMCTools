//! Biome and enchantment id tables.
//!
//! **Biomes:** Chunker's resolver test resource carries the classic
//! Bedrock numeric biome ids (`bedrock/resolver/biome_ids.json`, stable
//! since 1.16 when per-block biomes shipped — `basalt_deltas=181` confirms
//! the modern set). Java names join by *short name* with the hand-map
//! `biome_aliases` bridging the naming eras data cannot
//! (`minecraft:snowy_plains` → `ice_plains`, `minecraft:nether_wastes` →
//! `hell`); numeric equality is never assumed across editions.
//!
//! **Enchantments:** Chunker's MIT resolvers give the complete
//! Java-name ↔ Bedrock-numeric table through `lunge` (41); the pinned
//! jar's `tags/enchantment/tooltip_order.json` (43 names) cross-verifies
//! the Java side. Java `sweeping_edge` has no Bedrock home — omitted here,
//! dropped with a loss row at runtime (never guessed).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

/// (java full name → bedrock numeric id, warnings). `aliases` maps a Java
/// short name to the Bedrock short name when they differ across naming
/// eras.
pub fn read_biomes(
    bedrock_ids: &Path,
    aliases: &BTreeMap<String, String>,
) -> Result<(BTreeMap<String, i32>, Vec<String>), String> {
    let bedrock: BTreeMap<String, i32> = read_string_int(bedrock_ids)?;

    let mut out = BTreeMap::new();
    let mut warnings = Vec::new();
    // Every Bedrock short name is directly reachable as `minecraft:<short>`.
    for (short, id) in &bedrock {
        out.insert(format!("minecraft:{short}"), *id);
    }
    // Aliased Java names (same id under the era-bridged Bedrock name).
    for (java_short, bedrock_short) in aliases {
        match bedrock.get(bedrock_short) {
            Some(id) => {
                out.insert(format!("minecraft:{java_short}"), *id);
            }
            None => warnings.push(format!(
                "biome alias minecraft:{java_short} → {bedrock_short}: target missing \
                 from the id table"
            )),
        }
    }
    if !out.contains_key("minecraft:ocean") {
        return Err("biome table lacks minecraft:ocean (id 0 fallback)".into());
    }
    Ok((out, warnings))
}

/// (java full name → bedrock numeric id, warnings). Chunker's Java and
/// Bedrock resolver id files are keyed by Chunker enum names; this module
/// carries the semantic join (enum → Java id string, enum → Bedrock id)
/// as data copied with attribution, cross-checked against the jar's
/// tooltip_order tag by the caller.
pub fn read_enchantments(jar_tooltip_order: Option<&[u8]>) -> Result<(BTreeMap<String, i16>, Vec<String>), String> {
    // (Chunker enum name, Java id, Bedrock numeric id) — Bedrock ids from
    // BedrockEnchantmentIDResolver, Java ids from JavaEnchantmentResolver
    // (HiveGamesOSS/Chunker, MIT). `sweeping_edge` has no Bedrock id.
    const TABLE: &[(&str, &str, Option<i16>)] = &[
        ("ALL_DAMAGE_PROTECTION", "minecraft:protection", Some(0)),
        ("FIRE_PROTECTION", "minecraft:fire_protection", Some(1)),
        ("FALL_PROTECTION", "minecraft:feather_falling", Some(2)),
        ("BLAST_PROTECTION", "minecraft:blast_protection", Some(3)),
        ("PROJECTILE_PROTECTION", "minecraft:projectile_protection", Some(4)),
        ("RESPIRATION", "minecraft:respiration", Some(6)),
        ("AQUA_AFFINITY", "minecraft:aqua_affinity", Some(8)),
        ("THORNS", "minecraft:thorns", Some(5)),
        ("DEPTH_STRIDER", "minecraft:depth_strider", Some(7)),
        ("SHARPNESS", "minecraft:sharpness", Some(9)),
        ("SMITE", "minecraft:smite", Some(10)),
        ("BANE_OF_ARTHROPODS", "minecraft:bane_of_arthropods", Some(11)),
        ("KNOCKBACK", "minecraft:knockback", Some(12)),
        ("FIRE_ASPECT", "minecraft:fire_aspect", Some(13)),
        ("MOB_LOOTING", "minecraft:looting", Some(14)),
        ("BLOCK_EFFICIENCY", "minecraft:efficiency", Some(15)),
        ("SILK_TOUCH", "minecraft:silk_touch", Some(16)),
        ("UNBREAKING", "minecraft:unbreaking", Some(17)),
        ("BLOCK_FORTUNE", "minecraft:fortune", Some(18)),
        ("POWER_ARROWS", "minecraft:power", Some(19)),
        ("PUNCH_ARROWS", "minecraft:punch", Some(20)),
        ("FLAMING_ARROWS", "minecraft:flame", Some(21)),
        ("INFINITY_ARROWS", "minecraft:infinity", Some(22)),
        ("FISHING_LUCK", "minecraft:luck_of_the_sea", Some(23)),
        ("FISHING_SPEED", "minecraft:lure", Some(24)),
        ("FROST_WALKER", "minecraft:frost_walker", Some(25)),
        ("MENDING", "minecraft:mending", Some(26)),
        ("BINDING_CURSE", "minecraft:binding_curse", Some(27)),
        ("VANISHING_CURSE", "minecraft:vanishing_curse", Some(28)),
        ("IMPALING", "minecraft:impaling", Some(29)),
        ("RIPTIDE", "minecraft:riptide", Some(30)),
        ("LOYALTY", "minecraft:loyalty", Some(31)),
        ("CHANNELING", "minecraft:channeling", Some(32)),
        ("MULTISHOT", "minecraft:multishot", Some(33)),
        ("PIERCING", "minecraft:piercing", Some(34)),
        ("QUICK_CHARGE", "minecraft:quick_charge", Some(35)),
        ("SOUL_SPEED", "minecraft:soul_speed", Some(36)),
        ("SWIFT_SNEAK", "minecraft:swift_sneak", Some(37)),
        // Java-only enchantment: carried as a documented `None` so the
        // warning (and the runtime loss row) name it explicitly.
        ("SWEEPING_EDGE", "minecraft:sweeping_edge", None),
        ("WIND_BURST", "minecraft:wind_burst", Some(38)),
        ("DENSITY", "minecraft:density", Some(39)),
        ("BREACH", "minecraft:breach", Some(40)),
        ("LUNGE", "minecraft:lunge", Some(41)),
    ];
    let mut out = BTreeMap::new();
    let mut warnings = Vec::new();
    for (enum_name, java_id, bedrock) in TABLE {
        match bedrock {
            Some(id) => {
                out.insert(java_id.to_string(), *id);
            }
            None => warnings.push(format!("{enum_name} ({java_id}): no Bedrock id")),
        }
    }
    // Cross-check against the pinned jar when available.
    if let Some(bytes) = jar_tooltip_order {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| format!("parsing tooltip_order.json: {e}"))?;
        let names: Vec<&str> = value
            .get("values")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        for name in &names {
            if !out.contains_key(*name) && *name != "minecraft:sweeping_edge" {
                warnings.push(format!(
                    "jar enchantment {name} absent from the mapping table"
                ));
            }
        }
        let unmapped_in_jar: Vec<&&str> =
            names.iter().filter(|n| !out.contains_key(**n)).collect();
        warnings.push(format!(
            "jar lists {} enchantments; {} without a Bedrock id (dropped with a loss row)",
            names.len(),
            unmapped_in_jar.len(),
        ));
    }
    Ok((out, warnings))
}

fn read_string_int(path: &Path) -> Result<BTreeMap<String, i32>, String> {
    let value: Value = parse_json(path)?;
    let map = value
        .as_object()
        .ok_or_else(|| format!("{}: not an object", path.display()))?;
    let mut out = BTreeMap::new();
    for (k, v) in map {
        out.insert(
            k.clone(),
            v.as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .ok_or_else(|| format!("{}.{k}: not an int", path.display()))?,
        );
    }
    Ok(out)
}

fn parse_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("reading {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("parsing {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enchantment_table_is_complete_and_grounded() {
        let (enchants, warnings) = read_enchantments(None).unwrap();
        assert_eq!(enchants.get("minecraft:sharpness"), Some(&9));
        assert_eq!(enchants.get("minecraft:protection"), Some(&0));
        assert_eq!(enchants.get("minecraft:lunge"), Some(&41));
        assert_eq!(enchants.get("minecraft:density"), Some(&39));
        // sweeping_edge documented as unmapped.
        assert!(warnings.iter().any(|w| w.contains("sweeping_edge") || w.contains("SWEEPING")));
        assert!(!enchants.contains_key("minecraft:sweeping_edge"));
    }

    #[test]
    fn biomes_join_aliases() {
        let dir = std::env::temp_dir().join(format!("omt-reg-biomes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bedrock = dir.join("bedrock.json");
        std::fs::write(&bedrock, r#"{"ocean": 0, "plains": 1, "ice_plains": 12}"#).unwrap();
        let mut aliases = BTreeMap::new();
        aliases.insert("snowy_plains".to_string(), "ice_plains".to_string());
        let (biomes, warnings) = read_biomes(&bedrock, &aliases).unwrap();
        assert_eq!(biomes.get("minecraft:plains"), Some(&1));
        assert_eq!(biomes.get("minecraft:snowy_plains"), Some(&12));
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
