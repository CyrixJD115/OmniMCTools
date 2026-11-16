//! Probe: compare raw LevelDB chunk records against `iter_chunks`/`read_chunk`
//! for a world — finds enumeration gaps and per-chunk decode failures, and
//! dumps per-chunk surface block census for render cross-checks.

use std::collections::BTreeSet;
use std::path::PathBuf;

use omt_bedrock::keys::{self, DbKey};
use omt_world::WorldStore;

fn main() {
    let db = PathBuf::from(std::env::args().nth(1).expect("db path"));
    let world = omt_bedrock::BedrockWorld::open_read_only(&db).expect("open");
    let index = world.key_index().expect("key index");

    // Ground truth: decode every raw key with the key codec.
    let mut truth: BTreeSet<(u8, i32, i32)> = BTreeSet::new();
    let mut version_keys = 0usize;
    let mut subchunk_keys = 0usize;
    for (k, _) in index.iter() {
        if let DbKey::ChunkRecord { dim, x, z, tag } = keys::decode_key(k) {
            if tag == keys::tag::VERSION {
                version_keys += 1;
                truth.insert((dim, x, z));
            }
            if (0x2F..=0x3F).contains(&tag) {
                subchunk_keys += 1;
            }
        }
    }
    println!("raw DB: {} chunk-version keys, {} subchunk records", version_keys, subchunk_keys);
    let ow: Vec<(u8, i32, i32)> = truth.iter().copied().filter(|(d, _, _)| *d == 0).collect();
    println!("raw overworld columns (version keys): {}", ow.len());

    // Store view.
    let listed: Vec<(i32, i32)> = world.iter_chunks(0).expect("iter").collect();
    println!("iter_chunks(0): {} listed", listed.len());
    let missing: Vec<_> = ow.iter().filter(|(_, x, z)| !listed.contains(&(*x, *z))).collect();
    println!("listed missing vs raw: {missing:?}");

    for &(_, x, z) in &ow {
        match world.read_chunk(0, x, z) {
            Ok(Some(c)) => {
                let slices: Vec<_> = c.slices.keys().copied().collect();
                let blocks: usize = c.slices.values().map(|s| s.indices.len()).sum();
                println!("chunk {x}:{z}: {} slices {:?}, {} block refs", slices.len(), slices, blocks);
            }
            Ok(None) => println!("chunk {x}:{z}: read_chunk → None (!)"),
            Err(e) => println!("chunk {x}:{z}: read_chunk ERROR {e}"),
        }
    }
}
