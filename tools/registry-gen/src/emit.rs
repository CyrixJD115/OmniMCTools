//! Output writers: the versioned RON rule table and the assembly stats.
//!
//! Format choice (documented per PLAN §4 M3): the generated table lands as
//! a **versioned RON** committed at
//! `crates/omt-convert/data/registry.ron`, embedded into `omt-convert`
//! with `include_str!`. RON keeps the generated data human-reviewable and
//! diffable (a generated `.rs` constant would obscure review), matches the
//! workspace's `omt-versions/registry.ron` precedent, and the existing
//! `RegistryRules` serde shape loads it unchanged. A user-supplied
//! `--rules` RON can still override the embedded default at runtime.

use std::path::Path;

use crate::semantic::AssemblyStats;

/// RON header shown above the generated table (attribution + scope).
pub const TABLE_HEADER: &str = r#"// GENERATED FILE — tools/registry-gen output; do not edit by hand.
// Hand-authored semantic mappings live in tools/registry-gen/hand-maps.ron.
//
// Sources (2026-11-08 pins):
// - Java: Mojang client.jar 26.3 assets (blockstates; data only, never
//   game code) cross-verified against HiveGamesOSS/Chunker 26.3 registry
//   data (MIT — reused as data with this attribution note).
// - Bedrock: Chunker 1.26.50.0 block-state registry (the complete
//   registry; the 1.26.52 fixture worlds verified as a subset via
//   omt-bedrock/examples/paletteprobe.rs).
// - Value semantics: Chunker BedrockStateGroups/StateTypes reference
//   tables, probe-verified against the fixture worlds.
// Scope: blocks absent from this table keep the loss-ledger passthrough
// (omt-convert::rules::map_block) — nothing is guessed.
"#;

/// Write the rule table RON (header + sorted serialization).
pub fn write_table(path: &Path, rules: &omt_convert::RegistryRules) -> Result<(), String> {
    let body = ron::ser::to_string_pretty(rules, Default::default())
        .map_err(|e| format!("serializing rule table: {e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    std::fs::write(path, format!("{TABLE_HEADER}{body}\n"))
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Write the assembly stats as JSON (committed next to the table so the
/// ≥95% sanity test can assert the generated artifact's coverage).
pub fn write_stats(path: &Path, stats: &AssemblyStats) -> Result<(), String> {
    let gaps = stats
        .gaps
        .iter()
        .map(|(block, reason)| serde_json::json!({ "block": block, "reason": reason }))
        .collect::<Vec<_>>();
    let value = serde_json::json!({
        "java_blocks": stats.java_blocks,
        "java_states": stats.java_states,
        "blocks_mapped": stats.blocks_mapped,
        "states_mapped": stats.states_mapped,
        "static_rules": stats.static_rules,
        "state_rules": stats.state_rules,
        "gap_blocks": stats.gaps.iter().map(|(b, _)| b.clone()).collect::<Vec<_>>(),
        "reverse_collisions": stats.reverse_collisions,
        "warnings": stats.warnings.iter().collect::<Vec<_>>(),
        "gaps": gaps,
    });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|e| format!("serializing stats: {e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    std::fs::write(path, format!("{text}\n"))
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_round_trips_through_ron() {
        let rules = omt_convert::RegistryRules::seed();
        let dir = std::env::temp_dir().join(format!("omt-reg-emit-{}", std::process::id()));
        let path = dir.join("registry.ron");
        write_table(&path, &rules).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("// GENERATED FILE"));
        let back: omt_convert::RegistryRules = ron::from_str(&text).unwrap();
        assert_eq!(back, rules);
    }
}
