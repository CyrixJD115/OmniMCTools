use bedrock_leveldb::{Db, OpenOptions};
use omt_bedrock::keys;
fn main() {
    let world = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let args: Vec<i32> = std::env::args().skip(2).map(|a| a.parse().unwrap()).collect();
    let (x, z, y) = (args[0], args[1], args[2] as i8);
    let db = Db::open(world.join("db"), OpenOptions { read_only: true, ..Default::default() }).unwrap();
    let key = keys::subchunk_key(x, z, 0, y);
    let v = db.get(&key).unwrap().expect("subchunk present");
    println!("subchunk ({x},{z}) y={y}: {} bytes", v.len());
    println!("hdr: {:02x?}", &v[..8.min(v.len())]);
    let (ver, count, yb) = (v[0], v[1], v[2]);
    println!("version={ver} storages={count} y={yb}");
    let mut pos = 3usize;
    for s in 0..count {
        let h = v[pos]; let bits = h >> 1;
        let cont = 4096 * bits as usize / 8;
        let aligned = if bits == 0 {0} else { 4096usize.div_ceil(32 / bits as usize) * 4 };
        println!("storage {s}: header={h:#04x} bits={bits} continuous={cont} aligned={aligned}");
        pos += 1;
        // try to locate the palette: after indices, a u32 count then NBT entries with 0x08 'name'
        pos += aligned;
    }
    println!("tail after aligned walk: pos={pos} of {}", v.len());
    println!("bytes around: {:02x?}", &v[pos.min(v.len().saturating_sub(16))..(pos+16).min(v.len())]);
}
