//! Map-read benchmark: column index (key-only scan) → batched exact-key
//! reads → parallel decode → render, in bounded batches.
//!
//! `cargo run --release -p omt-bedrock --example renderbench -- <world> [max_columns] [batch]`

use omt_render::{Palette, RenderOptions, render_chunk};

fn rss_mb() -> u64 {
    omt_world::sysmem::process_rss().unwrap_or(0) / (1024 * 1024)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("world path");
    let max: usize = args.next().map_or(usize::MAX, |s| s.parse().expect("max columns"));
    let batch: usize = args.next().map_or(512, |s| s.parse().expect("batch"));
    let world = omt_bedrock::BedrockWorld::open_read_only(std::path::Path::new(&path)).unwrap();

    let t = std::time::Instant::now();
    let index = world.column_index().unwrap();
    let dim = index.dim(0).cloned().unwrap_or_default();
    println!(
        "index: {} keys → {} overworld chunks, {} dims, {:.1} MB index, {:?}, rss {} MB",
        index.keys_scanned,
        dim.len(),
        index.dims().len(),
        index.heap_bytes() as f64 / 1e6,
        t.elapsed(),
        rss_mb()
    );

    let mut cols: Vec<(i32, i32)> = dim.chunks().iter().collect();
    cols.sort_unstable_by_key(|&(x, z)| (z, x));
    cols.truncate(max);
    let palette = Palette::embedded();
    let opts = RenderOptions::default();
    let t = std::time::Instant::now();
    let (mut ok, mut errs, mut read_t, mut peak) = (0usize, 0usize, std::time::Duration::ZERO, 0u64);
    for chunk in cols.chunks(batch) {
        let infos: Vec<_> = chunk.iter().map(|&(x, z)| (x, z, dim.get(x, z).unwrap())).collect();
        let tr = std::time::Instant::now();
        let decoded = world.read_render_columns(0, &infos).unwrap();
        read_t += tr.elapsed();
        use rayon::prelude::*;
        let renders: Vec<_> = decoded
            .into_par_iter()
            .map(|(_, _, c)| match c {
                Ok(Some(c)) => Some(render_chunk(&c, palette, &opts)),
                _ => None,
            })
            .collect();
        for r in renders {
            if r.is_some() { ok += 1 } else { errs += 1 }
        }
        peak = peak.max(rss_mb());
    }
    let el = t.elapsed();
    println!(
        "rendered {ok} ({errs} empty/err) of {} in {el:?} ({:.1} µs/chunk; read+decode {read_t:?}), peak rss {peak} MB",
        cols.len(),
        el.as_micros() as f64 / cols.len().max(1) as f64
    );
}
