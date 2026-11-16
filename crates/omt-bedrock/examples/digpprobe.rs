//! Probe: replicate the fixture_prune actor→column map on the source world
//! and check the specific actor id from the test failure.

use std::collections::HashMap;
use std::path::PathBuf;

use omt_bedrock::keys::{self, DbKey};

fn main() {
    let root = PathBuf::from("/home/cyrixjd115/Dev/Dev/Reverse_Engineering/UMT_Crack/test_worlds/bedrock-lostsurfur");
    let w = omt_bedrock::BedrockWorld::open_read_only(&root).unwrap();
    let before: std::collections::BTreeMap<Vec<u8>, Vec<u8>> = w.iter_raw().unwrap().into_iter().collect();

    let mut actors_to_column: HashMap<Vec<u8>, (u8, i32, i32)> = HashMap::new();
    for (k, v) in &before {
        if let DbKey::Digp { dim, x, z } = keys::decode_key(k) {
            for id in v.as_chunks::<8>().0 {
                actors_to_column.insert(keys::actor_key(*id), (dim, x, z));
            }
        }
    }
    println!("map entries: {}", actors_to_column.len());

    let needle = keys::actor_key([0, 0, 0, 2, 0, 0, 0, 5]);
    match actors_to_column.get(&needle) {
        Some(col) => println!("actor → {col:?}"),
        None => {
            println!("actor NOT in map; scanning raw digp values for the id bytes…");
            let id_bytes = [0u8, 0, 0, 2, 0, 0, 0, 5];
            for (k, v) in &before {
                if k.starts_with(b"digp") && v.len() % 8 == 0 && v.chunks(8).any(|c| c == id_bytes) {
                    println!("  found in digp key len {} value_len {} decode {:?}", k.len(), v.len(), keys::decode_key(k));
                }
            }
        }
    }
    // Count digps that decode to non-Digp (would be invisible to the map).
    let mut odd = 0;
    for k in before.keys() {
        if k.starts_with(b"digp") && !matches!(keys::decode_key(k), DbKey::Digp { .. }) {
            odd += 1;
            if odd <= 3 {
                println!("odd digp: {:02x?} → {:?}", k, keys::decode_key(k));
            }
        }
    }
    println!("digps failing Digp decode: {odd}");
}
