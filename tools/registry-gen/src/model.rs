//! Shared registry model for the generator: one Java block (with its valid
//! property states) and one Bedrock block (with its valid state combos).
//!
//! Canonical value forms follow `omt_nbt::snbt::to_snbt` so that the rule
//! table keys agree byte-for-byte with what `omt-bedrock` decodes palette
//! NBT into: ints are bare (`3`), bytes are suffixed (`1b`), strings are
//! their raw text (never quoted). String state values that would parse as
//! an SNBT scalar are rejected by validation (see [`crate::semantic`]) —
//! the runtime writer would re-type them.

use std::collections::{BTreeMap, BTreeSet};

use omt_nbt::snbt::to_snbt;
use omt_nbt::Tag;

/// One Java registry block: property definitions plus every valid
/// property combination (Chunker's generated `java/<ver>/blocks.json`
/// carries both; the client.jar blockstates cross-verify).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JavaBlock {
    /// Fully qualified id (`minecraft:oak_stairs`).
    pub name: String,
    /// property → sorted value set.
    pub props: BTreeMap<String, BTreeSet<String>>,
    /// Every valid state as a property→value map (registry order kept by
    /// the caller for stable iteration).
    pub states: Vec<BTreeMap<String, String>>,
}

/// One Bedrock registry block: every valid state combo with canonical
/// SNBT-string values (decoded exactly like `omt-bedrock` palette entries).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BedrockBlock {
    pub name: String,
    pub states: Vec<BTreeMap<String, String>>,
}

impl BedrockBlock {
    /// Every distinct state key with its value set (for auto-join checks).
    pub fn props(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for state in &self.states {
            for (k, v) in state {
                out.entry(k.clone()).or_default().insert(v.clone());
            }
        }
        out
    }

    /// True when this exact name+state combo exists in the registry.
    pub fn has_state(&self, state: &BTreeMap<String, String>) -> bool {
        self.states.iter().any(|s| s == state)
    }
}

/// Canonical string for a Bedrock state value of the given Chunker type
/// (`int`/`byte`/`string`). Unknown types are a data-format error — typed,
/// never guessed.
pub fn canonical_state_value(ty: &str, raw: &serde_json::Value) -> Result<String, String> {
    let tag = match ty {
        "int" => Tag::Int(
            raw.as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .ok_or_else(|| format!("int state value not an i32: {raw}"))?,
        ),
        "byte" => Tag::Byte(
            raw.as_i64()
                .and_then(|v| i8::try_from(v).ok())
                .ok_or_else(|| format!("byte state value not an i8: {raw}"))?,
        ),
        "string" => return Ok(raw.as_str().ok_or("string state value not a string")?.to_string()),
        other => return Err(format!("unsupported bedrock state type {other:?}")),
    };
    Ok(to_snbt(&tag))
}

/// Serialize the canonical key for a name + property map:
/// `name[k=v,k=v]` with keys sorted. This is the exact form the runtime
/// rule table keys on for both directions.
pub fn state_key(name: &str, props: &BTreeMap<String, String>) -> String {
    if props.is_empty() {
        return name.to_string();
    }
    let inner: Vec<String> = props.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{name}[{}]", inner.join(","))
}

/// Serialize a `BlockState`'s properties (Vec pairs) into the same key form.
pub fn state_key_from_pairs(name: &str, props: &[(String, String)]) -> String {
    let sorted: BTreeMap<String, String> = props.iter().cloned().collect();
    state_key(name, &sorted)
}
