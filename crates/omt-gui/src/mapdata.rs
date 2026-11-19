//! Map data for the viewer canvas: an **on-demand, level-of-detail tile
//! pyramid** with memory protection, shared by the Chunk Locator and the
//! Pruner through [`MapController`].
//!
//! * Only tiles the camera can see are built (nearest the center first);
//!   tiles that scroll off screen are offloaded first when over budget.
//! * Zooming out switches to coarser levels: a level-L tile covers
//!   `32 << L` chunks in one image whose pixel = 2^L blocks (exact box
//!   average). The **Map Quality** setting caps chunk reads per tile — past
//!   the cap, tiles sample one chunk per cell (fast, lower detail).
//! * The in-memory cache is capped in **bytes** (Map Memory setting) and
//!   shrinks automatically when the system runs low on RAM; new builds
//!   pause when memory is critical.
//! * Finished tiles are also written to an on-disk cache keyed by content,
//!   so reopening a world shows its map instantly.
//! * The world is never materialized: Bedrock reads go through a key-only
//!   column index + batched exact reads; the source holds only a weak
//!   reference to the store (Save keeps working while a map is open).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use iced::{Size, Task};
use omt_bedrock::ColumnIndex;
use omt_render::style::StyleParams;
use omt_render::{MapSource, Palette, RenderOptions, TileCache, TileImage, TileKey, MAX_LEVEL};


use crate::mapview::Camera;
use crate::session::{Store, blocking};


pub use crate::mapconfig::*;

/// Display level for a zoom (screen px per block).
pub fn level_for(zoom: f32, bias: i8) -> u8 {
    let l = if zoom >= 1.0 { 0 } else { (1.0 / zoom).log2().floor() as i32 };
    (l - bias as i32).clamp(0, MAX_LEVEL as i32) as u8
}

/// Inclusive tile range of `level` covering a block-space rectangle.
pub fn tile_range(level: u8, min_bx: f64, min_bz: f64, max_bx: f64, max_bz: f64) -> (i32, i32, i32, i32) {
    let span = (omt_render::TILE_CHUNKS as i64) << level;
    let t = |b: f64| ((b.floor() as i64).div_euclid(16)).div_euclid(span) as i32;
    (t(min_bx).min(t(max_bx)), t(min_bz).min(t(max_bz)), t(min_bx).max(t(max_bx)), t(min_bz).max(t(max_bz)))
}

pub use crate::maptile::TileData;

/// The lazy map of one dimension: its source + a byte-bounded tile cache.
pub struct MapData {
    pub source: MapSource,
    pub tiles: HashMap<TileKey, TileData>,
    bytes: usize,
    /// Level currently displayed.
    pub level: u8,
    /// Coarser tiles whose on-screen area is fully covered by loaded tiles
    /// of the display level — not drawn (their big blurred pixels would
    /// otherwise halo through the gaps of the sharper tiles).
    hidden: HashSet<TileKey>,
    /// Style post-process (Map Viewer); `None` = show tiles as rendered.
    pub post: Option<StyleParams>,
}

impl std::fmt::Debug for MapData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapData").field("tiles", &self.tiles.len()).field("bytes", &self.bytes).finish()
    }
}

impl MapData {
    pub fn new(source: MapSource) -> Self {
        Self { source, tiles: HashMap::new(), bytes: 0, level: 0, hidden: HashSet::new(), post: None }
    }

    /// Unreadable chunks across the loaded tiles (drawn as holes).
    pub fn errors(&self) -> usize {
        self.tiles.values().map(|t| t.errors).sum()
    }

    /// Change the style post-process and re-light every loaded tile (in
    /// parallel; no world reads).
    pub fn set_post(&mut self, p: Option<StyleParams>) {
        if self.post == p {
            return;
        }
        self.post = p;
        if let Some(p) = p {
            use rayon::prelude::*;
            let mut tiles: Vec<&mut TileData> = self.tiles.values_mut().collect();
            tiles.par_iter_mut().for_each(|t| t.restyle(&p));
        }
    }

    pub fn insert(&mut self, key: TileKey, t: TileImage) {
        let d = TileData::from_image(t, self.post.as_ref());
        self.bytes += d.bytes;
        if let Some(old) = self.tiles.insert(key, d) {
            self.bytes -= old.bytes;
        }
    }

    fn remove(&mut self, key: &TileKey) {
        if let Some(old) = self.tiles.remove(key) {
            self.bytes -= old.bytes;
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Drop tiles until under `budget` bytes: off-screen tiles first
    /// (farthest from the camera), then other levels (most distant level
    /// first). Tiles in `keep` (on screen at the current level) stay.
    pub fn evict(&mut self, budget: usize, level: u8, keep: &HashSet<TileKey>, center: (f64, f64)) -> usize {
        if self.bytes <= budget {
            return 0;
        }
        let mut cands: Vec<(TileKey, (u8, u64))> = self
            .tiles
            .keys()
            .filter(|k| !keep.contains(k))
            .map(|&k| {
                let half = k.span_blocks() as f64 / 2.0;
                let (x0, z0, _, _) = k.chunk_rect();
                let (cx, cz) = (x0 as f64 * 16.0 + half, z0 as f64 * 16.0 + half);
                let d = ((cx - center.0).powi(2) + (cz - center.1).powi(2)).sqrt() as u64;
                (k, ((k.level as i32 - level as i32).unsigned_abs() as u8, d))
            })
            .collect();
        // Worst first: farthest level, then farthest distance.
        cands.sort_unstable_by_key(|&(_, (l, d))| std::cmp::Reverse((l, d)));
        let mut n = 0;
        for (k, _) in cands {
            if self.bytes <= budget {
                break;
            }
            self.remove(&k);
            n += 1;
        }
        n
    }

    /// Recompute which coarse fallback tiles are hidden: a tile coarser than
    /// `level` is hidden once every display-level tile inside it that is on
    /// screen (`view`, an inclusive tile range at `level`) and has chunks is
    /// loaded.
    pub fn update_hidden(&mut self, level: u8, view: (i32, i32, i32, i32)) {
        let (tx0, tz0, tx1, tz1) = view;
        let coarse: Vec<TileKey> = self.tiles.keys().copied().filter(|k| k.level > level).collect();
        let mut hidden = HashSet::new();
        for k in coarse {
            let shift = (k.level - level) as u32;
            let n = 1i64 << shift;
            let (cx0, cz0) = (k.tx as i64 * n, k.tz as i64 * n);
            let a0 = cx0.max(tx0 as i64);
            let b0 = cz0.max(tz0 as i64);
            let a1 = (cx0 + n - 1).min(tx1 as i64);
            let b1 = (cz0 + n - 1).min(tz1 as i64);
            let covered = a0 > a1
                || b0 > b1
                || (b0..=b1).all(|tz| {
                    (a0..=a1).all(|tx| {
                        let c = TileKey::new(level, tx as i32, tz as i32);
                        let (x0, z0, x1, z1) = c.chunk_rect();
                        self.tiles.contains_key(&c) || !self.source.chunks().any_in_rect(x0, z0, x1, z1)
                    })
                });
            if covered {
                hidden.insert(k);
            }
        }
        self.hidden = hidden;
    }

    /// Tiles to draw, coarsest first (finer levels paint over coarser
    /// fallbacks while they load; fully covered fallbacks are skipped).
    pub fn draw_list(&self) -> Vec<(TileKey, &TileData)> {
        let mut v: Vec<(TileKey, &TileData)> = self
            .tiles
            .iter()
            .filter(|(k, t)| t.handle.is_some() && !self.hidden.contains(k))
            .map(|(k, t)| (*k, t))
            .collect();
        v.sort_unstable_by_key(|(k, _)| std::cmp::Reverse(k.level));
        v
    }

    /// Surface block + height at a block position from the finest loaded
    /// tile. `approx` is true when that tile is not full detail.
    pub fn identify(&self, bx: i32, bz: i32) -> Option<(&str, i32, bool)> {
        let (cx, cz) = (bx.div_euclid(16), bz.div_euclid(16));
        if !self.source.chunks().contains(cx, cz) {
            return None;
        }
        for level in 0..=MAX_LEVEL {
            let key = TileKey::containing(level, cx, cz);
            let Some(t) = self.tiles.get(&key) else { continue };
            if t.px == 0 {
                continue;
            }
            let bpp = key.span_blocks() / t.px as i64;
            let (x0, z0, _, _) = key.chunk_rect();
            let lx = (bx as i64 - x0 as i64 * 16).div_euclid(bpp);
            let lz = (bz as i64 - z0 as i64 * 16).div_euclid(bpp);
            if lx < 0 || lz < 0 || lx >= t.px as i64 || lz >= t.px as i64 {
                continue;
            }
            let i = (lz * t.px as i64 + lx) as usize;
            let h = *t.heights.get(i)?;
            if h == i16::MIN {
                return None;
            }
            let name = t.names.get(*t.surface.get(i)? as usize)?;
            return Some((name.as_str(), h as i32, level > 0 || t.reduced()));
        }
        None
    }

    /// Full-detail surface block + styled color at a block (level-0 tile,
    /// sampled 1:1 only).
    pub fn block_sample(&self, bx: i32, bz: i32) -> Option<(&str, [u8; 4])> {
        let key = TileKey::containing(0, bx.div_euclid(16), bz.div_euclid(16));
        let t = self.tiles.get(&key)?;
        if t.px as i64 != key.span_blocks() {
            return None;
        }
        let (x0, z0, _, _) = key.chunk_rect();
        t.sample((bx - x0 * 16) as usize, (bz - z0 * 16) as usize)
    }

    /// Whether the full-detail tile holding a block is loaded.
    pub fn full_tile_loaded(&self, bx: i32, bz: i32) -> bool {
        let key = TileKey::containing(0, bx.div_euclid(16), bz.div_euclid(16));
        self.tiles.get(&key).is_some_and(|t| t.px as i64 == key.span_blocks())
    }

    pub fn chunk_present(&self, cx: i32, cz: i32) -> bool {
        self.source.chunks().contains(cx, cz)
    }

    /// Bounds used to fit the view: the world's core (2 % trimmed per side),
    /// so far-away outliers and long travel trails don't shrink the main
    /// area to a dot. Everything stays reachable by zooming out.
    pub fn chunk_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        self.source.chunks().core_bounds(0.02)
    }

    pub fn chunk_total(&self) -> usize {
        self.source.chunks().len()
    }
}

/// Result of loading a dimension.
#[derive(Clone)]
pub struct Loaded {
    pub source: MapSource,
    pub index: Option<Arc<ColumnIndex>>,
    pub millis: u128,
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loaded").field("chunks", &self.source.chunks().len()).field("millis", &self.millis).finish()
    }
}

#[derive(Debug, Clone)]
pub enum MapMsg {
    Loaded(u64, u8, Result<Loaded, String>),
    /// `Ok(None)` = cancelled (scrolled away / options changed).
    Tile(u64, TileKey, Result<Option<Arc<TileImage>>, String>),
}

/// Owns one viewer's camera, tile pyramid and background builds.
pub struct MapController {
    pub dim: u8,
    pub camera: Camera,
    pub view: Size,
    pub map: Option<MapData>,
    opts: RenderOptions,
    pub loading: bool,
    fitted: bool,
    /// Bumped whenever results in flight become stale.
    epoch: Arc<AtomicU64>,
    inflight: HashSet<TileKey>,
    /// Tiles still wanted (builds of anything else cancel themselves).
    wanted: Arc<Mutex<HashSet<TileKey>>>,
    /// Bedrock column index (all dimensions) of the current store state.
    index: Option<Arc<ColumnIndex>>,
    pub status: String,
    pub paused: bool,
    /// Quality the cached tiles were built with (a change rebuilds them).
    quality_seen: Option<MapQuality>,
    /// Debug/testing hook (`OMT_DEBUG_MAP_CAMERAS="x,z,zoom;x,z,zoom"`): each
    /// camera is applied once the previous view has finished loading — lets
    /// headless runs reproduce zoom sequences without input devices.
    script: std::collections::VecDeque<Camera>,
    /// Style post-process applied to every map this controller loads.
    pub post: Option<StyleParams>,
    /// `view` holds a real canvas size (not the startup placeholder).
    view_known: bool,
    /// A debug camera script moved the camera (don't auto-fit over it).
    scripted: bool,
}

fn debug_cameras() -> std::collections::VecDeque<Camera> {
    std::env::var("OMT_DEBUG_MAP_CAMERAS")
        .unwrap_or_default()
        .split(';')
        .filter_map(|c| {
            let v: Vec<f64> = c.split(',').filter_map(|n| n.trim().parse().ok()).collect();
            (v.len() == 3).then(|| Camera { cx: v[0], cz: v[1], zoom: (v[2] as f32).clamp(Camera::MIN_ZOOM, Camera::MAX_ZOOM) })
        })
        .collect()
}

impl MapController {
    pub fn new(dim: u8, opts: RenderOptions) -> Self {
        Self {
            dim,
            camera: Camera::default(),
            view: Size::new(900.0, 640.0),
            map: None,
            opts,
            loading: false,
            fitted: false,
            epoch: Arc::new(AtomicU64::new(0)),
            inflight: HashSet::new(),
            wanted: Arc::default(),
            index: None,
            status: String::new(),
            paused: false,
            quality_seen: None,
            script: debug_cameras(),
            post: None,
            view_known: false,
            scripted: false,
        }
    }

    fn generation(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// Invalidate everything in flight (results of older generations are
    /// dropped; running builds cancel at their next batch).
    fn bump(&mut self) -> u64 {
        self.inflight.clear();
        self.wanted.lock().expect("wanted").clear();
        self.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Change render options (Slicer…): cached tiles are dropped, the
    /// source is kept (no rescan).
    /// Change the style post-process (instant: loaded tiles are re-lit).
    pub fn set_post(&mut self, p: Option<StyleParams>) {
        self.post = p;
        if let Some(map) = &mut self.map {
            map.set_post(p);
        }
    }

    pub fn options(&self) -> RenderOptions {
        self.opts
    }

    pub fn set_options(&mut self, opts: RenderOptions, store: Option<&Store>) -> Task<MapMsg> {
        if opts == self.opts {
            return Task::none();
        }
        self.opts = opts;
        self.bump();
        if let Some(map) = &mut self.map {
            map.tiles.clear();
            map.bytes = 0;
        }
        self.refill(store)
    }

    /// (Re)load the dimension. `rescan` rebuilds the Bedrock column index
    /// (after the world changed); otherwise a cached index is reused, so
    /// dimension switches are instant.
    pub fn load(&mut self, store: Option<&Store>, rescan: bool) -> Task<MapMsg> {
        let Some(store) = store.cloned() else { return Task::none() };
        let g = self.bump();
        self.loading = true;
        self.map = None;
        if rescan {
            self.index = None;
        }
        let dim = self.dim;
        let index = self.index.clone();
        self.status = if index.is_some() { "Opening dimension…".into() } else { "Indexing chunks…".into() };
        Task::perform(
            blocking(move || {
                let started = std::time::Instant::now();
                let (source, index) = match &store {
                    Store::Bedrock(w) => {
                        let index = match index {
                            Some(i) => i,
                            None => Arc::new(w.column_index().map_err(|e| e.to_string())?),
                        };
                        (MapSource::bedrock(w, &index, dim), Some(index))
                    }
                    Store::Java(w) => (MapSource::java(w, dim).map_err(|e| e.to_string())?, None),
                };
                Ok(Loaded { source, index, millis: started.elapsed().as_millis() })
            }),
            move |r| MapMsg::Loaded(g, dim, r),
        )
    }

    pub fn set_dim(&mut self, dim: u8, opts: RenderOptions, store: Option<&Store>) -> Task<MapMsg> {
        if dim == self.dim && self.map.is_some() {
            return Task::none();
        }
        self.dim = dim;
        self.opts = opts;
        self.fitted = false;
        self.load(store, false)
    }

    pub fn fit(&mut self) {
        if let Some(b) = self.map.as_ref().and_then(MapData::chunk_bounds) {
            self.camera = Camera::fit(b, self.view);
        }
    }

    pub fn on_viewport(&mut self, size: Size, store: Option<&Store>) -> Task<MapMsg> {
        if size.width < 1.0 || size.height < 1.0 || size == self.view {
            return Task::none();
        }
        // The first real viewport size replaces the placeholder the map
        // may have been fitted to while loading.
        let first = !self.fitted || (!self.view_known && !self.scripted);
        self.view_known = true;
        self.view = size;
        if first && self.map.is_some() {
            self.fit();
            self.fitted = true;
        }
        self.refill(store)
    }

    pub fn on_msg(&mut self, msg: MapMsg, store: Option<&Store>) -> Task<MapMsg> {
        match msg {
            MapMsg::Loaded(g, dim, res) => {
                if g != self.generation() || dim != self.dim {
                    return Task::none();
                }
                self.loading = false;
                match res {
                    Ok(l) => {
                        if l.index.is_some() {
                            self.index = l.index;
                        }
                        let total = l.source.chunks().len();
                        let mut map = MapData::new(l.source);
                        map.set_post(self.post);
                        self.map = Some(map);
                        self.status = format!("{total} chunks indexed in {:.1}s — rendering visible tiles…", l.millis as f32 / 1000.0);
                        if !self.fitted {
                            self.fit();
                            self.fitted = true;
                        }
                        return self.refill(store);
                    }
                    Err(e) => self.status = format!("map failed: {e}"),
                }
            }
            MapMsg::Tile(g, key, res) => {
                self.inflight.remove(&key);
                if g != self.generation() {
                    return Task::none();
                }
                match res {
                    Ok(Some(t)) => {
                        if let Some(map) = &mut self.map {
                            let t = Arc::try_unwrap(t).unwrap_or_else(|a| (*a).clone());
                            map.insert(key, t);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => self.status = format!("tile failed: {e}"),
                }
                return self.refill(store);
            }
        }
        Task::none()
    }

    /// Point the source at the session's current store handle (it is
    /// reopened after Save).
    fn rebind(&mut self, store: Option<&Store>) {
        let (Some(map), Some(store)) = (&mut self.map, store) else { return };
        match store {
            Store::Bedrock(w) => {
                if !map.source.bound_to_bedrock(w) {
                    map.source.rebind_bedrock(w);
                }
            }
            Store::Java(w) => {
                if !map.source.bound_to_java(w) {
                    map.source.rebind_java(w);
                }
            }
        }
    }

    /// Plan the visible tiles, offload what is not needed, and launch
    /// background builds (nearest first, bounded concurrency).
    pub fn refill(&mut self, store: Option<&Store>) -> Task<MapMsg> {
        self.rebind(store);
        let cfg = config();
        // Quality changed (toolbar or Settings page): tiles were sampled with
        // the old read budget — rebuild them.
        if self.quality_seen.is_some_and(|q| q != cfg.quality)
            && let Some(map) = &mut self.map
        {
            self.inflight.clear();
            self.wanted.lock().expect("wanted").clear();
            self.epoch.fetch_add(1, Ordering::SeqCst);
            map.tiles.clear();
            map.bytes = 0;
        }
        self.quality_seen = Some(cfg.quality);
        let (press, mem) = pressure();
        let budget_bytes = memory_budget(&cfg, press, mem);
        let reads = cfg.quality.budget(press);
        let level = level_for(self.camera.zoom, cfg.detail_bias);
        let view = self.view;
        let cam = self.camera;
        let g = self.generation();
        let Some(map) = &mut self.map else { return Task::none() };
        map.level = level;
        let (bx0, bz0, bx1, bz1) = cam.visible_blocks(view);
        let (tx0, tz0, tx1, tz1) = tile_range(level, bx0, bz0, bx1, bz1);
        let center = ((bx0 + bx1) / 2.0, (bz0 + bz1) / 2.0);
        // On-screen tiles + a one-tile margin, minus tiles with no chunks.
        let mut want: Vec<TileKey> = Vec::new();
        for tz in tz0 - 1..=tz1 + 1 {
            for tx in tx0 - 1..=tx1 + 1 {
                let k = TileKey::new(level, tx, tz);
                let (x0, z0, x1, z1) = k.chunk_rect();
                if map.source.chunks().any_in_rect(x0, z0, x1, z1) {
                    want.push(k);
                }
            }
        }
        let on_screen: HashSet<TileKey> = want
            .iter()
            .copied()
            .filter(|k| k.tx >= tx0 && k.tx <= tx1 && k.tz >= tz0 && k.tz <= tz1)
            .collect();
        let dropped = map.evict(budget_bytes, level, &on_screen, center);
        map.update_hidden(level, (tx0, tz0, tx1, tz1));
        *self.wanted.lock().expect("wanted") = want.iter().copied().collect();

        let max_inflight = match cfg.parallel {
            0 => std::thread::available_parallelism().map_or(2, |n| (n.get() / 4).clamp(1, 3)),
            n => n as usize,
        };
        let max_inflight = if press == Pressure::Normal { max_inflight } else { 1 };
        self.paused = press == Pressure::Critical && map.bytes() >= budget_bytes / 2;
        let mut todo: Vec<TileKey> = want.into_iter().filter(|k| !map.tiles.contains_key(k) && !self.inflight.contains(k)).collect();
        todo.sort_unstable_by_key(|k| {
            let half = k.span_blocks() as f64 / 2.0;
            let (x0, z0, _, _) = k.chunk_rect();
            let (cx, cz) = (x0 as f64 * 16.0 + half, z0 as f64 * 16.0 + half);
            (((cx - center.0).powi(2) + (cz - center.1).powi(2)).sqrt()) as u64
        });
        let slots = if self.paused { 0 } else { max_inflight.saturating_sub(self.inflight.len()) };
        let mut tasks = Vec::new();
        for key in todo.iter().copied().take(slots) {
            self.inflight.insert(key);
            let src = map.source.clone();
            let opts = self.opts;
            let wanted = Arc::clone(&self.wanted);
            let epoch = Arc::clone(&self.epoch);
            let cache = disk_cache(&cfg);
            let dim = self.dim;
            let threads = cfg.threads as usize;
            tasks.push(Task::perform(
                blocking(move || omt_render::workers::install(threads, || build(src, dim, key, reads, opts, wanted, epoch, g, cache))),
                move |r| MapMsg::Tile(g, key, r),
            ));
        }

        if todo.is_empty() && self.inflight.is_empty() && tasks.is_empty() && !self.loading
            && let Some(cam) = self.script.pop_front()
        {
            self.camera = cam;
            self.scripted = true;
            return self.refill(store);
        }
        let mb = map.bytes() as f64 / (1024.0 * 1024.0);
        // Coarsest sampling among the visible tiles of this level.
        let stride = on_screen.iter().filter_map(|k| map.tiles.get(k)).map(|t| t.stride).max().unwrap_or(1);
        let detail = if stride > 1 { format!(" · reduced detail (1 chunk per {stride}×{stride})") } else { String::new() };
        let mem_note = match (press, mem) {
            (Pressure::Critical, Some(m)) if self.paused => format!(" · PAUSED: system memory low ({} MB free)", m.available / (1024 * 1024)),
            (Pressure::Critical, Some(m)) => format!(" · memory critical ({} MB free) — cache shrunk", m.available / (1024 * 1024)),
            (Pressure::Low, _) => " · memory low — cache halved".to_string(),
            _ => String::new(),
        };
        let pending = todo.len().saturating_sub(tasks.len()) + self.inflight.len();
        let errors = match map.errors() {
            0 => String::new(),
            n => format!(" · {n} unreadable chunks (holes)"),
        };
        self.status = format!(
            "{} chunks · zoom level {level}{detail} · {} tiles ({mb:.0}/{:.0} MB){} · quality {}{errors}{mem_note}{}",
            map.chunk_total(),
            map.tiles.len(),
            budget_bytes as f64 / (1024.0 * 1024.0),
            if pending > 0 { format!(" · {pending} rendering…") } else { String::new() },
            cfg.quality,
            if dropped > 0 { format!(" · offloaded {dropped}") } else { String::new() },
        );
        Task::batch(tasks)
    }
}

/// Build (or load from the disk cache) one tile. Cancels itself when the
/// tile stops being wanted or the generation changes.
#[allow(clippy::too_many_arguments)]
fn build(
    src: MapSource,
    dim: u8,
    key: TileKey,
    reads: Option<u64>,
    opts: RenderOptions,
    wanted: Arc<Mutex<HashSet<TileKey>>>,
    epoch: Arc<AtomicU64>,
    g: u64,
    cache: Option<TileCache>,
) -> Result<Option<Arc<TileImage>>, String> {
    let cancelled = || epoch.load(Ordering::SeqCst) != g || !wanted.lock().map(|w| w.contains(&key)).unwrap_or(false);
    if cancelled() {
        return Ok(None);
    }
    let stride = src.level_stride(key.level, reads);
    let (x0, z0, x1, z1) = key.chunk_rect();
    let hash = cache
        .as_ref()
        .filter(|_| src.cacheable())
        .map(|_| omt_render::tilecache::tile_key_hash(src.fingerprint(x0, z0 - 1, x1, z1), dim, &opts, key, stride));
    if let (Some(c), Some(h)) = (&cache, hash)
        && let Some(t) = c.load(h, key)
    {
        return Ok(Some(Arc::new(t)));
    }
    if !src.is_bound() {
        return Ok(None);
    }
    let Some(t) = omt_render::build_tile(&src, key, reads, Palette::embedded(), &opts, &cancelled) else {
        return Ok(None);
    };
    if let (Some(c), Some(h)) = (&cache, hash) {
        c.store(h, &t);
        // Occasional size-cap enforcement (cheap directory walk).
        if (h & 0x3f) == 0 {
            c.prune();
        }
    }
    Ok(Some(Arc::new(t)))
}

#[cfg(test)]
#[path = "mapdata_tests.rs"]
mod tests;
