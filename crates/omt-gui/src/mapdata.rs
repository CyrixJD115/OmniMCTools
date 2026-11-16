//! Map data for the canvas: **on-demand region tiles** (32×32 chunks →
//! 512×512 px) built from a world snapshot, plus per-chunk surface info for
//! click-identify. Large worlds never render up front: the GUI asks for the
//! tiles the camera can see, keeps a bounded cache, and offloads tiles that
//! scroll off screen.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use iced::widget::image::Handle;
use omt_render::{Palette, RenderOptions};

use crate::session::Store;

pub const TILE_PX: u32 = 512;
/// Chunks per tile edge (32×32 chunks → 512×512 px at 1 px/block).
pub const TILE_CHUNKS: i32 = 32;
/// Built-tile cache cap; tiles farther than the keep-range are dropped.
pub const MAX_TILES: usize = 64;
/// Concurrent background tile builds.
pub const MAX_INFLIGHT: usize = 2;

/// Surface info of one chunk: block-name ids (into a tile's `names`) and
/// heights per column (`i16::MIN` = empty).
#[derive(Debug, Clone)]
pub struct Surface {
    pub names: Box<[u16; 256]>,
    pub heights: Box<[i16; 256]>,
}

/// Cheap whole-dimension enumeration (key decode only — no chunk decode).
#[derive(Debug, Clone)]
pub struct MapScan {
    pub dim: u8,
    /// Bounding box of present chunks (min_cx, min_cz, max_cx, max_cz).
    pub bounds: Option<(i32, i32, i32, i32)>,
    /// Every present chunk (used for "is this chunk loaded?" checks and
    /// pruner overlays without building tiles).
    pub chunks: Arc<HashSet<(i32, i32)>>,
    pub millis: u128,
}

/// Raw output of one background tile build (no GPU handles yet).
#[derive(Debug, Clone)]
pub struct MapBuild {
    pub tile: (i32, i32),
    pub rgba: Vec<u8>,
    pub chunks: HashSet<(i32, i32)>,
    pub surface: HashMap<(i32, i32), Surface>,
    pub names: Vec<String>,
    pub errors: usize,
    pub millis: u128,
}

/// One built tile: the GPU image plus its chunk metadata.
#[derive(Debug, Clone)]
pub struct TileData {
    pub handle: Handle,
    pub surface: Arc<HashMap<(i32, i32), Surface>>,
    pub names: Arc<Vec<String>>,
}

/// The lazy map: scan metadata + a bounded cache of built tiles.
#[derive(Debug, Clone, Default)]
pub struct MapData {
    pub scan: Option<MapScan>,
    pub tiles: HashMap<(i32, i32), TileData>,
}

impl MapData {
    pub fn with_scan(scan: MapScan) -> Self {
        Self { scan: Some(scan), tiles: HashMap::new() }
    }

    /// Insert a finished tile build (replacing any previous build of it).
    pub fn insert(&mut self, build: MapBuild) {
        let MapBuild { tile, rgba, surface, names, .. } = build;
        let handle = Handle::from_rgba(TILE_PX, TILE_PX, rgba);
        self.tiles
            .insert(tile, TileData { handle, surface: Arc::new(surface), names: Arc::new(names) });
    }

    /// Range-eviction + cache cap in one step. Returns dropped tiles.
    pub fn prune_cache(&mut self, keep: (i32, i32, i32, i32), center_tile: (i32, i32)) -> usize {
        let dropped = self.evict_outside((keep.0 - 3, keep.1 - 3, keep.2 + 3, keep.3 + 3));
        let extra = evict_plan(&self.tiles, keep, center_tile, MAX_TILES);
        let n = extra.len();
        for t in extra {
            self.tiles.remove(&t);
        }
        dropped + n
    }

    /// Drop tiles outside the keep range (off-screen offload). Returns the
    /// number of tiles dropped.
    pub fn evict_outside(&mut self, keep: (i32, i32, i32, i32)) -> usize {
        let before = self.tiles.len();
        let (rx0, rz0, rx1, rz1) = keep;
        self.tiles.retain(|&(rx, rz), _| rx >= rx0 && rx <= rx1 && rz >= rz0 && rz <= rz1);
        before - self.tiles.len()
    }

    /// Surface block + height at a block position, if rendered.
    pub fn identify(&self, bx: i32, bz: i32) -> Option<(&str, i32)> {
        let (cx, cz) = (bx.div_euclid(16), bz.div_euclid(16));
        let tile = self.tiles.get(&(cx.div_euclid(TILE_CHUNKS), cz.div_euclid(TILE_CHUNKS)))?;
        let s = tile.surface.get(&(cx, cz))?;
        let i = (bz.rem_euclid(16) * 16 + bx.rem_euclid(16)) as usize;
        let h = s.heights[i];
        if h == i16::MIN {
            return None;
        }
        Some((tile.names.get(s.names[i] as usize)?.as_str(), h as i32))
    }

    /// Whether the chunk exists in the world (scan-level, tile-independent).
    pub fn chunk_present(&self, cx: i32, cz: i32) -> bool {
        self.scan.as_ref().is_some_and(|s| s.chunks.contains(&(cx, cz)))
    }

    /// Full-world bounding box from the scan (stable regardless of which
    /// tiles are built).
    pub fn chunk_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        self.scan.as_ref().and_then(|s| s.bounds)
    }

    /// Total chunks present in the dimension.
    pub fn chunk_total(&self) -> usize {
        self.scan.as_ref().map_or(0, |s| s.chunks.len())
    }

}

/// Tile range covering a block-space viewport rectangle (a tile is
/// `TILE_CHUNKS * 16` blocks on a side).
pub fn tile_range(min_bx: f64, min_bz: f64, max_bx: f64, max_bz: f64) -> (i32, i32, i32, i32) {
    const BLOCKS_PER_TILE: i32 = TILE_CHUNKS * 16;
    let tx = |bx: f64| (bx.floor() as i32).div_euclid(BLOCKS_PER_TILE);
    let tz = |bz: f64| (bz.floor() as i32).div_euclid(BLOCKS_PER_TILE);
    let (a, b) = (tx(min_bx).min(tx(max_bx)), tx(min_bx).max(tx(max_bx)));
    let (c, d) = (tz(min_bz).min(tz(max_bz)), tz(min_bz).max(tz(max_bz)));
    (a, c, b, d)
}

/// Which missing tiles to build next, nearest the camera center first, up to
/// `max` entries. `want` is an inclusive tile-space rectangle.
pub fn refill_plan(
    built: &HashMap<(i32, i32), TileData>,
    inflight: &HashSet<(i32, i32)>,
    want: (i32, i32, i32, i32),
    center_tile: (i32, i32),
    max: usize,
) -> Vec<(i32, i32)> {
    let (rx0, rz0, rx1, rz1) = want;
    let mut cands: Vec<(i32, i32)> = (rx0..=rx1)
        .flat_map(|rx| (rz0..=rz1).map(move |rz| (rx, rz)))
        .filter(|t| !built.contains_key(t) && !inflight.contains(t))
        .collect();
    cands.sort_by_key(|&(rx, rz)| {
        let (dx, dz) = (rx - center_tile.0, rz - center_tile.1);
        dx * dx + dz * dz
    });
    cands.truncate(max);
    cands
}

/// Tiles to drop when over the cache cap: farthest from the camera center
/// first, never the ones in `keep`.
pub fn evict_plan(
    tiles: &HashMap<(i32, i32), TileData>,
    keep: (i32, i32, i32, i32),
    center_tile: (i32, i32),
    cap: usize,
) -> Vec<(i32, i32)> {
    let overflow = tiles.len().saturating_sub(cap);
    if overflow == 0 {
        return Vec::new();
    }
    let (rx0, rz0, rx1, rz1) = keep;
    let mut cands: Vec<((i32, i32), u64)> = tiles
        .keys()
        .filter(|&&(rx, rz)| !(rx >= rx0 && rx <= rx1 && rz >= rz0 && rz <= rz1))
        .map(|&(rx, rz)| {
            let (dx, dz) = (rx - center_tile.0, rz - center_tile.1);
            ((rx, rz), (dx * dx + dz * dz) as u64)
        })
        .collect();
    cands.sort_by_key(|&(_, d)| std::cmp::Reverse(d));
    cands.into_iter().take(overflow).map(|(t, _)| t).collect()
}

/// Cheap whole-dimension enumeration.
pub fn scan(store: &Store, dim: u8) -> Result<MapScan, String> {
    let started = std::time::Instant::now();
    let mut chunks = HashSet::new();
    for c in store.world().iter_chunks(dim).map_err(|e| e.to_string())? {
        chunks.insert(c);
    }
    let bounds = chunks.iter().fold(None, |acc: Option<(i32, i32, i32, i32)>, &(x, z)| match acc {
        None => Some((x, z, x, z)),
        Some((x0, z0, x1, z1)) => Some((x0.min(x), z0.min(z), x1.max(x), z1.max(z))),
    });
    Ok(MapScan { dim, bounds, chunks: Arc::new(chunks), millis: started.elapsed().as_millis() })
}

/// A shareable per-world decode source for background tile builds. Take it
/// **once** per map (the Bedrock snapshot materializes the database once —
/// per-tile snapshots would copy the whole DB per tile).
#[derive(Clone)]
pub enum TileSource {
    Bedrock(bedrock_leveldb::Snapshot),
    Java(Arc<omt_java::JavaWorld>),
}

pub fn tile_source(store: &Store) -> Result<TileSource, String> {
    match store {
        Store::Bedrock(w) => Ok(TileSource::Bedrock(w.snapshot().map_err(|e| e.to_string())?)),
        Store::Java(w) => Ok(TileSource::Java(w.clone())),
    }
}

/// Render one region tile (blocking; call via `session::blocking`).
pub fn build_tile(src: &TileSource, dim: u8, opts: RenderOptions, rx: i32, rz: i32) -> Result<MapBuild, String> {
    let started = std::time::Instant::now();
    let palette = Palette::embedded();
    let (x0, z0) = (rx * TILE_CHUNKS, rz * TILE_CHUNKS);
    // + the northern neighbor row: cross-chunk height shading needs its
    // heights for the tile's first row (kept out of the final tile pixels).
    let mut coords = Vec::with_capacity((TILE_CHUNKS * (TILE_CHUNKS + 1)) as usize);
    for cx in x0..x0 + TILE_CHUNKS {
        for cz in z0 - 1..z0 + TILE_CHUNKS {
            coords.push((cx, cz));
        }
    }
    match src {
        TileSource::Java(world) => {
            let (mut renders, errors) = omt_render::render_chunks(world.as_ref(), dim, &coords, palette, &opts);
            omt_render::shade_all(&mut renders);
            let coords: Vec<(i32, i32)> = coords
                .into_iter()
                .filter(|&(_, cz)| cz >= z0 && cz < z0 + TILE_CHUNKS)
                .collect();
            Ok(assemble(rx, rz, coords, renders, errors, started))
        }
        TileSource::Bedrock(snap) => {
            use rayon::prelude::*;
            let decoded: Vec<_> = coords
                .par_iter()
                .map(|&(cx, cz)| {
                    let base = omt_bedrock::keys::chunk_base(cx, cz, dim);
                    let records: Vec<(Vec<u8>, Vec<u8>)> = snap
                        .scan_prefix(&base)
                        .filter(|(k, _)| {
                            let rest = &k[base.len()..];
                            matches!(rest.len(), 1 | 2)
                        })
                        .map(|(k, v)| (k.to_vec(), v.to_vec()))
                        .collect();
                    (cx, cz, omt_bedrock::BedrockWorld::decode_snapshot_column(dim, cx, cz, &records))
                })
                .collect();
            let mut renders = HashMap::new();
            let mut errors = 0usize;
            for (cx, cz, chunk) in decoded {
                match chunk {
                    Ok(Some(chunk)) => {
                        renders.insert((cx, cz), omt_render::render_chunk(&chunk, palette, &opts));
                    }
                    Ok(None) => {}
                    Err(_) => errors += 1,
                }
            }
            omt_render::shade_all(&mut renders);
            let coords: Vec<(i32, i32)> = renders
                .keys()
                .copied()
                .filter(|&(_, cz)| cz >= z0 && cz < z0 + TILE_CHUNKS)
                .collect();
            Ok(assemble(rx, rz, coords, renders, errors, started))
        }
    }
}

fn assemble(
    rx: i32,
    rz: i32,
    coords: Vec<(i32, i32)>,
    renders: HashMap<(i32, i32), omt_render::ChunkRender>,
    errors: usize,
    started: std::time::Instant,
) -> MapBuild {
    let mut tiles: HashMap<(i32, i32), Vec<u8>> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    let mut name_ids: HashMap<String, u16> = HashMap::new();
    let mut surface = HashMap::new();
    let mut chunks = HashSet::new();
    for ((cx, cz), r) in renders {
        chunks.insert((cx, cz));
        let (ox, oz) = (cx.rem_euclid(TILE_CHUNKS) as usize * 16, cz.rem_euclid(TILE_CHUNKS) as usize * 16);
        let tile = tiles.entry((rx, rz)).or_insert_with(|| vec![0u8; (TILE_PX * TILE_PX * 4) as usize]);
        for lz in 0..16 {
            let dst = ((oz + lz) * TILE_PX as usize + ox) * 4;
            tile[dst..dst + 64].copy_from_slice(&r.rgba[lz * 64..lz * 64 + 64]);
        }
        let mut ids = Box::new([0u16; 256]);
        let mut hs = Box::new([i16::MIN; 256]);
        for i in 0..256 {
            if let Some(name) = &r.surface[i] {
                let next = names.len() as u16;
                let id = *name_ids.entry(name.clone()).or_insert_with(|| {
                    names.push(name.clone());
                    next
                });
                ids[i] = id;
            }
            if r.heights[i] != i32::MIN {
                hs[i] = r.heights[i].clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16;
            }
        }
        surface.insert((cx, cz), Surface { names: ids, heights: hs });
    }
    // Chunks that exist but failed to render still count as present.
    chunks.extend(coords);
    MapBuild {
        tile: (rx, rz),
        rgba: tiles.remove(&(rx, rz)).unwrap_or_else(|| vec![0u8; (TILE_PX * TILE_PX * 4) as usize]),
        chunks,
        surface,
        names,
        errors,
        millis: started.elapsed().as_millis(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile_set(tiles: &[(i32, i32)]) -> HashMap<(i32, i32), TileData> {
        tiles.iter().map(|&t| (t, TileData {
            handle: Handle::from_rgba(1, 1, vec![0; 4]),
            surface: Arc::default(),
            names: Arc::default(),
        })).collect()
    }

    #[test]
    fn tile_range_covers_viewport_and_handles_negatives() {
        // One tile = 32 chunks = 512 blocks.
        assert_eq!(tile_range(0.0, 0.0, 511.9, 511.9), (0, 0, 0, 0));
        assert_eq!(tile_range(-1.0, -1.0, 10.0, 10.0), (-1, -1, 0, 0));
        assert_eq!(tile_range(-600.0, 300.0, -100.0, 700.0), (-2, 0, -1, 1));
        assert_eq!(tile_range(513.0, 513.0, 1024.0, 1024.0), (1, 1, 2, 2));
    }

    #[test]
    fn refill_nearest_first_and_skips_built_inflight() {
        let built = tile_set(&[(0, 0)]);
        let mut inflight = HashSet::new();
        inflight.insert((0, 1));
        let plan = refill_plan(&built, &inflight, (-1, -1, 1, 1), (0, 0), 10);
        assert!(!plan.contains(&(0, 0)), "built tile not re-queued");
        assert!(!plan.contains(&(0, 1)), "inflight tile not re-queued");
        // Nearest ring first: all edge tiles share Manhattan distance 1
        // (ties keep enumeration order); corners (distance 2) come last.
        let first = plan.first().copied().unwrap();
        assert_eq!((first.0).abs() + (first.1).abs(), 1, "first {first:?} is on the distance-1 ring");
        let dists: Vec<i32> = plan.iter().map(|t| t.0.abs() + t.1.abs()).collect();
        let mut sorted = dists.clone();
        sorted.sort_unstable();
        assert_eq!(dists, sorted, "plan is distance-sorted");
        assert_eq!(plan.len(), 7); // 3×3 minus built minus inflight
        // cap respected
        assert_eq!(refill_plan(&built, &inflight, (-1, -1, 1, 1), (0, 0), 2).len(), 2);
    }

    #[test]
    fn eviction_drops_farthest_outside_keep_range() {
        let tiles = tile_set(&[(0, 0), (5, 5), (-6, -6), (1, 1)]);
        // cap 4, keep range around (0,0): (5,5) and (-6,-6) are outside → drop the farther one(s).
        let plan = evict_plan(&tiles, (-1, -1, 1, 1), (0, 0), 3);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0], (-6, -6)); // farther than (5,5)
        // cap not exceeded → nothing dropped
        assert!(evict_plan(&tiles, (-1, -1, 1, 1), (0, 0), 4).is_empty());
        // keep-range tiles are never dropped even when over cap
        let two = tile_set(&[(0, 0), (1, 1)]);
        assert!(evict_plan(&two, (-1, -1, 1, 1), (0, 0), 1).is_empty());
    }

    #[test]
    fn identify_and_eviction_through_mapdata() {
        let mut map = MapData::with_scan(MapScan {
            dim: 0,
            bounds: Some((-40, -40, 40, 40)),
            chunks: Arc::new(HashSet::from([(-1, 0)])),
            millis: 0,
        });
        assert!(map.chunk_present(-1, 0));
        assert!(!map.chunk_present(9, 9));
        assert_eq!(map.chunk_bounds(), Some((-40, -40, 40, 40)));
        assert_eq!(map.chunk_total(), 1);
        // A tile outside the keep range is evicted; inside stays.
        map.insert(MapBuild {
            tile: (2, 2),
            rgba: vec![0; (TILE_PX * TILE_PX * 4) as usize],
            chunks: HashSet::from([(64, 64)]),
            surface: HashMap::new(),
            names: Vec::new(),
            errors: 0,
            millis: 0,
        });
        assert_eq!(map.evict_outside((-1, -1, 1, 1)), 1);
        assert!(!map.tiles.contains_key(&(2, 2)));
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    fn fixture() -> Option<std::path::PathBuf> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../test_worlds/bedrock-lostsurfur");
        p.join("level.dat").is_file().then_some(p.to_path_buf())
    }

    /// The on-demand tile builder must produce the same pixels as the
    /// whole-world CLI render for the same area — lazy must equal eager.
    #[test]
    #[ignore = "needs the Bedrock test world"]
    fn tile_pixels_match_whole_world_render() {
        use omt_render::{RenderOptions, render_world};

        let Some(src_path) = fixture() else { eprintln!("skipped: no test world"); return };
        let world = omt_bedrock::BedrockWorld::open_read_only(&src_path).unwrap();
        let src = TileSource::Bedrock(world.snapshot().unwrap());
        let opts = RenderOptions::default();

        // Tile containing the spawn area (chunk 0,0 → tile 0,0).
        let build = build_tile(&src, 0, opts, 0, 0).unwrap();
        assert!(!build.chunks.is_empty(), "tile must contain chunks");

        // Reference: full render via the store, crop the same tile region.
        let world2 = omt_bedrock::BedrockWorld::open_read_only(&src_path).unwrap();
        let (image, rendered, errors) =
            render_world(&world2, 0, Some((-32, -32, 63, 63)), &opts).unwrap();
        assert!(rendered > 0 && errors == 0);
        let img = image.unwrap();
        let ox = ((0 - img.min_cx) * 16) as usize;
        let oz = ((0 - img.min_cz) * 16) as usize;
        let mut mismatch = 0usize;
        for lz in 0..512 {
            for lx in 0..512 {
                let x = ox + lx;
                let y = oz + lz;
                if x >= img.width || y >= img.height {
                    continue;
                }
                let a = &build.rgba[(lz * 512 + lx) * 4..(lz * 512 + lx) * 4 + 4];
                let b = &img.rgba[(y * img.width + x) * 4..(y * img.width + x) * 4 + 4];
                if a != b {
                    mismatch += 1;
                }
            }
        }
        assert_eq!(mismatch, 0, "tile pixels diverge from the whole-world render");
    }
}
