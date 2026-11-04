//! Empirical probe v2: shape analysis of a Bedrock fixture's LevelDB keys.

use bedrock_leveldb::{Db, OpenOptions};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn main() {
    let world = PathBuf::from(std::env::args().nth(1).expect("usage: probe <world-dir>"));
    let db = Db::open(world.join("db"), OpenOptions { read_only: true, ..Default::default() })
        .expect("open db");
    let snapshot = db.snapshot().expect("snapshot");

    // Shape = key length + tag hypothesis. For 10-byte keys show both
    // candidate positions (key[8] if y-last, key[9] if y-first).
    let mut shapes: BTreeMap<String, (usize, Vec<(Vec<u8>, Vec<u8>)>)> = BTreeMap::new();
    let mut total = 0usize;

    for (k, v) in snapshot.iter() {
        total += 1;
        let k = k.to_vec();
        let v = v.to_vec();
        let shape = if k.len() >= 4 && k.starts_with(b"digp") {
            format!("digp:{}", k.len())
        } else if k.starts_with(b"actorprefix") {
            format!("actor:{}", k.len())
        } else if k == b"~local_player" {
            "~local_player".into()
        } else if k.len() == 9 {
            format!("len9 tag={:#04x}", k[8])
        } else if k.len() == 10 {
            format!("len10 k8={:#04x} k9={:#04x}", k[8], k[9])
        } else {
            // printable → string key; else binary
            if k.iter().all(|b| b.is_ascii_graphic() || *b == b'_' || *b == b'~') {
                format!("str:{}", String::from_utf8_lossy(&k))
            } else {
                format!("bin:{}", k.len())
            }
        };
        let entry = shapes.entry(shape).or_insert((0, Vec::new()));
        entry.0 += 1;
        if entry.1.len() < 3 {
            entry.1.push((k, v));
        }
    }

    println!("total keys: {total}");
    for (shape, (count, samples)) in &shapes {
        println!("\n== {shape} × {count}");
        for (k, v) in samples {
            println!("  key : {k:02x?}");
            println!("  val({}B): {:02x?}", v.len(), &v[..v.len().min(72)]);
        }
    }
}
