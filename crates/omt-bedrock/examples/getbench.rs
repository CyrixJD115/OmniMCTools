//! Probe: point-read latency of a (copied!) Bedrock world's LevelDB.
//! `cargo run -p omt-bedrock --example getbench --release -- <world>`
fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("world"));
    let w = omt_bedrock::BedrockWorld::open_read_only(&root).expect("open");
    let t = std::time::Instant::now();
    let cols = w.list_chunks(0).expect("list");
    println!("list_chunks: {} in {:?}", cols.len(), t.elapsed());
    let sample: Vec<_> = cols.iter().step_by((cols.len() / 200).max(1)).take(200).collect();
    let t = std::time::Instant::now();
    let mut hits = 0;
    for &&(x, z) in &sample {
        if w.raw_get(&omt_bedrock::keys::version_key(x, z, 0)).expect("get").is_some() {
            hits += 1;
        }
    }
    let el = t.elapsed();
    println!("{} present gets ({hits} hits): {:?} total, {:?}/get", sample.len(), el, el / sample.len() as u32);
    let t = std::time::Instant::now();
    for &&(x, z) in &sample {
        let _ = w.raw_get(&omt_bedrock::keys::subchunk_key(x, z, 0, 18)).expect("get");
    }
    let el = t.elapsed();
    println!("{} probably-absent gets: {:?}/get", sample.len(), el / sample.len() as u32);
}
