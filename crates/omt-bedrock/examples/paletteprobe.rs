//! Empirical probe: what block names + state key/value forms does a real
//! 1.26.x world palette carry? Census over every subchunk palette entry of
//! every dimension: distinct names, per-name state keys with their value
//! type census, and a sample of typed (non-string) values per key.
//!
//! Ground truth for the registry generator's Bedrock side (PLAN §4 M3) and
//! for the SNBT canonical forms the rule table keys on.
//!
//! Usage: `cargo run -p omt-bedrock --example paletteprobe -- <world-dir>`

use bedrock_leveldb::{Db, OpenOptions};
use omt_bedrock::keys;
use omt_bedrock::subchunk::Subchunk;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// name -> (count, state key -> (occurrences, value-form -> count, sample))
type NameShapes =
    BTreeMap<String, (u64, BTreeMap<String, (u64, BTreeMap<String, u64>, String)>)>;

fn main() {
    let world = PathBuf::from(std::env::args().nth(1).expect("usage: paletteprobe <world-dir>"));
    let db = Db::open(world.join("db"), OpenOptions { read_only: true, ..Default::default() })
        .expect("open db");
    let snapshot = db.snapshot().expect("snapshot");

    // name -> (count, state key -> (occurrences, value-form -> count, sample))
    let mut names: NameShapes = BTreeMap::new();
    let mut entries = 0u64;
    let mut parse_errors = 0u64;

    for (k, _) in snapshot.iter() {
        let k = k.to_vec();
        if !matches!(keys::decode_key(&k), keys::DbKey::Subchunk { .. }) {
            continue;
        }
        let Some(v) = db.get(&k).expect("get") else { continue };
        let Ok(sub) = Subchunk::parse(&v, None) else {
            parse_errors += 1;
            continue;
        };
        for storage in &sub.storages {
            // The decoded model already carries name + SNBT-canonical states.
            for state in &storage.palette {
                entries += 1;
                let slot = names.entry(state.name.clone()).or_default();
                slot.0 += 1;
                for (pk, pv) in &state.properties {
                    let st = slot.1.entry(pk.clone()).or_default();
                    st.0 += 1;
                    *st.1.entry(value_form(pv)).or_insert(0) += 1;
                    st.2 = pv.clone();
                }
            }
        }
    }

    println!("palette entries: {entries}, subchunk parse errors: {parse_errors}");
    println!("distinct block names: {}", names.len());
    for (name, (count, states)) in &names {
        if !name.starts_with("minecraft:") {
            continue; // modded blocks are summarized separately
        }
        let keys: Vec<String> = states
            .iter()
            .map(|(k, (n, forms, sample))| {
                let fs: Vec<String> = forms.iter().map(|(f, c)| format!("{f}×{c}")).collect();
                format!("{k}({n};{};e.g. {sample})", fs.join(","))
            })
            .collect();
        println!("{name} ×{count}: {}", keys.join(" | "));
    }
    let modded: Vec<&String> = names.keys().filter(|n| !n.starts_with("minecraft:")).collect();
    println!("\nmodded ({} names): {:?}", modded.len(), modded.iter().take(30).collect::<Vec<_>>());
}

/// Classify the SNBT canonical form of a decoded state value.
fn value_form(v: &str) -> String {
    if v.ends_with('b') && v[..v.len() - 1].parse::<i8>().is_ok() {
        "byte".into()
    } else if v.ends_with('s') && v[..v.len() - 1].parse::<i16>().is_ok() {
        "short".into()
    } else if v.ends_with('L') && v[..v.len() - 1].parse::<i64>().is_ok() {
        "long".into()
    } else if v.ends_with('f') && v[..v.len() - 1].parse::<f64>().is_ok() {
        "float".into()
    } else if v.parse::<i64>().is_ok() {
        "int".into()
    } else {
        "string".into()
    }
}
