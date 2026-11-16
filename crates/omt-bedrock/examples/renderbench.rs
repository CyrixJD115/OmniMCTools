//! Probe: GUI lazy-map timings — scan cost and one 32×32-chunk tile build.

use rayon::prelude::*;
use std::time::Instant;

fn main() {
    let root = std::path::PathBuf::from("/home/cyrixjd115/Dev/Dev/Reverse_Engineering/UMT_Crack/test_worlds/bedrock-lostsurfur");
    let world = omt_bedrock::BedrockWorld::open_read_only(&root).unwrap();
    let t = Instant::now();
    let snap = world.snapshot().unwrap();
    println!("snapshot (one-time): {:.2}s", t.elapsed().as_secs_f64());

    // Scan = key decode only.
    let t = Instant::now();
    let mut count = 0usize;
    for (k, _) in snap.iter() {
        if let omt_bedrock::keys::DbKey::ChunkRecord { dim: 0, tag, .. } = omt_bedrock::keys::decode_key(&k)
            && tag == omt_bedrock::keys::tag::VERSION
        {
            count += 1;
        }
    }
    println!("scan: {count} chunks in {:.2}s", t.elapsed().as_secs_f64());

    // One tile build (tile 0,0 = chunks 0..32 × 0..32).
    let opts = omt_render::RenderOptions::default();
    let palette = omt_render::Palette::embedded();
    let t = Instant::now();
    let (x0, z0) = (0, 0);
    let coords: Vec<(i32, i32)> = (x0..x0 + 32)
        .flat_map(|cx| (z0 - 1..z0 + 32).map(move |cz| (cx, cz)))
        .collect();
    let decoded: Vec<_> = coords
        .par_iter()
        .map(|&(cx, cz)| {
            let base = omt_bedrock::keys::chunk_base(cx, cz, 0);
            let records: Vec<(Vec<u8>, Vec<u8>)> = snap
                .scan_prefix(&base)
                .map(|(k, v)| (k.to_vec(), v.to_vec()))
                .collect();
            (cx, cz, omt_bedrock::BedrockWorld::decode_snapshot_column(0, cx, cz, &records))
        })
        .collect();
    let mut renders = std::collections::HashMap::new();
    let mut errors = 0;
    for (cx, cz, c) in decoded {
        match c {
            Ok(Some(chunk)) => {
                renders.insert((cx, cz), omt_render::render_chunk(&chunk, palette, &opts));
            }
            Ok(None) => {}
            Err(_) => errors += 1,
        }
    }
    omt_render::shade_all(&mut renders);
    println!(
        "tile build: {} chunks rendered ({} errors) in {:.2}s → on-screen in ≈{:.2}s after scan",
        renders.len(),
        errors,
        t.elapsed().as_secs_f64(),
        t.elapsed().as_secs_f64()
    );
}
