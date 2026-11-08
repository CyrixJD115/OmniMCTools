//! Java registry input: the pinned client.jar's `assets/minecraft/blockstates/*.json`
//! (Mojang data — names, properties, variant coverage) plus Chunker's
//! generated `java/26.3/blocks.json` (MIT data reuse — complete valid state
//! lists; blockstate `when` conditions only enumerate a subset for
//! multipart blocks like fences, so the jar alone cannot give full value
//! sets). The two sources are cross-verified; disagreements are reported,
//! never silently merged.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

use serde_json::Value;

use crate::model::JavaBlock;

/// Blockstate JSON parsed from the jar: properties the variants/multipart
/// model JSONs visibly enumerate (complete for `variants` blocks, partial
/// for `multipart` blocks).
#[derive(Debug, Default, Clone)]
pub struct JarBlockstate {
    pub variants_props: BTreeMap<String, BTreeSet<String>>,
    pub multipart_props: BTreeMap<String, BTreeSet<String>>,
}

/// Read every `assets/minecraft/blockstates/*.json` from a client.jar.
/// Item-only names carried as blockstates (`item_frame`, `glow_item_frame`)
/// are included and flagged by the verifier; real extraction of the item
/// registry is M4.
pub fn read_jar_blockstates(
    jar_path: &Path,
) -> Result<BTreeMap<String, JarBlockstate>, String> {
    let file = std::fs::File::open(jar_path)
        .map_err(|e| format!("opening {}: {e}", jar_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("reading jar {}: {e}", jar_path.display()))?;

    let mut out = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("jar entry {i}: {e}"))?;
        let name = entry.name().to_string();
        let Some(base) = name
            .strip_prefix("assets/minecraft/blockstates/")
            .filter(|n| n.ends_with(".json"))
        else {
            continue;
        };
        let block = format!("minecraft:{}", base.trim_end_matches(".json"));
        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .map_err(|e| format!("reading {name}: {e}"))?;
        let value: Value =
            serde_json::from_str(&text).map_err(|e| format!("parsing {name}: {e}"))?;
        out.insert(block, parse_blockstate_json(&value)?);
    }
    Ok(out)
}

/// Parse one blockstate JSON body: `variants` keys (`a=b,c=d` or empty) and
/// `multipart` `when` conditions (flat maps, `{OR|AND: [...]}` trees, or
/// arrays; values may be `a|b` alternatives).
fn parse_blockstate_json(value: &Value) -> Result<JarBlockstate, String> {
    let mut out = JarBlockstate::default();
    if let Some(variants) = value.get("variants").and_then(Value::as_object) {
        for key in variants.keys() {
            for (prop, val) in parse_variant_key(key)? {
                out.variants_props.entry(prop).or_default().insert(val);
            }
        }
    }
    if let Some(parts) = value.get("multipart").and_then(Value::as_array) {
        for part in parts {
            collect_when(part.get("when"), &mut out.multipart_props)?;
        }
    }
    Ok(out)
}

/// `"a=b,c=d"` (or `""` for the propertyless variant) → property→value pairs.
fn parse_variant_key(key: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    if key.is_empty() {
        return Ok(out);
    }
    for part in key.split(',') {
        let Some((prop, val)) = part.split_once('=') else {
            return Err(format!("malformed variant key {key:?}"));
        };
        out.push((prop.trim().to_string(), val.trim().to_string()));
    }
    Ok(out)
}

/// Walk a `when` clause, collecting every property it mentions.
fn collect_when(
    when: Option<&Value>,
    into: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    let Some(when) = when else { return Ok(()) };
    if let Some(list) = when.as_array() {
        for item in list {
            collect_when(Some(item), into)?;
        }
        return Ok(());
    }
    let Some(map) = when.as_object() else {
        return Err("multipart `when` is neither object nor array".into());
    };
    for (key, val) in map {
        if key == "OR" || key == "AND" {
            collect_when(Some(val), into)?;
            continue;
        }
        let raw = val.as_str().ok_or_else(|| format!("when {key:?} not a string"))?;
        for alt in raw.split('|') {
            into.entry(key.clone()).or_default().insert(alt.trim().to_string());
        }
    }
    Ok(())
}

/// Read Chunker's `java/<ver>/blocks.json` into full [`JavaBlock`]s.
pub fn read_chunker_java(path: &Path) -> Result<BTreeMap<String, JavaBlock>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("reading {}: {e}", path.display()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("parsing {}: {e}", path.display()))?;
    let map = value
        .as_object()
        .ok_or_else(|| format!("{}: not a JSON object", path.display()))?;

    let mut out = BTreeMap::new();
    for (name, entry) in map {
        let mut block = JavaBlock { name: name.clone(), ..Default::default() };
        if let Some(props) = entry.get("properties").and_then(Value::as_object) {
            for (prop, values) in props {
                let list = values
                    .as_array()
                    .ok_or_else(|| format!("{name}.{prop}: values not a list"))?;
                let mut set = BTreeSet::new();
                for v in list {
                    set.insert(
                        v.as_str()
                            .ok_or_else(|| format!("{name}.{prop}: value not a string"))?
                            .to_string(),
                    );
                }
                block.props.insert(prop.clone(), set);
            }
        }
        for state in entry.get("states").and_then(Value::as_array).unwrap_or(&vec![]) {
            // `properties` is absent for propertyless states — treated as
            // an empty combo, not an error.
            let empty = serde_json::Map::new();
            let props = state.get("properties").and_then(Value::as_object).unwrap_or(&empty);
            let mut combo = BTreeMap::new();
            for (k, v) in props {
                combo.insert(
                    k.clone(),
                    v.as_str()
                        .ok_or_else(|| format!("{name}.{k}: state value not a string"))?
                        .to_string(),
                );
            }
            block.states.push(combo);
        }
        out.insert(name.clone(), block);
    }
    Ok(out)
}

/// One jar ↔ Chunker agreement finding (a bug or a documented difference —
/// reported to the operator, never silently merged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    /// Jar blockstate with no Chunker registry entry (known: the two
    /// `item_frame` items).
    JarOnly(String),
    /// Chunker block with no jar blockstate (known: blocks without an
    /// assets entry — not user-placeable in the mapped set).
    ChunkerOnly(String),
    /// Jar enumerates a property Chunker's registry lacks — a version
    /// mismatch that would produce wrong mappings (hard error at the
    /// caller's discretion).
    PropMissingInChunker { block: String, prop: String },
    /// Jar variant value outside Chunker's value set.
    ValueMissingInChunker { block: String, prop: String, value: String },
}

/// Cross-verify jar blockstates against the Chunker registry.
pub fn verify_jar_vs_chunker(
    jar: &BTreeMap<String, JarBlockstate>,
    chunker: &BTreeMap<String, JavaBlock>,
) -> Vec<Verification> {
    let mut out = Vec::new();
    for (name, state) in jar {
        let Some(block) = chunker.get(name) else {
            out.push(Verification::JarOnly(name.clone()));
            continue;
        };
        let visible = state.variants_props.iter().chain(state.multipart_props.iter());
        for (prop, values) in visible {
            let Some(allowed) = block.props.get(prop) else {
                out.push(Verification::PropMissingInChunker {
                    block: name.clone(),
                    prop: prop.clone(),
                });
                continue;
            };
            for value in values {
                if !allowed.contains(value) {
                    out.push(Verification::ValueMissingInChunker {
                        block: name.clone(),
                        prop: prop.clone(),
                        value: value.clone(),
                    });
                }
            }
        }
    }
    for name in chunker.keys() {
        if !jar.contains_key(name) {
            out.push(Verification::ChunkerOnly(name.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_jar(entries: &[(&str, &str)]) -> std::path::PathBuf {
        let path = std::env::temp_dir()
            .join(format!("omt-reg-java-{}.jar", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions = Default::default();
        for (name, body) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn parses_variants_and_multipart_keys() {
        let path = tmp_jar(&[
            (
                "assets/minecraft/blockstates/oak_stairs.json",
                r#"{"variants": {
                    "facing=east,half=bottom,shape=straight": {"model": "a"},
                    "facing=east,half=bottom,shape=inner_left": {"model": "b"},
                    "": {"model": "c"}
                }}"#,
            ),
            (
                "assets/minecraft/blockstates/oak_fence.json",
                r#"{"multipart": [
                    {"apply": {"model": "post"}},
                    {"apply": {"model": "side"}, "when": {"north": "true"}},
                    {"apply": {"model": "tall"}, "when": {"AND": [
                        {"west": "low|tall"}, {"east": "none"}
                    ]}},
                    {"apply": {"model": "or"}, "when": [{"south": "side"}, {"west": "side"}]}
                ]}"#,
            ),
        ]);
        let jar = read_jar_blockstates(&path).unwrap();
        let stairs = &jar["minecraft:oak_stairs"];
        assert_eq!(
            stairs.variants_props.get("shape").map(|s| s.len()),
            Some(2),
            "variant props fully enumerated"
        );
        assert!(stairs.variants_props.is_empty() || !stairs.variants_props.contains_key(""),
            "empty variant key carries no property");
        let fence = &jar["minecraft:oak_fence"];
        // Multipart conditions are partial: `north` shows only "true".
        assert_eq!(
            fence.multipart_props.get("north").map(|s| s.iter().map(String::as_str).collect::<Vec<_>>()),
            Some(vec!["true"])
        );
        assert_eq!(
            fence.multipart_props.get("west").map(|s| s.iter().map(String::as_str).collect::<Vec<_>>()),
            // Values are the union across ALL clauses: the AND clause brings
            // `low|tall`, and the OR array contributes its `west: "side"`.
            Some(vec!["low", "side", "tall"])
        );
        assert!(fence.variants_props.is_empty());
    }

    #[test]
    fn parses_chunker_java_registry() {
        let dir = std::env::temp_dir().join(format!("omt-reg-java-ch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blocks.json");
        std::fs::write(
            &path,
            r#"{
              "minecraft:oak_stairs": {
                "properties": {"facing": ["north","east"], "half": ["bottom","top"]},
                "states": [
                  {"id": 1, "properties": {"facing": "north", "half": "bottom"}},
                  {"id": 2, "properties": {"facing": "east", "half": "top"}}
                ]
              },
              "minecraft:stone": {"states": [{"id": 3, "properties": {}}]}
            }"#,
        )
        .unwrap();
        let blocks = read_chunker_java(&path).unwrap();
        let stairs = &blocks["minecraft:oak_stairs"];
        assert_eq!(stairs.props["facing"].len(), 2);
        assert_eq!(stairs.states.len(), 2);
        assert_eq!(stairs.states[0]["half"], "bottom");
        assert!(blocks["minecraft:stone"].props.is_empty());
    }

    #[test]
    fn verification_flags_missing_and_mismatched() {
        let mut jar = BTreeMap::new();
        jar.insert(
            "minecraft:a".to_string(),
            JarBlockstate {
                variants_props: [("face".to_string(), BTreeSet::from(["floor".to_string()]))]
                    .into_iter()
                    .collect(),
                multipart_props: BTreeMap::new(),
            },
        );
        jar.insert("minecraft:only_jar".to_string(), JarBlockstate::default());
        let mut chunker = BTreeMap::new();
        chunker.insert(
            "minecraft:a".to_string(),
            JavaBlock {
                name: "minecraft:a".into(),
                props: BTreeMap::new(),
                states: vec![BTreeMap::new()],
            },
        );
        chunker.insert("minecraft:only_chunker".to_string(), JavaBlock::default());
        let findings = verify_jar_vs_chunker(&jar, &chunker);
        assert!(findings.contains(&Verification::JarOnly("minecraft:only_jar".into())));
        assert!(findings.contains(&Verification::ChunkerOnly("minecraft:only_chunker".into())));
        assert!(findings.contains(&Verification::PropMissingInChunker {
            block: "minecraft:a".into(),
            prop: "face".into()
        }));
    }
}
