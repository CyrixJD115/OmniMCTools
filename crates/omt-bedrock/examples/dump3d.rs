use bedrock_leveldb::{Db, OpenOptions};
use omt_bedrock::keys;
fn main() {
    let world = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let db = Db::open(world.join("db"), OpenOptions { read_only: true, ..Default::default() }).unwrap();
    let snap = db.snapshot().unwrap();
    let mut want: Vec<usize> = vec![576, 568, 1616];
    for (k, v) in snap.iter() {
        if let omt_bedrock::DbKey::ChunkRecord { dim: 0, tag: 0x2b, .. } = keys::decode_key(&k) {
            if let Some(pos) = want.iter().position(|&w| w == v.len()) {
                want.remove(pos);
                let section = &v[512..];
                println!("=== len {} section {} bytes:", v.len(), section.len());
                for (i, chunk) in section.chunks(16).enumerate() {
                    println!("  +{:03}: {:02x?}", i * 16, chunk);
                }
            }
            if want.is_empty() { break; }
        }
    }
}
