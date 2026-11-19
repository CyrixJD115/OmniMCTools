//! Bulk chunk reads for the viewer's scanners (highlighters, inspector):
//! Java groups coordinates by region (one file read per region, parallel
//! decode); Bedrock reads columns in parallel. Runs on the renderer's
//! worker pool so the CPU setting applies.

use std::collections::BTreeMap;

use omt_world::{Chunk, WorldStore};

use crate::session::Store;

/// Read `coords` of dimension `dim`; missing / unreadable chunks are skipped.
pub fn read_chunks(store: &Store, dim: u8, coords: &[(i32, i32)], threads: usize) -> Vec<Chunk> {
    use rayon::prelude::*;
    omt_render::workers::install(threads, || match store {
        Store::Java(w) => {
            let mut by_region: BTreeMap<(i32, i32), Vec<(i32, i32)>> = BTreeMap::new();
            for &(x, z) in coords {
                by_region.entry((x.div_euclid(32), z.div_euclid(32))).or_default().push((x, z));
            }
            by_region
                .into_iter()
                .flat_map(|((rx, rz), cs)| w.read_region_chunks(dim, rx, rz, &cs).unwrap_or_default())
                .filter_map(|(_, r)| r.ok().flatten())
                .collect()
        }
        Store::Bedrock(w) => coords.par_iter().filter_map(|&(x, z)| w.read_chunk(dim, x, z).ok().flatten()).collect(),
    })
}
