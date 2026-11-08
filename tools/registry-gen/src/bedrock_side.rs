//! Bedrock registry input: Chunker's generated `bedrock/<ver>/block_states.json`
//! (MIT, HiveGamesOSS/Chunker — reused as *data* with attribution; see PLAN
//! §0 and the emitted table header). It carries every valid Bedrock block
//! state combo for the pinned version — a world fixture is only a sample,
//! this is the complete registry the reverse direction needs.
//!
//! Value types are `int`/`byte`/`string` only (verified across the 1.26.x
//! files); they are canonicalized through `omt_nbt::snbt::to_snbt` so the
//! table keys agree with `omt-bedrock`'s palette decoding.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::model::{canonical_state_value, BedrockBlock};

/// Read a Chunker `block_states.json`. Both layouts seen in the repo are
/// accepted: a top-level array (older files) or `{"blocks": [...]}`.
pub fn read_chunker_bedrock(path: &Path) -> Result<BTreeMap<String, BedrockBlock>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("reading {}: {e}", path.display()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("parsing {}: {e}", path.display()))?;
    let entries = match &value {
        Value::Array(items) => items.clone(),
        Value::Object(map) => map
            .get("blocks")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| format!("{}: no top-level array nor `blocks`", path.display()))?,
        _ => return Err(format!("{}: not a JSON array or object", path.display())),
    };

    let mut out: BTreeMap<String, BedrockBlock> = BTreeMap::new();
    for entry in entries {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{}: entry without name", path.display()))?
            .to_string();
        let mut state: BTreeMap<String, String> = BTreeMap::new();
        for s in entry.get("states").and_then(Value::as_array).unwrap_or(&vec![]) {
            let key = s
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{name}: state without name"))?;
            let ty = s
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{name}.{key}: state without type"))?;
            let raw = s.get("value").ok_or_else(|| format!("{name}.{key}: state without value"))?;
            let value = canonical_state_value(ty, raw)
                .map_err(|e| format!("{name}.{key}: {e}"))?;
            // Duplicate keys inside one state combo are malformed data.
            if state.insert(key.to_string(), value).is_some() {
                return Err(format!("{name}: duplicate state key {key}"));
            }
        }
        out.entry(name.clone()).or_insert_with(|| BedrockBlock { name: name.clone(), states: Vec::new() })
            .states
            .push(state);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(contents: &str) -> std::path::PathBuf {
        // Unique dir per call: tests run in parallel in one process, and a
        // shared file made create-truncate race with reads (empty-parse EOF).
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "omt-reg-bedrock-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("block_states.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    #[test]
    fn parses_typed_states_into_canonical_forms() {
        let path = tmp(
            r#"{"blocks": [
                {"name": "minecraft:oak_stairs", "states": [
                    {"name": "weirdo_direction", "type": "int", "value": 3},
                    {"name": "upside_down_bit", "type": "byte", "value": 0},
                    {"name": "minecraft:corner", "type": "string", "value": "inner_left"}
                ]},
                {"name": "minecraft:oak_stairs", "states": [
                    {"name": "weirdo_direction", "type": "int", "value": 0},
                    {"name": "upside_down_bit", "type": "byte", "value": 1},
                    {"name": "minecraft:corner", "type": "string", "value": "none"}
                ]},
                {"name": "minecraft:stone", "states": []}
            ]}"#,
        );
        let blocks = read_chunker_bedrock(&path).unwrap();
        let stairs = &blocks["minecraft:oak_stairs"];
        assert_eq!(stairs.states.len(), 2);
        assert!(stairs.has_state(
            &[
                ("minecraft:corner".to_string(), "inner_left".to_string()),
                ("upside_down_bit".to_string(), "0b".to_string()),
                ("weirdo_direction".to_string(), "3".to_string()),
            ]
            .into_iter()
            .collect()
        ));
        let stone = &blocks["minecraft:stone"];
        assert!(stone.states.iter().all(|s| s.is_empty()));
    }

    #[test]
    fn accepts_top_level_array_layout() {
        let path = tmp(r#"[{"name": "minecraft:air", "states": []}]"#);
        let blocks = read_chunker_bedrock(&path).unwrap();
        assert!(blocks.contains_key("minecraft:air"));
    }

    #[test]
    fn rejects_unsupported_value_types() {
        let path = tmp(
            r#"[{"name": "minecraft:x", "states": [{"name": "weird", "type": "double", "value": 0.5}]}]"#,
        );
        assert!(read_chunker_bedrock(&path).is_err());
    }
}
