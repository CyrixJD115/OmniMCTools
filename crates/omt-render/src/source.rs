//! [`MapSource`]: one dimension of a world opened for map rendering, with
//! memory-bounded bulk reads.
//!
//! * Bedrock: a [`omt_bedrock::ColumnIndex`] (one key-only scan) says which
//!   columns and subchunks exist; chunks are fetched with batched exact-key
//!   reads in bounded batches. The database is never materialized.
//! * Java: chunk presence comes from region headers (8 KiB per region);
//!   chunks of a region are decoded from ONE read of its `.mca`.
//!
//! The source holds only a **weak** reference to the world: a map left open
//! must never pin the store (the GUI's Save requires the session to hold
//! the only handle). Reads upgrade it for their duration; after the store
//! is reopened, [`MapSource::rebind_bedrock`]/[`MapSource::rebind_java`]
//! point the source at the new handle and keep the index.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use omt_bedrock::{BedrockWorld, ColumnIndex, DimIndex};
use omt_java::JavaWorld;
use omt_world::{ChunkSet, WorldError, WorldResult, WorldStore};
use rayon::prelude::*;

use crate::palette::Palette;
use crate::render::{ChunkRender, RenderOptions, render_chunk};

/// Columns decoded per Bedrock read batch (bounds transient memory: a
/// decoded column is ~50–150 KB).
pub const BEDROCK_BATCH: usize = 384;

#[derive(Clone)]
enum Backend {
    Bedrock { world: Weak<BedrockWorld>, root: std::path::PathBuf, index: Arc<DimIndex> },
    Java { world: Weak<JavaWorld>, chunks: Arc<ChunkSet>, region_fp: Arc<Mutex<HashMap<(i32, i32), RegionStamp>>> },
    Store { store: Arc<dyn WorldStore + Send + Sync>, chunks: Arc<ChunkSet> },
}

/// (level, read budget) → sampling stride.
type StrideMemo = HashMap<(u8, Option<u64>), i32>;

/// Memoized region fingerprint, valid while the file's size + mtime match.
#[derive(Clone, Copy)]
struct RegionStamp {
    len: u64,
    mtime: Option<std::time::SystemTime>,
    fp: u64,
}

/// A world dimension prepared for rendering. Cheap to clone (all `Arc`).
#[derive(Clone)]
pub struct MapSource {
    backend: Backend,
    dim: u8,
    /// Memo of [`crate::area::level_stride`] per (level, budget).
    strides: Arc<Mutex<StrideMemo>>,
}

/// Result of reading + rendering a set of chunks.
pub struct RenderedChunks {
    pub renders: Vec<ChunkRender>,
    /// Chunks that exist but failed to decode.
    pub errors: usize,
}

impl MapSource {
    /// Bedrock dimension from an already built column index (build it once
    /// per world with [`BedrockWorld::column_index`]; it covers every
    /// dimension, so switching dimension costs nothing).
    /// The caller keeps `world` alive (the source only holds a weak ref).
    pub fn bedrock(world: &Arc<BedrockWorld>, index: &ColumnIndex, dim: u8) -> Self {
        let dim_index = index.dim(dim).cloned().unwrap_or_default();
        Self {
            backend: Backend::Bedrock { world: Arc::downgrade(world), root: world.root.clone(), index: Arc::new(dim_index) },
            dim,
            strides: Arc::default(),
        }
    }

    /// Java dimension (region headers only). The caller keeps `world` alive.
    pub fn java(world: &Arc<JavaWorld>, dim: u8) -> WorldResult<Self> {
        let chunks: ChunkSet = world.list_chunks(dim)?.into_iter().collect();
        Ok(Self {
            backend: Backend::Java { world: Arc::downgrade(world), chunks: Arc::new(chunks), region_fp: Arc::default() },
            dim,
            strides: Arc::default(),
        })
    }

    /// Point a Bedrock source at a reopened handle of the same (unchanged)
    /// world. Returns false when this is not a Bedrock source.
    pub fn rebind_bedrock(&mut self, world: &Arc<BedrockWorld>) -> bool {
        match &mut self.backend {
            Backend::Bedrock { world: w, .. } => {
                *w = Arc::downgrade(world);
                true
            }
            _ => false,
        }
    }

    /// Point a Java source at a reopened handle of the same world.
    pub fn rebind_java(&mut self, world: &Arc<JavaWorld>) -> bool {
        match &mut self.backend {
            Backend::Java { world: w, .. } => {
                *w = Arc::downgrade(world);
                true
            }
            _ => false,
        }
    }

    /// Whether the world handle behind this source is still open.
    pub fn is_bound(&self) -> bool {
        match &self.backend {
            Backend::Bedrock { world, .. } => world.strong_count() > 0,
            Backend::Java { world, .. } => world.strong_count() > 0,
            Backend::Store { .. } => true,
        }
    }

    /// Whether this source reads through `world` (pointer identity).
    pub fn bound_to_bedrock(&self, world: &Arc<BedrockWorld>) -> bool {
        matches!(&self.backend, Backend::Bedrock { world: w, .. } if std::ptr::eq(w.as_ptr(), Arc::as_ptr(world)))
    }

    /// Whether this source reads through `world` (pointer identity).
    pub fn bound_to_java(&self, world: &Arc<JavaWorld>) -> bool {
        matches!(&self.backend, Backend::Java { world: w, .. } if std::ptr::eq(w.as_ptr(), Arc::as_ptr(world)))
    }

    /// Any other store (generic per-chunk reads; no caching fingerprint).
    pub fn store(store: Arc<dyn WorldStore + Send + Sync>, dim: u8) -> WorldResult<Self> {
        let chunks: ChunkSet = store.iter_chunks(dim)?.collect();
        Ok(Self { backend: Backend::Store { store, chunks: Arc::new(chunks) }, dim, strides: Arc::default() })
    }

    pub fn dim(&self) -> u8 {
        self.dim
    }

    /// Every chunk of the dimension.
    pub fn chunks(&self) -> &ChunkSet {
        match &self.backend {
            Backend::Bedrock { index, .. } => index.chunks(),
            Backend::Java { chunks, .. } | Backend::Store { chunks, .. } => chunks,
        }
    }

    /// The sampling stride of a LOD level under a per-tile read budget (one
    /// stride for the whole level; memoized).
    pub fn level_stride(&self, level: u8, budget: Option<u64>) -> i32 {
        if let Some(&s) = self.strides.lock().expect("stride memo").get(&(level, budget)) {
            return s;
        }
        let s = crate::area::level_stride(self.chunks(), level, budget);
        self.strides.lock().expect("stride memo").insert((level, budget), s);
        s
    }

    /// Whether render results of this source can be cached on disk.
    pub fn cacheable(&self) -> bool {
        !matches!(self.backend, Backend::Store { .. })
    }

    /// Content fingerprint of the data behind a chunk rectangle (inclusive):
    /// changes whenever any chunk in it may have changed. Bedrock: the
    /// database manifest + write-ahead log (re-read every call — edits made
    /// through the same workspace must never be cached under the old key);
    /// Java: the covered region files' headers (memo validated by size +
    /// mtime).
    pub fn fingerprint(&self, x0: i32, z0: i32, x1: i32, z1: i32) -> u64 {
        match &self.backend {
            Backend::Bedrock { root, .. } => bedrock_fingerprint(root),
            Backend::Store { .. } => 0,
            Backend::Java { world, chunks, region_fp } => {
                let Some(world) = world.upgrade() else { return 0 };
                let mut regions: Vec<(i32, i32)> = chunks
                    .in_rect(x0, z0, x1, z1)
                    .into_iter()
                    .map(|(x, z)| (x.div_euclid(32), z.div_euclid(32)))
                    .collect();
                regions.sort_unstable();
                regions.dedup();
                let mut h = Fnv::new();
                for r in regions {
                    let meta = world
                        .region_path(self.dim, r.0, r.1)
                        .metadata()
                        .ok()
                        .map(|m| (m.len(), m.modified().ok()));
                    let memo = region_fp.lock().expect("fingerprint memo").get(&r).copied();
                    let fp = match (memo, meta) {
                        (Some(st), Some((len, mtime))) if st.len == len && st.mtime == mtime => st.fp,
                        _ => {
                            let v = world.region_fingerprint(self.dim, r.0, r.1).unwrap_or(0);
                            if let Some((len, mtime)) = meta {
                                region_fp.lock().expect("fingerprint memo").insert(r, RegionStamp { len, mtime, fp: v });
                            }
                            v
                        }
                    };
                    h.u64(r.0 as u64);
                    h.u64(r.1 as u64);
                    h.u64(fp);
                }
                h.finish()
            }
        }
    }

    /// Read and render chunks (parallel, memory-bounded). Absent chunks are
    /// skipped; `cancel` is polled between batches. Returns `None` when it
    /// fired or the world handle was closed (see [`Self::is_bound`]).
    pub fn render(
        &self,
        coords: &[(i32, i32)],
        palette: &Palette,
        opts: &RenderOptions,
        cancel: &(dyn Fn() -> bool + Sync),
    ) -> Option<RenderedChunks> {
        let mut renders = Vec::with_capacity(coords.len());
        let mut errors = 0usize;
        match &self.backend {
            Backend::Bedrock { world, index, .. } => {
                let world = world.upgrade()?;
                let infos: Vec<_> = coords.iter().filter_map(|&(x, z)| Some((x, z, index.get(x, z)?))).collect();
                for batch in infos.chunks(BEDROCK_BATCH) {
                    if cancel() {
                        return None;
                    }
                    match world.read_render_columns(self.dim, batch) {
                        Ok(decoded) => {
                            let out: Vec<Result<Option<ChunkRender>, ()>> = decoded
                                .into_par_iter()
                                .map(|(_, _, c)| match c {
                                    Ok(Some(chunk)) => Ok(Some(render_chunk(&chunk, palette, opts))),
                                    // Listed by the index but every record
                                    // was deleted since: not an error.
                                    Ok(None) => Ok(None),
                                    Err(_) => Err(()),
                                })
                                .collect();
                            for r in out {
                                match r {
                                    Ok(Some(r)) => renders.push(r),
                                    Ok(None) => {}
                                    Err(()) => errors += 1,
                                }
                            }
                        }
                        Err(_) => errors += batch.len(),
                    }
                }
            }
            Backend::Java { world, chunks, .. } => {
                let world = world.upgrade()?;
                let mut by_region: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
                for &(x, z) in coords {
                    if chunks.contains(x, z) {
                        by_region.entry((x.div_euclid(32), z.div_euclid(32))).or_default().push((x, z));
                    }
                }
                let mut regions: Vec<_> = by_region.into_iter().collect();
                regions.sort_unstable_by_key(|(r, _)| (r.1, r.0));
                for ((rx, rz), cs) in regions {
                    if cancel() {
                        return None;
                    }
                    match world.read_region_chunks(self.dim, rx, rz, &cs) {
                        Ok(decoded) => {
                            let out: Vec<Result<Option<ChunkRender>, ()>> = decoded
                                .into_par_iter()
                                .map(|(_, c)| match c {
                                    Ok(Some(chunk)) => Ok(Some(render_chunk(&chunk, palette, opts))),
                                    Ok(None) => Ok(None),
                                    Err(_) => Err(()),
                                })
                                .collect();
                            for r in out {
                                match r {
                                    Ok(Some(r)) => renders.push(r),
                                    Ok(None) => {}
                                    Err(()) => errors += 1,
                                }
                            }
                        }
                        Err(_) => errors += cs.len(),
                    }
                }
            }
            Backend::Store { store, chunks } => {
                let present: Vec<(i32, i32)> = coords.iter().copied().filter(|&(x, z)| chunks.contains(x, z)).collect();
                for batch in present.chunks(256) {
                    if cancel() {
                        return None;
                    }
                    let out: Vec<Result<Option<ChunkRender>, WorldError>> = batch
                        .par_iter()
                        .map(|&(x, z)| store.read_chunk(self.dim, x, z).map(|c| c.map(|c| render_chunk(&c, palette, opts))))
                        .collect();
                    for r in out {
                        match r {
                            Ok(Some(r)) => renders.push(r),
                            Ok(None) => {}
                            Err(_) => errors += 1,
                        }
                    }
                }
            }
        }
        Some(RenderedChunks { renders, errors })
    }
}

/// Bedrock database fingerprint: CURRENT + the manifest it names (content
/// hash — stable across workspace copies, changes on any write/compaction).
pub fn bedrock_fingerprint(root: &std::path::Path) -> u64 {
    let db = root.join("db");
    let mut h = Fnv::new();
    if let Ok(current) = std::fs::read(db.join("CURRENT")) {
        h.bytes(&current);
        let name = String::from_utf8_lossy(&current).trim().to_string();
        if let Ok(m) = std::fs::read(db.join(&name)) {
            h.bytes(&m);
        }
    }
    // The write-ahead log holds records not yet in tables.
    if let Ok(entries) = std::fs::read_dir(&db) {
        let mut logs: Vec<(String, u64)> = entries
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.ends_with(".log").then(|| (n, e.metadata().map_or(0, |m| m.len())))
            })
            .collect();
        logs.sort();
        for (n, len) in logs {
            h.bytes(n.as_bytes());
            h.u64(len);
        }
    }
    h.finish()
}

/// FNV-1a 64 (stable across runs and platforms — cache keys).
#[derive(Clone, Copy)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self::new()
    }
}

impl Fnv {
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    pub fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn finish(self) -> u64 {
        self.0
    }
}
