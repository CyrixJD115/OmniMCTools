//! Probe: census of block-entity ids in a (copied!) world + one sample each.
//! `cargo run -p omt-convert --example beprobe --release -- <world> [dim]`
use omt_world::WorldStore;
fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("world"));
    let dim: u8 = std::env::args().nth(2).map(|d| d.parse().unwrap()).unwrap_or(0);
    let store: Box<dyn WorldStore> = match omt_world::detect(&root).expect("detect") {
        omt_world::Detected::JavaFolder { .. } => Box::new(omt_java::JavaWorld::open(&root).unwrap()),
        _ => Box::new(omt_bedrock::BedrockWorld::open_read_only(&root).unwrap()),
    };
    let mut counts: std::collections::BTreeMap<String, (usize, String)> = Default::default();
    for (x, z) in store.iter_chunks(dim).unwrap().collect::<Vec<_>>() {
        let Ok(Some(c)) = store.read_chunk(dim, x, z) else { continue };
        for be in &c.block_entities {
            let id = be.get("id").and_then(|t| t.as_str()).unwrap_or("?").to_string();
            let e = counts.entry(id).or_insert((0, String::new()));
            e.0 += 1;
            if e.1.is_empty() || (e.1.len() < 200 && omt_nbt::snbt::to_snbt(be).len() > e.1.len()) {
                let s = omt_nbt::snbt::to_snbt(be);
                e.1 = s.chars().take(700).collect();
            }
        }
    }
    for (id, (n, sample)) in counts {
        println!("{n:>6}  {id}\n        {sample}");
    }
}
