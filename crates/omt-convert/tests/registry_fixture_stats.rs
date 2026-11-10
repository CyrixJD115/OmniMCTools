//! Fixture-gated registry stats: run every distinct block state combo in
//! the real 1.26.52 Bedrock fixture world's palettes through the embedded
//! table (Bedrock → Java direction) and count matched/unmatched.
//!
//! The pre-registry baseline mapped **zero** stateful combos (stateful
//! blocks dropped their properties with a loss row); the gate asserts the
//! registry covers the overwhelming majority of what a real world carries.
//! Modded behavior-pack blocks (`panascais_caves:*`, `spark:*`) legitimately
//! stay unmapped — best-effort passthrough per PLAN §1.2 — so the gate is
//! set below 100%.
//!
//! Run with: `cargo test -p omt-convert --test registry_fixture_stats -- --ignored`
//! Fixture path: `OMT_BEDROCK_FIXTURE` env or `../../../../world_ref/e1j7l0I-l68=`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use omt_convert::RegistryRules;
use omt_world::WorldStore;

fn fixture_dir() -> PathBuf {
    std::env::var_os("OMT_BEDROCK_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../world_ref/e1j7l0I-l68=")
        })
}

#[test]
#[ignore = "requires the local 1.26.52 fixture world (world_ref/)"]
fn fixture_palettes_map_through_the_registry() {
    let fixture = fixture_dir();
    if !fixture.is_dir() {
        panic!("fixture world missing at {} — set OMT_BEDROCK_FIXTURE", fixture.display());
    }
    let world = omt_bedrock::BedrockWorld::open_read_only(&fixture).expect("open fixture");
    let rules = RegistryRules::embedded();

    // Distinct palette combos across the whole overworld (stateful only —
    // stateless names were already clean pre-registry, so the meaningful
    // metric is the stateful fraction). Vanilla namespace only: modded
    // behavior-pack blocks are best-effort passthrough by design (§1.2)
    // and are reported separately.
    let mut combos: BTreeSet<String> = BTreeSet::new();
    let mut modded: BTreeSet<String> = BTreeSet::new();
    let coords = world.list_chunks(0).expect("list chunks");
    let mut columns = 0usize;
    for &(x, z) in &coords {
        let Ok(Some(chunk)) = world.read_chunk(0, x, z) else { continue };
        columns += 1;
        for slice in chunk.slices.values() {
            for state in &slice.palette {
                if state.properties.is_empty() {
                    continue;
                }
                if state.name.starts_with("minecraft:") {
                    combos.insert(omt_convert::state_key(&state.name, &state.properties));
                } else {
                    modded.insert(omt_convert::state_key(&state.name, &state.properties));
                }
            }
        }
    }
    assert!(columns > 0, "fixture world carries no overworld columns");
    assert!(!combos.is_empty(), "fixture world carries no stateful vanilla palette entries");
    println!("modded (out of scope, passthrough): {} distinct combos", modded.len());

    // Map every distinct combo; a combo is matched when it converts
    // without an unmapped/dropped loss row.
    let mut matched = 0usize;
    let mut unmatched: Vec<String> = Vec::new();
    let reverse = rules.reverse();
    for key in &combos {
        let mut report = omt_convert::ConversionReport::new();
        let state = parse_combo(key);
        reverse.map_block(&state, &mut BTreeSet::new(), &mut report);
        if report.losses.iter().any(|l| {
            matches!(l.kind, omt_convert::LossKind::UnmappedId | omt_convert::LossKind::FieldDropped)
        }) {
            unmatched.push(key.clone());
        } else {
            matched += 1;
        }
    }
    let total = combos.len();
    let pct = 100.0 * matched as f64 / total as f64;
    println!("registry fixture stats: {matched}/{total} stateful combos matched ({pct:.1}%) over {columns} columns");
    let mut by_family: std::collections::BTreeMap<String, usize> = Default::default();
    for key in &unmatched {
        let stem = key.split('[').next().unwrap_or(key).to_string();
        *by_family.entry(stem).or_default() += 1;
    }
    let mut families: Vec<_> = by_family.into_iter().collect();
    families.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (stem, n) in families.iter().take(40) {
        println!("  unmatched {n:4} {stem}");
    }
    // Pre-registry baseline: 0% of stateful combos mapped. Gate at 85% —
    // comfortably above vanilla coverage while tolerating the fixture's
    // modded behavior-pack content.
    assert!(pct >= 85.0, "stateful combo coverage {pct:.1}% below the 85% gate");
}

/// Parse a `name[k=v,…]` combo back into a BlockState (the inverse of
/// `state_key`; the test's copy keeps the runtime function lossless for
/// the converter while the fixture census spells combos itself).
fn parse_combo(key: &str) -> omt_world::BlockState {
    let (name, rest) = match key.split_once('[') {
        Some((n, r)) => (n, r.trim_end_matches(']')),
        None => return omt_world::BlockState { name: key.to_string(), properties: vec![] },
    };
    let mut properties: Vec<(String, String)> =
        rest.split(',').filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
    properties.sort();
    omt_world::BlockState { name: name.to_string(), properties }
}
