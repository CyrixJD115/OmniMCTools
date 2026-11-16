//! Probe: entity census of a (copied!) world, one sample per id.
//! `cargo run -p omt-convert --example entprobe --release -- <world>`
use omt_world::WorldStore;
fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("world"));
    let mut counts: std::collections::BTreeMap<String, (usize, String)> = Default::default();
    let mut note = |e: &omt_nbt::Tag| {
        let id = e.get("id").or(e.get("identifier")).and_then(|t| t.as_str()).unwrap_or("?").to_string();
        let s = omt_nbt::snbt::to_snbt(e);
        let entry = counts.entry(id).or_insert((0, String::new()));
        entry.0 += 1;
        if entry.1.is_empty() {
            entry.1 = s.chars().take(900).collect();
        }
    };
    match omt_world::detect(&root).expect("detect") {
        omt_world::Detected::JavaFolder { .. } => {
            let w = omt_java::JavaWorld::open(&root).unwrap();
            let dir = w.entity_dimension_dir(0).unwrap();
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                let Some((rx, rz)) = omt_java::RegionFile::coords_from_name(&p) else { continue };
                let bytes = std::fs::read(&p).unwrap();
                let Ok(r) = omt_java::RegionFile::from_bytes(&p, &bytes) else { continue };
                for (x, z) in r.chunk_coords(rx, rz) {
                    if let Ok(n) = r.read_chunk(&bytes, x, z) {
                        for ent in n.tag.get("Entities").and_then(|t| t.as_list()).unwrap_or(&[]) {
                            note(ent);
                        }
                    }
                }
            }
        }
        _ => {
            let w = omt_bedrock::BedrockWorld::open_read_only(&root).unwrap();
            for (x, z) in w.list_chunks(0).unwrap() {
                if let Ok(Some(c)) = w.read_chunk(0, x, z) {
                    for ent in &c.entities {
                        note(ent);
                    }
                }
            }
        }
    }
    for (id, (n, s)) in counts {
        println!("{n:>5} {id}\n      {s}");
    }
}
