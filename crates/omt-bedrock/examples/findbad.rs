use bedrock_leveldb::{Db, OpenOptions};
use omt_bedrock::keys;
fn main() {
    let world = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let db = Db::open(world.join("db"), OpenOptions { read_only: true, ..Default::default() }).unwrap();
    let snap = db.snapshot().unwrap();
    let mut bits_hist = std::collections::BTreeMap::new();
    let mut fails = 0;
    for (k, v) in snap.iter() {
        if let omt_bedrock::DbKey::Subchunk { x, z, dim: 0, y } = keys::decode_key(&k) {
            // header census
            let count = v[1];
            let mut pos = 3usize;
            for _ in 0..count {
                let h = v[pos];
                let bits = h >> 1;
                *bits_hist.entry(bits).or_insert(0) += 1;
                pos += 1 + omt_bedrock::subchunk::packed_len(bits);
                // skip palette properly by parsing entry roots
                if v.get(pos).is_some() {
                    // We can't skip without a real parser; stop census here for multi-storage.
                    break;
                }
            }
            if let Err(e) = omt_bedrock::Subchunk::parse(&v, Some(y)) {
                fails += 1;
                if fails <= 5 {
                    println!("FAIL ({x},{z}) y={y} len={}: {e}", v.len());
                    println!("  hdr: {:02x?}", &v[..12.min(v.len())]);
                }
            }
        }
    }
    println!("bits histogram: {:?}", bits_hist);
    println!("total parse failures: {fails}");
}
