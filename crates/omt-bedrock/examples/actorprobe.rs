//! Empirical probe: Bedrock actor NBT shape (M3b reverse-conversion ground
//! truth). Walks `digp` → `actorprefix` entries in a fixture world and
//! prints the top-level keys of the first few actors, plus whether they
//! carry `identifier` (Bedrock naming) / `id` (Java naming) / `UUID`
//! (IntArray of 4) / `UniqueID` (Long). Fixture-gated via a path argument;
//! read-only.
//!
//! Run: cargo run -p omt-bedrock --example actorprobe <world-dir>

use bedrock_leveldb::{Db, OpenOptions};
use std::path::PathBuf;

fn main() {
    let world = PathBuf::from(std::env::args().nth(1).expect("usage: actorprobe <world-dir>"));
    let db = Db::open(world.join("db"), OpenOptions { read_only: true, ..Default::default() })
        .expect("open db");
    let snapshot = db.snapshot().expect("snapshot");

    let mut actor_keys = Vec::new();
    let mut digp_count = 0usize;
    for (k, _) in snapshot.iter() {
        if k.starts_with(b"actorprefix") {
            actor_keys.push(k.to_vec());
        } else if k.starts_with(b"digp") {
            digp_count += 1;
        }
    }
    println!("digp records: {digp_count}, actorprefix records: {}", actor_keys.len());

    let mut shown = 0usize;
    let mut key_shapes: std::collections::BTreeMap<String, usize> = Default::default();
    for key in actor_keys {
        let Some(v) = snapshot.get(&key) else { continue };
        let Ok(nbt) = omt_nbt::read(&v, omt_nbt::ByteOrder::Little) else {
            println!("  actor {}: NBT parse failed ({} bytes)", hex(&key[11..]), v.len());
            continue;
        };
        let mut shape = String::new();
        let omt_nbt::Tag::Compound(fields) = &nbt.tag else { continue };
        for (k, v) in fields {
            match (k.as_str(), v) {
                ("identifier", omt_nbt::Tag::String(s)) => shape.push_str(&format!("identifier={s} ")),
                ("id", omt_nbt::Tag::String(s)) => shape.push_str(&format!("id={s} ")),
                ("UUID", t) => shape.push_str(&format!("UUID({}) ", type_name(t))),
                ("UniqueID", t) => shape.push_str(&format!("UniqueID({}) ", type_name(t))),
                _ => {}
            }
        }
        *key_shapes.entry(shape).or_default() += 1;
        if shown < 8 {
            shown += 1;
            let keys: Vec<&str> = match &nbt.tag {
                omt_nbt::Tag::Compound(fields) => {
                    fields.iter().map(|(k, _)| k.as_str()).take(24).collect()
                }
                _ => Vec::new(),
            };
            println!("  actor {}: keys: {}", hex(&key[11..]), keys.join(", "));
        }
    }
    println!("\nidentifier/UUID shapes:");
    for (shape, n) in &key_shapes {
        println!("  ×{n}: {shape}");
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn type_name(t: &omt_nbt::Tag) -> &'static str {
    use omt_nbt::Tag::*;
    match t {
        Byte(_) => "Byte",
        Short(_) => "Short",
        Int(_) => "Int",
        Long(_) => "Long",
        Float(_) => "Float",
        Double(_) => "Double",
        ByteArray(_) => "ByteArray",
        String(_) => "String",
        List(_) => "List",
        Compound(_) => "Compound",
        IntArray(_) => "IntArray",
        LongArray(_) => "LongArray",
    }
}
